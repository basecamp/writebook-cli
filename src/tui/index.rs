//! A book's pages over the page being read, to jump to one.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::help::Binding;
use super::leaf::LeafPage;
use super::{Request, views};

/// The book's pages, to jump to one.
pub struct Index {
    selected: usize,
    offset: usize,
}

pub enum Action {
    Nothing,
    Close,
    /// Another page was turned to; the request loads it.
    Turned(Request),
}

impl Index {
    /// Opens on the page being read.
    pub fn new(page: &LeafPage) -> Index {
        Index {
            selected: page.index,
            offset: 0,
        }
    }

    pub fn key(&mut self, key: KeyEvent, page: &mut LeafPage) -> Action {
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Enter if self.selected == page.index => return Action::Close,
            KeyCode::Enter => return Action::Turned(page.turn_to(self.selected)),
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(page.leaves.len().saturating_sub(1))
            }
            _ => {}
        }

        Action::Nothing
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
            let title = if leaf.title.is_empty() {
                "Untitled".to_string()
            } else {
                leaf.title.clone()
            };
            let marker = if index == selected { "› " } else { "  " };
            let dots = ".".repeat(usize::from(width).saturating_sub(title.width() + 5));
            let style = if leaf.is_section() {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let style = if index == selected {
                style.fg(Color::Blue).add_modifier(Modifier::BOLD)
            } else {
                style
            };
            let here = if index == page.index { " ◂" } else { "" };

            lines.push(Line::from(vec![
                Span::styled(format!("{marker}{title} "), style),
                Span::from(dots).fg(Color::DarkGray),
                Span::from(here).fg(Color::Blue),
            ]));
        }

        views::window(frame, area, " Index ", lines, width + 4);
    }
}
