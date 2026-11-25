//! One page of a book, read in the terminal, or edited: pages as rendered
//! Markdown, pictures and sections centered, ← and → turning to the pages
//! beside it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::confirm::{Confirm, Question};
use super::data::{self, Book, Leaf, LeafContent};
use super::editor::{self, Editor};
use super::help::Binding;
use super::images::{Images, Source};
use super::markdown::{self, Row};
use super::{Id, Loading, Message, Overlay, Reply, Request, next_id, views};

pub struct LeafPage {
    /// A new one for each page turned to, so a reply for the page before
    /// isn't taken for this one.
    pub id: Id,
    pub host: String,
    pub book_id: u64,
    pub book_title: String,
    pub editable: bool,
    pub leaves: Vec<Leaf>,
    pub index: usize,
    pub content: Loading<LeafContent>,
    /// The page's Markdown as rows, for the width they were rendered at.
    rendered: Option<Rendered>,
    pub scroll: usize,
    pub editor: Option<Editor>,
    pub saving: bool,
    /// How many files are still uploading into the page.
    uploads: usize,
}

struct Rendered {
    width: u16,
    rows: Vec<Row>,
}

/// What a key on the page asks the app to do.
pub enum Action {
    Nothing,
    /// Back to the book, past the first or last page or with esc.
    Back,
    /// Choosing a file to upload into the page being edited.
    Upload,
    Overlay(Overlay),
    Request(Request),
    Notice(&'static str),
}

/// The book's pages, to jump to one.
pub struct Index {
    selected: usize,
    offset: usize,
}

pub enum IndexAction {
    Nothing,
    Close,
    /// Another page was turned to; the request loads it.
    Turned(Request),
}

impl LeafPage {
    pub fn new(host: String, book_id: u64, book: &Book, index: usize) -> LeafPage {
        LeafPage {
            id: next_id(),
            host,
            book_id,
            book_title: book.title.clone(),
            editable: book.editable,
            leaves: book.leaves.clone(),
            index,
            content: Loading::Pending,
            rendered: None,
            scroll: 0,
            editor: None,
            saving: false,
            uploads: 0,
        }
    }

    pub fn load(&self) -> Request {
        let id = self.id;
        let host = self.host.clone();
        let book_id = self.book_id;
        let leaf = self.leaf().clone();
        Box::new(move || Message::To(id, Reply::LeafLoaded(data::leaf(&host, book_id, &leaf))))
    }

    pub fn loaded(&mut self, content: anyhow::Result<LeafContent>) {
        self.content = Loading::from(content);
        self.rendered = None;
    }

    pub fn leaf(&self) -> &Leaf {
        &self.leaves[self.index]
    }

    /// Pages and sections have text to edit; pictures have only a caption.
    pub fn can_edit_text(&self) -> bool {
        self.editable && !self.leaf().is_picture()
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.saving {
            return Action::Nothing;
        }

        match &mut self.editor {
            Some(editor) => match editor.key(key) {
                editor::Action::Nothing => Action::Nothing,
                editor::Action::Save => self.save(),
                editor::Action::Upload => Action::Upload,
                editor::Action::Leave if editor.changed() => Action::Overlay(Overlay::Confirm(Confirm::new(Question::Discard))),
                editor::Action::Leave => {
                    self.editor = None;
                    Action::Nothing
                }
            },
            None => self.read_key(key),
        }
    }

    /// Pasted text goes into the editor as it is, but not while the page is
    /// saving what was in it.
    pub fn paste(&mut self, text: &str) {
        if !self.saving
            && let Some(editor) = &mut self.editor
        {
            editor.paste(text);
        }
    }

    /// Save the page, once every file uploading into it is in.
    fn save(&mut self) -> Action {
        if self.uploads > 0 {
            return Action::Notice("Wait for the upload to finish, then save.");
        }

        let (Some(editor), Loading::Ready(content)) = (&self.editor, &self.content) else { return Action::Nothing };
        let title = editor.title.clone();
        let body = editor.body();
        let fingerprint = content.fingerprint.clone();
        let id = self.id;
        let host = self.host.clone();
        let book_id = self.book_id;
        let leaf = self.leaf().clone();
        self.saving = true;

        Action::Request(Box::new(move || Message::To(id, Reply::LeafSaved(data::save_leaf(&host, book_id, &leaf, &title, &body, &fingerprint)))))
    }

