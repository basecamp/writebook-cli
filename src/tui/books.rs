//! An instance's books, as a gallery of covers or a list.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::data::{self, BookCard, Library};
use super::help::Binding;
use super::images::{Images, Source};
use super::search::Search;
use super::views::{self, Row, View};
use super::{Id, Loading, Message, Overlay, Reply, Request, next_id, theme};

const TILE_WIDTH: u16 = 24;
const TILE_HEIGHT: u16 = 18;

pub struct Books {
    pub id: Id,
    pub host: String,
    pub instance_name: String,
    pub books: Loading<Vec<BookCard>>,
    can_write: bool,
    selected: usize,
    columns: usize,
    offset: usize,
}

pub enum Action {
    Nothing,
    Back,
    Open(u64),
    ToggleView,
    NewBook,
    Settings,
    Overlay(Overlay),
}

impl Books {
    pub fn new(host: String, instance_name: String) -> Books {
        Books {
            id: next_id(),
            host,
            instance_name,
            books: Loading::Pending,
            can_write: false,
            selected: 0,
            columns: 1,
            offset: 0,
        }
    }

    pub fn load(&self) -> Request {
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || Message::To(id, Reply::BooksLoaded(data::books(&host))))
    }

    /// The books as loaded, the selection kept on one of them.
    pub fn loaded(&mut self, library: anyhow::Result<Library>) {
        match library {
            Ok(library) => {
                self.can_write = library.can_write;
                self.books = Loading::Ready(library.books);
            }
            Err(error) => self.books = Loading::from(Err(error)),
        }
        self.selected = self.selected.min(self.count().saturating_sub(1));
    }

    pub fn covers(&self) -> Vec<String> {
        self.books
            .ready()
            .into_iter()
            .flatten()
            .filter_map(|book| book.cover.as_ref().map(|cover| cover.url.clone()))
            .collect()
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => match self.books.ready().and_then(|it| it.get(self.selected)) {
                Some(book) => Action::Open(book.id),
                None => Action::Nothing,
            },
            KeyCode::Esc => Action::Back,
            KeyCode::Char('v') => Action::ToggleView,
            KeyCode::Char('n') if self.can_write => Action::NewBook,
            KeyCode::Char('s') => Action::Settings,
            KeyCode::Char('/') => {
                Action::Overlay(Overlay::Search(Box::new(Search::new(None, "all books".to_string()))))
            }
            _ => {
                let count = self.count();
                views::navigate(key, &mut self.selected, self.columns, count);
                Action::Nothing
            }
        }
    }

    pub fn bindings(&self, view: View) -> Vec<Binding> {
        let mut bindings = vec![
            (view.arrows(), "select"),
            ("enter", "open"),
            ("v", view.other_name()),
            ("/", "search"),
        ];

        if self.can_write {
            bindings.push(("n", "new book"));
        }

        bindings.extend([
            ("s", "settings"),
            ("esc", "instances"),
            ("ctrl+i", "instances"),
            ("ctrl+c", "quit"),
        ]);
        bindings
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: View, images: &mut Images) {
        let body = views::breadcrumbs(frame, area, &[&self.instance_name, "Books"]);

        let books = match &self.books {
            Loading::Ready(books) if books.is_empty() => {
                return views::placeholder(frame, body, "There are no books here yet.");
            }
            Loading::Ready(books) => books,
            Loading::Pending => return views::placeholder(frame, body, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, body, error),
        };

        match view {
            View::Gallery => {
                let tiles = views::grid(
                    body,
                    TILE_WIDTH,
                    TILE_HEIGHT,
                    books.len(),
                    self.selected,
                    &mut self.columns,
                    &mut self.offset,
                );

                for (index, tile) in tiles {
                    book_tile(frame, tile, &self.host, &books[index], index == self.selected, images);
                }
            }
            View::List => {
                let rows: Vec<Row> = books
                    .iter()
                    .map(|book| Row {
                        left: vec![
                            Span::from(book.title.clone()).bold(),
                            Span::from(byline(book.subtitle.as_deref(), book.author.as_deref())).dim(),
                        ],
                        right: String::new(),
                        color: theme::color(book.theme.as_deref()),
                        strong: false,
                    })
                    .collect();
                views::list(frame, body, &rows, self.selected, &mut self.offset);
                self.columns = 1;
            }
        }
    }

    fn count(&self) -> usize {
        match self.books.ready() {
            Some(books) => books.len(),
            None => 0,
        }
    }
}

fn book_tile(frame: &mut Frame, area: Rect, host: &str, book: &BookCard, selected: bool, images: &mut Images) {
    let color = theme::color(book.theme.as_deref());
    let [cover, text] = Layout::vertical([Constraint::Fill(1), Constraint::Length(4)]).areas(area);
    let inside = views::selection_box(frame, cover, selected.then_some(color));

    let drawn = match &book.cover {
        Some(cover) => images.draw(frame, inside, &Source::remote(host, &cover.url)),
        None => false,
    };

    if !drawn {
        views::themed_cover(frame, inside, &book.title, book.theme.as_deref());
    }

    let lines = vec![
        Line::from(book.title.clone()).bold(),
        Line::from(book.subtitle.clone().unwrap_or_default()).dim(),
        Line::from(book.author.clone().unwrap_or_default()).italic().dim(),
    ];
    frame.render_widget(
        Paragraph::new(views::in_selection_color(lines, selected, color)),
        views::beside_gap(text),
    );
}

fn byline(subtitle: Option<&str>, author: Option<&str>) -> String {
    let parts: Vec<&str> = [subtitle, author]
        .into_iter()
        .flatten()
        .filter(|it| !it.is_empty())
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("  {}", parts.join(" · "))
    }
}
