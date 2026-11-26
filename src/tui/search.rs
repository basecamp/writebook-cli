//! Searching a book, or every book: a field to type in, and the pages that
//! match, each as its title and the excerpt around the match.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::data::Found;
use super::help::Binding;
use super::{Id, Loading, next_id, views};
use crate::web;

pub struct Search {
    pub id: Id,
    /// The book being searched, or none for every book.
    pub book_id: Option<u64>,
    scope: String,
    query: String,
    searched: Option<String>,
    pub results: Option<Loading<Vec<Found>>>,
    selected: usize,
    offset: usize,
}

pub enum Action {
    Nothing,
    Close,
    Search(String),
    Open(Found),
}

impl Search {
    pub fn new(book_id: Option<u64>, scope: String) -> Search {
        Search {
            id: next_id(),
            book_id,
            scope,
            query: String::new(),
            searched: None,
            results: None,
            selected: 0,
            offset: 0,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Close,
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::Nothing
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.count().saturating_sub(1));
                Action::Nothing
            }
            KeyCode::Enter => self.enter(),
            KeyCode::Backspace => {
                self.query.pop();
                Action::Nothing
            }
            KeyCode::Char(character) if views::types_character(key) => {
                self.query.push(character);
                Action::Nothing
            }
            _ => Action::Nothing,
        }
    }

    /// Show what a search found, unless it was for words searched before the
    /// last ones.
    pub fn arrived(&mut self, query: &str, found: anyhow::Result<Vec<Found>>) {
        if self.searched.as_deref() == Some(query) {
            self.results = Some(Loading::from(found));
            self.selected = self.selected.min(self.count().saturating_sub(1));
        }
    }

    pub fn bindings(&self) -> Vec<Binding> {
        let fresh =
            self.searched.as_deref() == Some(self.query.trim()) && self.found().is_some_and(|it| !it.is_empty());

        if fresh {
            vec![("↑↓", "select"), ("enter", "open"), ("esc", "close")]
        } else {
            vec![("enter", "search"), ("esc", "close")]
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let inside = views::large_window(frame, area, &format!(" Search {} ", self.scope));

        let [input, _, results] =
            Layout::vertical([Constraint::Length(3), Constraint::Length(1), Constraint::Fill(1)]).areas(inside);
        let field = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .padding(ratatui::widgets::Padding::horizontal(1));
        frame.render_widget(
            Paragraph::new(Line::from(format!("{}▏", self.query))).block(field),
            input,
        );

        let lines = self.result_lines(results);
        frame.render_widget(Paragraph::new(lines), results);
    }

    fn enter(&mut self) -> Action {
        let query = self.query.trim().to_string();
        let failed = matches!(self.results, Some(Loading::Failed(_)));

        if query.is_empty() {
            Action::Nothing
        } else if self.searched.as_deref() != Some(&query) || failed {
            self.searched = Some(query.clone());
            self.results = Some(Loading::Pending);
            self.selected = 0;
            self.offset = 0;
            Action::Search(query)
        } else {
            match self.found().and_then(|it| it.get(self.selected)) {
                Some(found) => Action::Open(found.clone()),
                None => Action::Nothing,
            }
        }
    }

    fn result_lines(&mut self, area: Rect) -> Vec<Line<'static>> {
        let height = usize::from(area.height);
        let selected = self.selected;

        if selected < self.offset {
            self.offset = selected;
        } else if selected >= self.offset + height {
            self.offset = selected + 1 - height;
        }

        match &self.results {
            None => vec![Line::from("Type what to look for, then press enter.").dim()],
            Some(Loading::Pending) => vec![Line::from("Searching…").dim()],
            Some(Loading::Failed(error)) => vec![Line::from(error.clone()).red()],
            Some(Loading::Ready(found)) if found.is_empty() => vec![Line::from("Nothing matches.").dim()],
            Some(Loading::Ready(found)) => found
                .iter()
                .enumerate()
                .skip(self.offset)
                .take(height)
                .map(|(index, found)| result_line(found, index == selected, self.book_id.is_none()))
                .collect(),
        }
    }

    fn found(&self) -> Option<&[Found]> {
        match &self.results {
            Some(Loading::Ready(found)) => Some(found),
            _ => None,
        }
    }

    fn count(&self) -> usize {
        match self.found() {
            Some(found) => found.len(),
            None => 0,
        }
    }
}

fn result_line(found: &Found, selected: bool, across_books: bool) -> Line<'static> {
    let marker = Span::from(if selected { "› " } else { "  " }).fg(Color::Blue).bold();
    let title = Span::from(format!("{}: ", found.title)).bold();
    let title = if selected { title.fg(Color::Blue) } else { title };

    let mut spans = vec![marker];

    if across_books && !found.book.is_empty() {
        spans.push(Span::from(found.book.clone()).dim());
        spans.push(Span::from(" · ").dim());
    }

    spans.push(title);
    spans.extend(marked(&found.snippet).into_iter().map(|(text, matched)| {
        if matched {
            Span::styled(text, Style::default().fg(Color::Black).bg(Color::Yellow))
        } else {
            Span::from(text)
        }
    }));

    Line::from(spans)
}

fn marked(snippet: &str) -> Vec<(String, bool)> {
    let flat = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut parts = Vec::new();
    let mut rest = flat.as_str();

    while let Some(start) = rest.find("<mark>") {
        parts.push((web::unescape(&rest[..start]), false));
        rest = &rest[start + "<mark>".len()..];

        let end = rest.find("</mark>").unwrap_or(rest.len());
        parts.push((web::unescape(&rest[..end]), true));
        rest = rest.get(end + "</mark>".len()..).unwrap_or("");
    }

    parts.push((web::unescape(rest), false));
    parts.retain(|(text, _)| !text.is_empty());
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn found(title: &str) -> Found {
        Found {
            book: "Handbook".to_string(),
            title: title.to_string(),
            snippet: String::new(),
            book_id: Some(2),
            leaf_id: Some(8),
        }
    }

    #[test]
    fn enter_searches_for_new_words_and_opens_a_result_for_the_same_ones() {
        let mut search = Search::new(Some(2), "The Trail Guide".to_string());

        assert!(
            matches!(search.key(press(KeyCode::Enter)), Action::Nothing),
            "nothing typed, nothing searched"
        );

        for character in "rabbit".chars() {
            search.key(press(KeyCode::Char(character)));
        }
        assert!(matches!(search.key(press(KeyCode::Enter)), Action::Search(query) if query == "rabbit"));

        search.arrived("rabbit", Ok(vec![found("Introduction"), found("Getting Around")]));
        search.key(press(KeyCode::Down));
        assert!(matches!(search.key(press(KeyCode::Enter)), Action::Open(found) if found.title == "Getting Around"));

        search.key(press(KeyCode::Char('s')));
        assert!(matches!(search.key(press(KeyCode::Enter)), Action::Search(query) if query == "rabbits"));
        assert!(matches!(search.key(press(KeyCode::Esc)), Action::Close));
    }
}