    fn read_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Back,
            KeyCode::Left if self.index == 0 => Action::Back,
            KeyCode::Right if self.index + 1 == self.leaves.len() => Action::Back,
            KeyCode::Left => Action::Request(self.turn_to(self.index - 1)),
            KeyCode::Right => Action::Request(self.turn_to(self.index + 1)),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-20),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(20),
            KeyCode::Home => self.scroll_by(-100_000),
            KeyCode::Char('e') if self.can_edit_text() => {
                if let Loading::Ready(content) = &self.content {
                    self.editor = Some(Editor::new(&content.title, &content.body));
                }
                Action::Nothing
            }
            KeyCode::Char('d') if self.editable => Action::Overlay(Overlay::Confirm(Confirm::new(Question::DeleteLeaf { book_id: self.book_id, leaf: self.leaf().clone() }))),
            _ => Action::Nothing,
        }
    }

    /// Turn to another page. Returns the request that loads it.
    fn turn_to(&mut self, index: usize) -> Request {
        self.id = next_id();
        self.uploads = 0;
        self.index = index;
        self.content = Loading::Pending;
        self.rendered = None;
        self.scroll = 0;
        self.load()
    }

    fn scroll_by(&mut self, lines: i64) -> Action {
        self.scroll = (self.scroll as i64 + lines).max(0) as usize;
        Action::Nothing
    }

    /// After saving, show the page as saved.
    pub fn saved(&mut self, fingerprint: String) {
        if let (Some(editor), Loading::Ready(content)) = (self.editor.take(), &mut self.content) {
            content.title = editor.title.clone();
            content.body = editor.body();
            content.fingerprint = fingerprint;
            self.leaves[self.index].title = editor.title;
            self.rendered = None;
        }
    }

    /// Uploading a file into the page, which only a page, not a section or
    /// picture, can take.
    pub fn upload(&mut self, path: std::path::PathBuf) -> Option<Request> {
        let record_gid = self.content.ready().and_then(|it| it.record_gid.clone())?;
        let id = self.id;
        let host = self.host.clone();
        self.uploads += 1;

        Some(Box::new(move || Message::To(id, Reply::Uploaded(data::upload(&host, &record_gid, &path)))))
    }

    /// An upload is done. Returns the editor its Markdown goes into, if the
    /// page is still being edited.
    pub fn upload_finished(&mut self) -> Option<&mut Editor> {
        self.uploads -= 1;
        self.editor.as_mut()
    }

    /// The images a page shows, to fetch.
    pub fn images(&self) -> Vec<String> {
        let Loading::Ready(content) = &self.content else { return Vec::new() };

        if self.leaf().is_picture() {
            return content.image.iter().cloned().collect();
        }

        markdown::render(&content.body, 80)
            .into_iter()
            .filter_map(|row| match row {
                Row::Image { url } => Some(url),
                Row::Text(_) => None,
            })
            .collect()
    }

    pub fn bindings(&self) -> Vec<Binding> {
        if self.editor.is_some() {
            return vec![("tab", "title or text"), ("esc", "stop editing"), ("ctrl+u", "upload a file"), ("ctrl+s", "save")];
        }

        let mut bindings = vec![("←→", "turn pages"), ("↑↓", "scroll")];

        if self.can_edit_text() {
            bindings.push(("e", "edit"));
        }

        if self.editable {
            bindings.push(("d", "delete"));
        }

        bindings.extend([("esc", "book"), ("ctrl+i", "index"), ("ctrl+c", "quit")]);
        bindings
    }

    /// The page below breadcrumbs from the instance, named by `instance`, down.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, instance: &str, images: &mut Images) {
        let title = match &self.content {
            Loading::Ready(content) if !content.title.is_empty() => content.title.clone(),
            _ => self.leaf().title.clone(),
        };
        let mut steps = vec![instance, "Books", self.book_title.as_str(), title.as_str()];

        if self.editor.is_some() {
            steps.push("Edit");
        }

        let area = views::breadcrumbs(frame, area, &steps);
        self.draw_leaf(frame, area, images);
    }

    fn draw_leaf(&mut self, frame: &mut Frame, area: Rect, images: &mut Images) {
        if let Some(editor) = &mut self.editor {
            return editor.draw(frame, area);
        }

        let column = reading_column(area);

        match &self.content {
            Loading::Pending => views::placeholder(frame, area, "Loading…"),
            Loading::Failed(error) => views::failure(frame, area, error),
            Loading::Ready(content) => {
                if self.leaves[self.index].is_picture() {
                    draw_picture(frame, area, &self.host, content, images);
                } else if self.leaves[self.index].is_section() {
                    draw_section(frame, column, content);
                } else {
                    if self.rendered.as_ref().is_none_or(|it| it.width != column.width) {
                        self.rendered = Some(Rendered { width: column.width, rows: markdown::render(&content.body, column.width) });
                    }

                    if let Some(rendered) = &self.rendered {
                        self.scroll = draw_rows(frame, column, &self.host, &rendered.rows, self.scroll, images);
                    }
                }

                let position = format!("{} of {}", self.index + 1, self.leaves.len());
                let corner = Rect { y: area.bottom().saturating_sub(1), height: 1, ..area };
                frame.render_widget(Paragraph::new(Line::from(position).dim()).alignment(Alignment::Right), corner);
            }
        }
    }
}

