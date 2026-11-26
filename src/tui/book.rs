//! A book's pages, as a gallery or a list, which can be put in another order.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::add::Add;
use super::books::Books;
use super::confirm::{Confirm, Question};
use super::data::{self, Book, Leaf, NewLeaf, Publication};
use super::help::Binding;
use super::images::{Images, Source};
use super::leaf::LeafPage;
use super::publishing::Publishing;
use super::search::Search;
use super::views::{self, Row, View};
use super::{Id, Loading, Message, Overlay, Reply, Request, next_id, theme};

const TILE_WIDTH: u16 = 24;
const TILE_HEIGHT: u16 = 14;

pub struct BookPage {
    pub id: Id,
    pub host: String,
    pub book_id: u64,
    pub book: Loading<Book>,
    pub selected: usize,
    columns: usize,
    offset: usize,
    new_order: Option<Vec<Leaf>>,
    /// The page to select once the book arrives, like one a search found.
    pub leaf_to_select: Option<u64>,
    /// The books to go back to.
    pub books: Books,
}

pub enum Action {
    Nothing,
    Back,
    OpenLeaf,
    ToggleView,
    EditBook(u64),
    Overlay(Overlay),
    Request(Request),
}

impl BookPage {
    pub fn new(books: Books, book_id: u64) -> BookPage {
        BookPage {
            id: next_id(),
            host: books.host.clone(),
            book_id,
            book: Loading::Pending,
            selected: 0,
            columns: 1,
            offset: 0,
            new_order: None,
            leaf_to_select: None,
            books,
        }
    }

    pub fn load(&self) -> Request {
        let id = self.id;
        let host = self.host.clone();
        let book_id = self.book_id;
        Box::new(move || Message::To(id, Reply::BookLoaded(data::book(&host, book_id))))
    }

    pub fn loaded(&mut self, book: Result<Book>) {
        self.book = Loading::from(book);

        if let Some(leaf_id) = self.leaf_to_select.take() {
            self.select_leaf(leaf_id);
        }

        self.selected = self.selected.min(self.leaf_count().saturating_sub(1));
    }

    pub fn select_leaf(&mut self, leaf_id: u64) {
        if let Some(index) = self
            .book
            .ready()
            .and_then(|it| it.leaves.iter().position(|leaf| leaf.id == leaf_id))
        {
            self.selected = index;
        }
    }

