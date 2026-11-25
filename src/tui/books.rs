//! An instance's books, as a gallery of covers or a list.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::data::{self, BookCard};
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
    pub name: String,
    pub books: Loading<Vec<BookCard>>,
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
    pub fn new(host: String, name: String) -> Books {
        Books { id: next_id(), host, name, books: Loading::Pending, selected: 0, columns: 1, offset: 0 }
    }

    pub fn load(&self) -> Request {
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || Message::To(id, Reply::Books(data::books(&host))))
    }

    /// The books as loaded.
    pub fn loaded(&mut self, books: anyhow::Result<Vec<BookCard>>) {
        self.books = Loading::from(books);
    }

    pub fn covers(&self) -> Vec<String> {
        self.books.ready().into_iter().flatten().filter_map(|book| book.cover.as_ref().map(|cover| cover.url.clone())).collect()
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => match self.books.ready().and_then(|it| it.get(self.selected)) {
                Some(book) => Action::Open(book.id),
                None => Action::Nothing,
            },
            KeyCode::Esc => Action::Back,
            KeyCode::Char('v') => Action::ToggleView,
            KeyCode::Char('n') => Action::NewBook,
            KeyCode::Char('s') => Action::Settings,
            KeyCode::Char('/') => Action::Overlay(Overlay::Search(Box::new(Search::new(None, "all books".to_string())))),
            _ => {
                let count = self.books.ready().map(Vec::len).unwrap_or(0);
                views::navigate(key, &mut self.selected, self.columns, count);
                Action::Nothing
            }
        }
    }

    pub fn bindings(&self, view: View) -> Vec<Binding> {
        vec![
            (view.arrows(), "select"),
            ("enter", "open"),
            ("v", view.other_name()),
            ("/", "search"),
            ("n", "new book"),
            ("s", "settings"),
            ("esc", "instances"),
            ("ctrl+i", "instances"),
            ("ctrl+c", "quit"),
        ]
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: View, images: &mut Images) {
        let body = views::breadcrumbs(frame, area, &[&self.name, "Books"]);

        let books = match &self.books {
            Loading::Ready(books) if books.is_empty() => return views::placeholder(frame, body, "There are no books here yet."),
            Loading::Ready(books) => books,
            Loading::Pending => return views::placeholder(frame, body, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, body, error),
        };

        match view {
            View::Gallery => {
                let tiles = views::grid(body, TILE_WIDTH, TILE_HEIGHT, books.len(), self.selected, &mut self.columns, &mut self.offset);

                for (index, tile) in tiles {
                    book_tile(frame, tile, &self.host, &books[index], index == self.selected, images);
                }
            }
            View::List => {
                let rows: Vec<Row> = books
                    .iter()
                    .map(|book| Row {
                        left: vec![Span::from(book.title.clone()).bold(), Span::from(byline(book.subtitle.as_deref(), book.author.as_deref())).dim()],
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
    frame.render_widget(Paragraph::new(views::in_selection_color(lines, selected, color)), views::beside_gap(text));
}

fn byline(subtitle: Option<&str>, author: Option<&str>) -> String {
    let parts: Vec<&str> = [subtitle, author].into_iter().flatten().filter(|it| !it.is_empty()).collect();
    if parts.is_empty() { String::new() } else { format!("  {}", parts.join(" · ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::File;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    fn card(id: u64, title: &str, subtitle: Option<&str>) -> BookCard {
        BookCard {
            id,
            title: title.to_string(),
            subtitle: subtitle.map(str::to_string),
            author: Some("Ops".to_string()),
            theme: Some("green".to_string()),
            cover: Some(File { url: "/covers/1".to_string() }),
            editable: true,
        }
    }

    fn books_screen() -> Books {
        Books {
            id: 1,
            host: "https://books.example.com".to_string(),
            name: "books.example.com".to_string(),
            books: Loading::Ready(vec![card(1, "Handbook", Some("How we work")), card(2, "Guide", None)]),
            selected: 1,
            columns: 1,
            offset: 0,
        }
    }

    #[test]
    fn frames_only_the_selected_books_cover_and_puts_the_titles_below() {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut screen = books_screen();
        let mut images = Images::none();

        terminal.draw(|frame| screen.draw(frame, frame.area(), View::Gallery, &mut images)).unwrap();
        let text = views::screen_text(&terminal);
        let buffer = terminal.backend().buffer();

        assert_eq!(screen.columns, 2);
        assert_eq!(text[4], format!("{}╭{}╮", " ".repeat(24), "─".repeat(20)));
        assert_eq!(text[17], format!("{}╰{}╯", " ".repeat(24), "─".repeat(20)));
        assert_eq!(buffer[(24, 4)].fg, Color::Green);
        assert_eq!(text[18..21], ["Handbook                Guide", "How we work", "Ops                     Ops"]);
        assert_eq!(buffer[(24, 18)].fg, Color::Green, "the selected book's title is in its theme's color");
        assert_eq!(buffer[(24, 20)].fg, Color::Green, "and so is its author");
        assert_eq!(buffer[(0, 18)].fg, Color::Reset);

        // Covers that haven't arrived are drawn in the theme's color, with the title.
        assert_eq!(buffer[(1, 5)].bg, Color::Green);
        assert_eq!(&text[6][2..10], "Handbook");
    }

    #[test]
    fn lists_books_with_their_subtitle_and_author() {
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        let mut screen = books_screen();
        let mut images = Images::none();

        terminal.draw(|frame| screen.draw(frame, frame.area(), View::List, &mut images)).unwrap();

        assert_eq!(views::screen_text(&terminal)[..3], ["─".repeat(60), "books.example.com > Books".to_string(), "─".repeat(60)]);
        assert_eq!(
            views::screen_text(&terminal)[4..6],
            [
                format!("  Handbook  How we work · Ops {}", ".".repeat(29)),
                format!("› Guide  Ops {}", ".".repeat(46)),
            ]
        );
    }
}