/// Draw the page's rows scrolled down by `scroll` lines, as far as they go.
/// Returns the scroll, kept from going past the end.
fn draw_rows(frame: &mut Frame, area: Rect, host: &str, rows: &[Row], scroll: usize, images: &mut Images) -> usize {
    let heights: Vec<u16> = rows.iter().map(row_height).collect();
    let total: usize = heights.iter().map(|it| usize::from(*it)).sum();
    let scroll = scroll.min(total.saturating_sub(usize::from(area.height)));

    for shown in visible_rows(&heights, scroll, usize::from(area.height)) {
        let room = Rect { y: area.y + shown.y as u16, height: shown.height as u16, ..area };

        match &rows[shown.index] {
            Row::Text(line) => frame.render_widget(Paragraph::new(line.clone()), room),
            Row::Image { url } if shown.height == usize::from(markdown::IMAGE_HEIGHT) => {
                if !images.draw_left(frame, room, &Source::remote(host, url)) {
                    frame.render_widget(Paragraph::new(Line::from("Loading the picture…").dim()), room);
                }
            }
            Row::Image { .. } => {}
        }
    }

    scroll
}

/// A row with some of it on the screen: which row, the line it starts on,
/// and how many of its lines show.
#[derive(Debug, PartialEq)]
struct Shown {
    index: usize,
    y: usize,
    height: usize,
}

/// The rows of these heights that show in `height` lines scrolled down by
/// `scroll`, a row cut off at the top taking only the lines left of it.
fn visible_rows(heights: &[u16], scroll: usize, height: usize) -> Vec<Shown> {
    let mut shown = Vec::new();
    let mut top = 0;

    for (index, row_height) in heights.iter().map(|it| usize::from(*it)).enumerate() {
        let bottom = top + row_height;

        if bottom > scroll && top < scroll + height {
            let y = top.saturating_sub(scroll);
            let visible = bottom.min(scroll + height) - top.max(scroll);
            shown.push(Shown { index, y, height: visible });
        }

        top = bottom;
    }

    shown
}

fn draw_picture(frame: &mut Frame, area: Rect, host: &str, content: &LeafContent, images: &mut Images) {
    let picture = Rect { height: area.height.saturating_sub(3), ..area };
    let shown = match &content.image {
        Some(url) => images.draw(frame, centered_image(picture), &Source::remote(host, url)),
        None => false,
    };

    if !shown {
        frame.render_widget(Paragraph::new(Line::from("Loading the picture…").dim()).alignment(Alignment::Center), Rect { y: picture.y + picture.height / 2, height: 1, ..picture });
    }

    let caption = Rect { y: area.bottom().saturating_sub(2), height: 1, ..area };
    frame.render_widget(Paragraph::new(Line::from(content.body.clone()).italic()).alignment(Alignment::Center), caption);
}

fn draw_section(frame: &mut Frame, area: Rect, content: &LeafContent) {
    let lines: Vec<Line> = content.body.lines().map(|line| Line::from(line.to_string())).collect();

    let height = (lines.len() as u16 + 2).min(area.height);
    let middle = Rect { y: area.y + (area.height - height) / 2, height, ..area };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true }), middle);
}

impl Index {
    /// Opens on the page being read.
    pub fn new(page: &LeafPage) -> Index {
        Index { selected: page.index, offset: 0 }
    }

    pub fn key(&mut self, key: KeyEvent, page: &mut LeafPage) -> IndexAction {
        match key.code {
            KeyCode::Esc => return IndexAction::Close,
            KeyCode::Enter if self.selected == page.index => return IndexAction::Close,
            KeyCode::Enter => return IndexAction::Turned(page.turn_to(self.selected)),
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(page.leaves.len().saturating_sub(1)),
            _ => {}
        }

        IndexAction::Nothing
    }