    /// The cover and the pictures, to fetch.
    pub fn pictures(&self) -> Vec<String> {
        match self.book.ready() {
            Some(book) => book
                .cover
                .iter()
                .cloned()
                .chain(
                    book.leaves
                        .iter()
                        .filter_map(|it| it.image.as_ref().map(|image| image.url.clone())),
                )
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn selected_leaf(&self) -> Option<&Leaf> {
        self.book.ready().and_then(|it| it.leaves.get(self.selected))
    }

    /// The selected page, opened to read.
    pub fn leaf_page(&self) -> Option<LeafPage> {
        let book = self.book.ready()?;
        Some(LeafPage::new(self.host.clone(), self.book_id, book, self.selected))
    }

    pub fn moving(&self) -> bool {
        self.new_order.is_some()
    }

    pub fn published(&mut self, publication: Publication) {
        if let Loading::Ready(book) = &mut self.book {
            book.published = publication.published;
            book.slug = publication.slug;
        }
    }

    pub fn add(&self, leaf: NewLeaf) -> Request {
        let id = self.id;
        let host = self.host.clone();
        let book_id = self.book_id;

        Box::new(move || Message::To(id, Reply::LeavesChanged(data::add(&host, book_id, &leaf))))
    }

    pub fn delete(&self, leaf: Leaf) -> Request {
        let id = self.id;
        let host = self.host.clone();
        let book_id = self.book_id;

        Box::new(move || Message::To(id, Reply::LeavesChanged(data::delete(&host, book_id, &leaf))))
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match self.new_order.take() {
            Some(order) => self.move_key(order, key),
            None => self.browse_key(key),
        }
    }

    pub fn bindings(&self, view: View) -> Vec<Binding> {
        if self.moving() {
            return vec![(view.arrows(), "move"), ("enter", "save"), ("esc", "cancel")];
        }

        let mut bindings = vec![
            (view.arrows(), "select"),
            ("enter", "open"),
            ("v", view.other_name()),
            ("/", "search"),
        ];

        if let Some(book) = self.book.ready().filter(|it| it.editable) {
            bindings.push(("a", "add"));

            if !book.leaves.is_empty() {
                bindings.extend([("d", "delete"), ("m", "move")]);
            }

            bindings.extend([("e", "edit book"), ("p", "publishing")]);
        }

        bindings.extend([("esc", "books"), ("ctrl+i", "instances"), ("ctrl+c", "quit")]);
        bindings
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: View, images: &mut Images) {
        let title = match self.book.ready() {
            Some(book) => book.title.as_str(),
            None => "…",
        };
        let area = views::breadcrumbs(frame, area, &[&self.books.instance_name, "Books", title]);

        let book = match &self.book {
            Loading::Ready(book) => book,
            Loading::Pending => return views::placeholder(frame, area, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, area, error),
        };

        let [header, body] = Layout::vertical([Constraint::Length(12), Constraint::Fill(1)]).areas(area);

        book_header(frame, header, &self.host, book, images);

        let leaves = self.new_order.as_ref().unwrap_or(&book.leaves);
        let color = theme::color(book.theme.as_deref());

        if leaves.is_empty() {
            let lines: &[&str] = if book.editable {
                &["This book has no pages yet.", "Press \"a\" to add some."]
            } else {
                &["This book has no pages yet."]
            };
            views::placeholder_lines(frame, body, lines);
        } else {
            match view {
                View::Gallery => {
                    let tiles = views::grid(
                        body,
                        TILE_WIDTH,
                        TILE_HEIGHT,
                        leaves.len(),
                        self.selected,
                        &mut self.columns,
                        &mut self.offset,
                    );

                    for (index, tile) in tiles {
                        let moving = self.new_order.is_some() && index == self.selected;
                        leaf_tile(
                            frame,
                            tile,
                            &self.host,
                            &leaves[index],
                            index == self.selected,
                            moving,
                            color,
                            images,
                        );
                    }
                }
                View::List => {
                    let rows: Vec<Row> = leaves
                        .iter()
                        .map(|leaf| Row {
                            left: vec![Span::from(views::leaf_title(leaf))],
                            right: leaf.word_count.map(words).unwrap_or_default(),
                            color: if self.new_order.is_some() { Color::Yellow } else { color },
                            strong: leaf.is_section(),
                        })
                        .collect();
                    views::list(frame, body, &rows, self.selected, &mut self.offset);
                    self.columns = 1;
                }
            }
        }
    }

    fn leaf_count(&self) -> usize {
        match self.book.ready() {
            Some(book) => book.leaves.len(),
            None => 0,
        }
    }

    fn move_key(&mut self, mut order: Vec<Leaf>, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Nothing,
            KeyCode::Enter => self.arrange(order),
            _ => {
                let from = self.selected;
                views::navigate(key, &mut self.selected, self.columns, order.len());
                let leaf = order.remove(from);
                order.insert(self.selected, leaf);
                self.new_order = Some(order);
                Action::Nothing
            }
        }
    }

    fn arrange(&mut self, order: Vec<Leaf>) -> Action {
        let ids: Vec<u64> = order.iter().map(|it| it.id).collect();
        let id = self.id;
        let host = self.host.clone();
        let book_id = self.book_id;

        if let Loading::Ready(book) = &mut self.book {
            book.leaves = order;
        }

        Action::Request(Box::new(move || {
            Message::To(id, Reply::LeavesChanged(data::arrange(&host, book_id, &ids)))
        }))
    }

    fn browse_key(&mut self, key: KeyEvent) -> Action {
        let editable = self.book.ready().is_some_and(|it| it.editable);
        let has_leaf = self.book.ready().is_some_and(|it| !it.leaves.is_empty());

        match key.code {
            KeyCode::Esc => Action::Back,
            KeyCode::Enter if has_leaf => Action::OpenLeaf,
            KeyCode::Char('v') => Action::ToggleView,
            KeyCode::Char('/') => {
                let title = match self.book.ready() {
                    Some(book) => format!("“{}”", book.title),
                    None => "this book".to_string(),
                };
                Action::Overlay(Overlay::Search(Box::new(Search::new(Some(self.book_id), title))))
            }
            KeyCode::Char('a') if editable => Action::Overlay(Overlay::Add(Add::default())),
            KeyCode::Char('d') if editable && has_leaf => match self.selected_leaf() {
                Some(leaf) => Action::Overlay(Overlay::Confirm(Confirm::new(Question::DeleteLeaf {
                    book_id: self.book_id,
                    leaf: leaf.clone(),
                }))),
                None => Action::Nothing,
            },
            KeyCode::Char('m') if editable && has_leaf => {
                self.new_order = self.book.ready().map(|it| it.leaves.clone());
                Action::Nothing
            }
            KeyCode::Char('p') if editable => match self.book.ready() {
                Some(book) => Action::Overlay(Overlay::Publishing(Publishing::new(&self.host, self.id, book))),
                None => Action::Nothing,
            },
            KeyCode::Char('e') if editable => Action::EditBook(self.book_id),
            _ => {
                let count = self.leaf_count();
                views::navigate(key, &mut self.selected, self.columns, count);
                Action::Nothing
            }
        }
    }
}

fn book_header(frame: &mut Frame, area: Rect, host: &str, book: &Book, images: &mut Images) {
    let [cover, _, text] =
        Layout::horizontal([Constraint::Length(18), Constraint::Length(2), Constraint::Fill(1)]).areas(area);
    let cover = Rect {
        height: cover.height.saturating_sub(1),
        ..cover
    };

    let drawn = match &book.cover {
        Some(path) => images.draw(frame, cover, &Source::remote(host, path)),
        None => false,
    };

    if !drawn {
        views::themed_cover(frame, cover, &book.title, book.theme.as_deref());
    }

    let mut lines = vec![
        Line::from(book.title.clone())
            .bold()
            .fg(theme::color(book.theme.as_deref())),
    ];
    lines.extend(book.subtitle.clone().map(Line::from));
    lines.extend(book.author.clone().map(|it| Line::from(it).italic().dim()));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text);
}

