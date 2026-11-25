//! One page of a book, read in the terminal, or edited: pages as rendered
//! Markdown, pictures and sections centered, ← and → turning to the pages
//! beside it.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::Loading;
use super::data::{Leaf, LeafContent};
use super::editor::Editor;
use super::images::Images;
use super::markdown::{self, Row};
use super::views;

pub struct LeafPage {
    pub host: String,
    pub book_id: u64,
    pub book_title: String,
    pub editable: bool,
    pub leaves: Vec<Leaf>,
    pub index: usize,
    pub content: Loading<LeafContent>,
    pub scroll: usize,
    pub editor: Option<Editor>,
    pub saving: bool,
}

/// What a key on the page asks the app to do.
pub enum Action {
    Nothing,
    /// Another page was turned to, so its content needs loading.
    Turned,
    /// Back to the book, past the first or last page or with esc.
    Back,
    Edit,
    Delete,
}

impl LeafPage {
    pub fn leaf(&self) -> &Leaf {
        &self.leaves[self.index]
    }

    /// Pages and sections have text to edit; pictures have only a caption.
    pub fn can_edit_text(&self) -> bool {
        self.editable && !self.leaf().is_picture()
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Back,
            KeyCode::Left if self.index == 0 => Action::Back,
            KeyCode::Right if self.index + 1 == self.leaves.len() => Action::Back,
            KeyCode::Left => self.turn_to(self.index - 1),
            KeyCode::Right => self.turn_to(self.index + 1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-20),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(20),
            KeyCode::Home => self.scroll_by(-100_000),
            KeyCode::Char('e') if self.can_edit_text() && matches!(self.content, Loading::Ready(_)) => Action::Edit,
            KeyCode::Char('d') if self.editable => Action::Delete,
            _ => Action::Nothing,
        }
    }

    pub fn turn_to(&mut self, index: usize) -> Action {
        self.index = index;
        self.content = Loading::Pending;
        self.scroll = 0;
        Action::Turned
    }

    fn scroll_by(&mut self, lines: i64) -> Action {
        self.scroll = (self.scroll as i64 + lines).max(0) as usize;
        Action::Nothing
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

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, images: &mut Images) {
        if let Some(editor) = &mut self.editor {
            return editor.draw(frame, area);
        }

        let content = match &self.content {
            Loading::Ready(content) => content.clone(),
            Loading::Pending => return views::placeholder(frame, area, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, area, error),
        };

        let leaf = self.leaf().clone();
        let column = reading_column(area);

        if leaf.is_picture() {
            self.draw_picture(frame, area, &content, images);
        } else if leaf.is_section() {
            self.draw_section(frame, column, &content);
        } else {
            self.draw_page(frame, column, &content, images);
        }

        let position = format!("{} of {}", self.index + 1, self.leaves.len());
        let corner = Rect { y: area.bottom().saturating_sub(1), height: 1, ..area };
        frame.render_widget(Paragraph::new(Line::from(position).dim()).alignment(Alignment::Right), corner);
    }

    fn draw_page(&mut self, frame: &mut Frame, area: Rect, content: &LeafContent, images: &mut Images) {
        let rows = markdown::render(&content.body, area.width);

        let heights: Vec<u16> = rows.iter().map(row_height).collect();
        let total: usize = heights.iter().map(|it| usize::from(*it)).sum();
        self.scroll = self.scroll.min(total.saturating_sub(usize::from(area.height)));

        let mut skipped = 0;
        let mut y = area.y;

        for (row, height) in rows.iter().zip(heights) {
            if skipped + usize::from(height) <= self.scroll {
                skipped += usize::from(height);
                continue;
            }

            if y >= area.bottom() {
                break;
            }

            let room = Rect { y, height: height.min(area.bottom() - y), ..area };

            match row {
                Row::Text(line) => frame.render_widget(Paragraph::new(line.clone()), room),
                Row::Image { url } if room.height == height => {
                    if !images.draw_left(frame, room, url) {
                        frame.render_widget(Paragraph::new(Line::from("Loading the picture…").dim()), room);
                    }
                }
                Row::Image { .. } => {}
            }

            y += height;
        }
    }

    fn draw_picture(&self, frame: &mut Frame, area: Rect, content: &LeafContent, images: &mut Images) {
        let picture = Rect { height: area.height.saturating_sub(3), ..area };
        let shown = match &content.image {
            Some(url) => images.draw(frame, centered_image(picture), url),
            None => false,
        };

        if !shown {
            frame.render_widget(Paragraph::new(Line::from("Loading the picture…").dim()).alignment(Alignment::Center), Rect { y: picture.y + picture.height / 2, height: 1, ..picture });
        }

        let caption = Rect { y: area.bottom().saturating_sub(2), height: 1, ..area };
        frame.render_widget(Paragraph::new(Line::from(content.body.clone()).italic()).alignment(Alignment::Center), caption);
    }

    fn draw_section(&self, frame: &mut Frame, area: Rect, content: &LeafContent) {
        let lines: Vec<Line> = content.body.lines().map(|line| Line::from(line.to_string())).collect();

        let height = (lines.len() as u16 + 2).min(area.height);
        let middle = Rect { y: area.y + (area.height - height) / 2, height, ..area };
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true }), middle);
    }

    /// The book's index: its pages, sections in bold, the current one marked.
    pub fn draw_index(&self, frame: &mut Frame, area: Rect, selected: usize) {
        let width = 50.min(area.width);
        let mut lines = vec![Line::from(self.book_title.clone()).bold(), Line::default()];

        for (index, leaf) in self.leaves.iter().enumerate() {
            let title = if leaf.title.is_empty() { "Untitled".to_string() } else { leaf.title.clone() };
            let marker = if index == selected { "› " } else { "  " };
            let dots = ".".repeat(usize::from(width).saturating_sub(title.chars().count() + 5));
            let style = if leaf.is_section() { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
            let style = if index == selected { style.fg(Color::Blue).add_modifier(Modifier::BOLD) } else { style };
            let here = if index == self.index { " ◂" } else { "" };

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
