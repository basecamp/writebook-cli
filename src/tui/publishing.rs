//! Whether a book is public, and at what address.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};

use super::data::{self, Book, Publication};
use super::help::Binding;
use super::{Id, Message, Reply, Request, next_id, views};

pub struct Publishing {
    pub id: Id,
    book_page_id: Id,
    host: String,
    book_id: u64,
    published: bool,
    slug: String,
    url: String,
    editing_slug: Option<String>,
    showing_qr: bool,
    saving: bool,
}

pub enum Action {
    Nothing,
    Close,
    Copy(String),
    Request(Request),
}

impl Publishing {
    pub fn new(host: &str, book_page_id: Id, book: &Book) -> Publishing {
        Publishing {
            id: next_id(),
            book_page_id,
            host: host.to_string(),
            book_id: book.id,
            published: book.published,
            slug: book.slug.clone(),
            url: format!("{host}/{}/{}", book.id, book.slug),
            editing_slug: None,
            showing_qr: false,
            saving: false,
        }
    }

    pub fn typing(&self) -> bool {
        self.editing_slug.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match &mut self.editing_slug {
            Some(slug) => match key.code {
                KeyCode::Esc => self.editing_slug = None,
                KeyCode::Backspace => {
                    slug.pop();
                }
                KeyCode::Char(character) if views::types_character(key) => slug.push(character),
                KeyCode::Enter if !slug.trim().is_empty() => {
                    let slug = std::mem::take(slug);
                    self.editing_slug = None;
                    return self.publish(self.published, slug);
                }
                _ => {}
            },
            None => match key.code {
                KeyCode::Esc | KeyCode::Enter => return Action::Close,
                KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') | KeyCode::Char(' ')
                    if !self.saving =>
                {
                    return self.publish(!self.published, self.slug.clone());
                }
                KeyCode::Char('c') if self.published => return Action::Copy(self.url.clone()),
                KeyCode::Char('q') if self.published => self.showing_qr = !self.showing_qr,
                KeyCode::Char('e') if !self.saving => self.editing_slug = Some(self.slug.clone()),
                _ => {}
            },
        }

        Action::Nothing
    }

    /// Done saving: show how the book is published now, if that went through.
    pub fn finished(&mut self, result: &Result<Publication>) {
        self.saving = false;

        if let Ok(publication) = result {
            self.published = publication.published;
            self.slug = publication.slug.clone();
            self.url = publication.url.clone();
        }
    }

    pub fn bindings(&self) -> Vec<Binding> {
        if self.editing_slug.is_some() {
            vec![("enter", "save address"), ("esc", "cancel")]
        } else if self.published {
            vec![
                ("↑↓", "visibility"),
                ("c", "copy address"),
                ("q", "QR code"),
                ("e", "edit address"),
                ("enter", "close"),
            ]
        } else {
            vec![("↑↓", "visibility"), ("e", "edit address"), ("enter", "close")]
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let mut lines = vec![
            views::choice_line("Private", !self.published),
            views::choice_line("Public", self.published),
            Line::from(""),
        ];

        match &self.editing_slug {
            Some(slug) => {
                lines.push(Line::from("Address").dim());
                lines.push(Line::from(vec![
                    Span::from(self.url_without_slug()).dim(),
                    Span::from(format!("{slug}▏")),
                ]));
            }
            None if self.published => lines.push(Line::from(self.url.clone()).fg(Color::Cyan)),
            None => lines.push(Line::from("Only people with access can read it.").dim()),
        }

        if self.showing_qr && self.published {
            lines.push(Line::from(""));
            lines.extend(views::qr_lines(&self.url, &self.host));
        }

        if self.saving {
            lines.push(Line::from(""));
            lines.push(Line::from("Saving…").dim());
        }

        views::window(frame, area, " Publishing ", lines, 50);
    }

    fn publish(&mut self, published: bool, slug: String) -> Action {
        let id = self.id;
        let book_page_id = self.book_page_id;
        let host = self.host.clone();
        let book_id = self.book_id;
        self.saving = true;

        Action::Request(Box::new(move || {
            Message::To(
                id,
                Reply::Published(book_page_id, data::publish(&host, book_id, published, &slug)),
            )
        }))
    }

    fn url_without_slug(&self) -> String {
        match self.url.rsplit_once('/') {
            Some((start, _)) => format!("{start}/"),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_only_the_slug_of_the_public_address() {
        let publishing = Publishing {
            id: 1,
            book_page_id: 2,
            host: "https://books.example.com".to_string(),
            book_id: 2,
            published: true,
            slug: "guide".to_string(),
            url: "https://books.example.com/2/guide".to_string(),
            editing_slug: None,
            showing_qr: false,
            saving: false,
        };

        assert_eq!(publishing.url_without_slug(), "https://books.example.com/2/");
    }
}