    pub fn bindings(&self) -> Vec<Binding> {
        vec![("↑↓", "select"), ("enter", "go to page"), ("esc", "close")]
    }

    /// The book's index: its pages, sections in bold, the current one
    /// marked, as many as fit around the selected one.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, page: &LeafPage) {
        let selected = self.selected;
        let width = 50.min(area.width);
        let mut lines = vec![Line::from(page.book_title.clone()).bold(), Line::default()];
        let fit = usize::from(area.height.saturating_sub(6)).max(1);
        views::scroll_into_view(&mut self.offset, selected, fit);

        for (index, leaf) in page.leaves.iter().enumerate().skip(self.offset).take(fit) {
            let title = if leaf.title.is_empty() { "Untitled".to_string() } else { leaf.title.clone() };
            let marker = if index == selected { "› " } else { "  " };
            let dots = ".".repeat(usize::from(width).saturating_sub(title.chars().count() + 5));
            let style = if leaf.is_section() { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
            let style = if index == selected { style.fg(Color::Blue).add_modifier(Modifier::BOLD) } else { style };
            let here = if index == page.index { " ◂" } else { "" };

            lines.push(Line::from(vec![
                ratatui::text::Span::styled(format!("{marker}{title} "), style),
                ratatui::text::Span::from(dots).fg(Color::DarkGray),
                ratatui::text::Span::from(here).fg(Color::Blue),
            ]));
        }

        views::window(frame, area, " Index ", lines, width + 4);
    }
}

/// Text reads best in a column about as wide as a printed page.
fn reading_column(area: Rect) -> Rect {
    let width = area.width.min(84);
    Rect { x: area.x + (area.width - width) / 2, width, height: area.height.saturating_sub(1), ..area }
}

/// A picture gets the middle of the screen, as wide as it can be.
fn centered_image(area: Rect) -> Rect {
    let width = area.width.min(area.height.saturating_mul(3));
    Rect { x: area.x + (area.width - width) / 2, width, ..area }
}

fn row_height(row: &Row) -> u16 {
    match row {
        Row::Text(_) => 1,
        Row::Image { .. } => markdown::IMAGE_HEIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn page(body: &str) -> LeafPage {
        let leaf = Leaf { id: 1, kind: "Page".to_string(), title: "On call".to_string(), word_count: None, preview: None, image: None };
        let book = Book {
            id: 1,
            title: "Handbook".to_string(),
            subtitle: None,
            author: None,
            theme: None,
            cover: None,
            slug: "handbook".to_string(),
            published: false,
            editable: true,
            leaves: vec![leaf],
        };
        let mut page = LeafPage::new("https://books.example.com".to_string(), 1, &book, 0);
        page.loaded(Ok(LeafContent { title: "On call".to_string(), body: body.to_string(), image: None, fingerprint: "f1".to_string(), record_gid: None }));
        page
    }

    fn drawn(page: &mut LeafPage) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let mut images = Images::none();
        terminal.draw(|frame| page.draw(frame, frame.area(), "books.example.com", &mut images)).unwrap();
        views::screen_text(&terminal)
    }

    #[test]
    fn the_index_keeps_the_selected_page_in_view() {
        let mut page = page("");
        page.leaves = (1..=40).map(|id| Leaf { id, kind: "Page".to_string(), title: format!("Page {id}"), word_count: None, preview: None, image: None }).collect();
        page.index = 39;
        let mut index = Index::new(&page);

        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        terminal.draw(|frame| index.draw(frame, frame.area(), &page)).unwrap();
        assert!(views::screen_text(&terminal).iter().any(|it| it.contains("› Page 40")));
    }

    #[test]
    fn the_end_of_a_page_shows_when_scrolled_past_a_picture() {
        let mut page = page("First\n\n![Diagram](/u/diagram.png)\n\nLast line");
        page.scroll = usize::MAX;

        assert!(drawn(&mut page).iter().any(|it| it.trim() == "Last line"));
    }

    #[test]
    fn shows_the_text_as_saved() {
        let mut page = page("Before");
        assert!(drawn(&mut page).iter().any(|it| it.trim() == "Before"));

        page.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        page.editor.as_mut().unwrap().paste("After ");
        page.saved("f2".to_string());

        assert!(drawn(&mut page).iter().any(|it| it.trim() == "After Before"));
    }
}
