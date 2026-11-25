//! Yes or no questions, answered by choosing one and pressing enter.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::help::Binding;
use super::{Overlay, Screen, views};

pub struct Confirm {
    pub question: Question,
    yes: bool,
}

/// What's asked, and so what a yes does.
pub enum Question {
    /// Deleting the selected page of the book, or the page being read.
    DeleteLeaf,
    DeleteBook,
    /// Throwing away the changes in the page editor.
    Discard,
    RemovePerson,
    ResetInvite,
    /// Quitting, over whatever overlay was open, to go back to when staying.
    Quit(Option<Box<Overlay>>),
}

impl Confirm {
    /// Quitting starts on Yes; everything else on No, so enter alone never
    /// deletes anything.
    pub fn new(question: Question) -> Confirm {
        let yes = matches!(question, Question::Quit(_));
        Confirm { question, yes }
    }

    /// The answer once there is one: enter takes the chosen one, esc is no.
    pub fn answer(&mut self, key: KeyEvent) -> Option<bool> {
        match key.code {
            KeyCode::Enter => Some(self.yes),
            KeyCode::Esc => Some(false),
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
                self.yes = true;
                None
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j') => {
                self.yes = false;
                None
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.yes = !self.yes;
                None
            }
            _ => None,
        }
    }

    pub fn bindings(&self) -> Vec<Binding> {
        match self.question {
            Question::Discard => vec![("←→", "choose"), ("enter", "answer"), ("esc", "keep editing")],
            Question::Quit(_) => vec![("←→", "choose"), ("enter", "answer"), ("esc", "stay"), ("ctrl+c", "quit")],
            _ => vec![("←→", "choose"), ("enter", "answer"), ("esc", "cancel")],
        }
    }

    /// The question about what's on the screen: the page, book or person it's about.
    pub fn draw(&self, frame: &mut Frame, area: Rect, screen: &Screen) {
        match &self.question {
            Question::DeleteLeaf => {
                let leaf = match screen {
                    Screen::Book(page) => page.selected_leaf(),
                    Screen::Leaf(page, _) => Some(page.leaf()),
                    _ => None,
                };

                if let Some(leaf) = leaf {
                    self.ask(frame, area, " Delete ", vec![Line::from(format!("Delete “{}”?", views::leaf_title(leaf)))], 44);
                }
            }
            Question::DeleteBook => {
                if let Screen::Edit(page, _) = screen
                    && let Some(form) = page.form.ready()
                {
                    let lines = vec![Line::from(format!("Delete “{}” and all its pages?", form.title)), Line::from("This can't be undone.").dim()];
                    self.ask(frame, area, " Delete book ", lines, 50);
                }
            }
            Question::Discard => self.ask(frame, area, " Stop editing ", vec![Line::from("Throw away your changes to this page?")], 50),
            Question::RemovePerson => {
                if let Screen::Settings(page, _) = screen
                    && let Some(person) = page.selected()
                {
                    let lines = vec![Line::from(format!("Remove {}?", person.name)), Line::from("They won't be able to sign in any more.").dim()];
                    self.ask(frame, area, " Remove ", lines, 50);
                }
            }
            Question::ResetInvite => {
                let lines = vec![Line::from("Make a new invite link?"), Line::from("The current link will stop working.").dim()];
                self.ask(frame, area, " New invite link ", lines, 50);
            }
            Question::Quit(_) => self.ask(frame, area, " Quit ", vec![Line::from("Quit writebook tui?")], 44),
        }
    }

    /// The question's lines, then Yes and No side by side, the chosen one filled in.
    fn ask(&self, frame: &mut Frame, area: Rect, title: &str, mut lines: Vec<Line>, width: u16) {
        lines.push(Line::from(""));
        lines.push(views::buttons(&[("Yes", self.yes), ("No", !self.yes)]));
        views::window(frame, area, title, lines, width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_confirmation_is_answered_by_choosing_and_pressing_enter() {
        let mut confirm = Confirm::new(Question::DeleteLeaf);

        assert_eq!(confirm.answer(press(KeyCode::Char('y'))), None);
        assert_eq!(confirm.answer(press(KeyCode::Enter)), Some(false));
        assert_eq!(confirm.answer(press(KeyCode::Left)), None);
        assert_eq!(confirm.answer(press(KeyCode::Enter)), Some(true));
        assert_eq!(confirm.answer(press(KeyCode::Tab)), None);
        assert_eq!(confirm.answer(press(KeyCode::Enter)), Some(false));
        assert_eq!(Confirm::new(Question::Quit(None)).answer(press(KeyCode::Esc)), Some(false));
    }
}