fn leaf_tile(
    frame: &mut Frame,
    area: Rect,
    host: &str,
    leaf: &Leaf,
    selected: bool,
    moving: bool,
    color: Color,
    images: &mut Images,
) {
    let border = if moving { Color::Yellow } else { color };
    let [preview, text] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);
    let inside = views::selection_box(frame, preview, Some(if selected { border } else { Color::DarkGray }));

    let drawn = match (&leaf.image, leaf.is_picture()) {
        (Some(image), true) => images.draw(frame, inside, &Source::remote(host, &image.url)),
        _ => false,
    };

    if !drawn {
        let paragraph = Paragraph::new(leaf.preview.clone().unwrap_or_default())
            .wrap(Wrap { trim: true })
            .dim();
        let paragraph = if leaf.is_section() {
            paragraph.alignment(Alignment::Center)
        } else {
            paragraph
        };
        frame.render_widget(
            paragraph,
            inside.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 0,
            }),
        );
    }

    let lines = vec![
        Line::from(views::leaf_title(leaf)).bold(),
        Line::from(leaf.word_count.map(words).unwrap_or_default()).dim(),
    ];
    frame.render_widget(
        Paragraph::new(views::in_selection_color(lines, selected, border)),
        views::beside_gap(text),
    );
}

fn words(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }

    if count == 1 {
        "1 word".to_string()
    } else {
        format!("{grouped} words")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn handbook() -> BookPage {
        let leaf = |id, kind: &str, title: &str, word_count| Leaf {
            id,
            kind: kind.to_string(),
            title: title.to_string(),
            word_count,
            preview: None,
            image: None,
        };
        let books = Books::new("https://books.example.com".to_string(), "books.example.com".to_string());
        let mut screen = BookPage::new(books, 1);
        screen.book = Loading::Ready(Book {
            id: 1,
            title: "Handbook".to_string(),
            subtitle: None,
            author: Some("Ops".to_string()),
            theme: None,
            cover: None,
            slug: "handbook".to_string(),
            published: false,
            editable: true,
            leaves: vec![
                leaf(1, "Section", "Part one", None),
                leaf(2, "Page", "On call", Some(1234)),
                leaf(3, "Page", "", Some(1)),
            ],
        });
        screen
    }

    #[test]
    fn asks_to_delete_the_page_selected_when_d_was_pressed() {
        let mut screen = handbook();
        screen.selected = 1;

        let confirm = match screen.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)) {
            Action::Overlay(Overlay::Confirm(confirm)) => confirm,
            _ => panic!("no question"),
        };
        screen.selected = 2;

        assert!(matches!(
            confirm.question,
            Question::DeleteLeaf {
                book_id: 1,
                leaf: Leaf { id: 2, .. }
            }
        ));
    }

    #[test]
    fn lists_pages_with_their_word_counts_on_the_right() {
        let mut terminal = Terminal::new(TestBackend::new(40, 19)).unwrap();
        let mut screen = handbook();
        screen.selected = 1;
        let mut images = Images::none();

        terminal
            .draw(|frame| screen.draw(frame, frame.area(), View::List, &mut images))
            .unwrap();

        let text = views::screen_text(&terminal);
        assert_eq!(text[1], "books.example.com > Books > Handbook");
        assert_eq!(
            &text[4][20..],
            "Handbook",
            "the cover stays to the left of the title in a list too"
        );
        assert_eq!(&text[5][20..], "Ops");
        assert_eq!(
            text[16..19],
            [
                "  Part one ............................",
                "› On call .................. 1,234 words",
                "  Untitled page ................. 1 word"
            ]
        );
    }
}
