//! Adding a page, picture or section to a book: choosing which, then filling
//! in its form.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use std::path::Path;

use super::data::NewLeaf;
use super::help::Binding;
use super::views;

#[derive(Default)]
pub struct Add {
    choice: usize,
    form: Option<Form>,
}

pub enum Action {
    Nothing,
    Close,
    ChooseFile,
    Submit,
}

struct Form {
    kind: Kind,
    fields: Vec<Field>,
    focus: usize,
}

#[derive(Clone, Copy)]
enum Kind {
    Page,
    Picture,
    Section,
}

struct Field {
    label: &'static str,
    value: String,
    file: bool,
}

impl Add {
    /// Whether keys go into the form's fields.
    pub fn typing(&self) -> bool {
        self.form.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match &mut self.form {
            Some(_) if key.code == KeyCode::Esc => {
                self.form = None;
                Action::Nothing
            }
            Some(form) => form.key(key),
            None => self.choice_key(key),
        }
    }

    pub fn use_file(&mut self, path: &Path) {
        if let Some(form) = &mut self.form
            && let Some(field) = form.fields.iter_mut().find(|it| it.file)
        {
            field.value = path.to_string_lossy().into_owned();
        }
    }

    /// The page, picture or section to add, once every field is filled in.
    pub fn new_leaf(&self) -> Option<NewLeaf> {
        let form = self
            .form
            .as_ref()
            .filter(|form| form.fields.iter().all(|it| !it.value.trim().is_empty()))?;
        let value = |index: usize| form.fields[index].value.trim().to_string();

        match form.kind {
            Kind::Page => Some(NewLeaf::Page { title: value(0) }),
            Kind::Section => Some(NewLeaf::Section { title: value(0) }),
            Kind::Picture => Some(NewLeaf::Picture {
                caption: value(0),
                path: value(1),
            }),
        }
    }

    pub fn bindings(&self) -> Vec<Binding> {
        match &self.form {
            None => vec![("↑↓", "select"), ("enter", "choose"), ("esc", "close")],
            Some(form) => vec![
                ("tab", "next"),
                ("enter", form.enter_name()),
                ("esc", "back"),
                ("ctrl+s", "add"),
            ],
        }
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        match &self.form {
            None => {
                let lines = ["Page", "Picture", "Section"]
                    .iter()
                    .enumerate()
                    .map(|(index, choice)| views::choice_line(choice, index == self.choice))
                    .collect();
                views::window(frame, area, " Add ", lines, 44);
            }
            Some(form) => form.draw(frame, area),
        }
    }

    fn choice_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Up | KeyCode::Char('k') => self.choice = self.choice.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.choice = (self.choice + 1).min(Kind::ALL.len() - 1),
            KeyCode::Enter => self.form = Some(Form::new(Kind::ALL[self.choice])),
            _ => {}
        }

        Action::Nothing
    }
}

impl Form {
    fn new(kind: Kind) -> Form {
        Form {
            kind,
            fields: kind.fields(),
            focus: 0,
        }
    }

    fn key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Submit;
        }

        match key.code {
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % self.stops(),
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + self.stops() - 1) % self.stops(),
            KeyCode::Enter if self.focus == self.cancel_button() => return Action::Close,
            KeyCode::Enter if self.field().is_some_and(|it| it.file) => return Action::ChooseFile,
            KeyCode::Enter if self.focus == self.add_button() => return Action::Submit,
            KeyCode::Enter => self.focus += 1,
            KeyCode::Backspace => {
                if let Some(field) = self.typed_field() {
                    field.value.pop();
                }
            }
            KeyCode::Char(character) if views::types_character(key) => {
                if let Some(field) = self.typed_field() {
                    field.value.push(character);
                }
            }
            _ => {}
        }

        Action::Nothing
    }

    fn stops(&self) -> usize {
        self.fields.len() + 2
    }

    fn cancel_button(&self) -> usize {
        self.fields.len() + 1
    }

    fn field(&self) -> Option<&Field> {
        self.fields.get(self.focus)
    }

    fn add_button(&self) -> usize {
        self.fields.len()
    }

    fn typed_field(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.focus).filter(|it| !it.file)
    }

    fn enter_name(&self) -> &'static str {
        match self.focus {
            focus if focus == self.add_button() => "add",
            focus if focus == self.cancel_button() => "cancel",
            _ if self.field().is_some_and(|it| it.file) => "choose a file",
            _ => "next",
        }
    }

    fn draw(&self, frame: &mut Frame, area: Rect) {
        let height = self.fields.len() as u16 * 3 + 2;
        let window = views::centered(area, 50, height + 4);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Blue))
            .title(format!(" Add a {} ", self.kind.name()).bold())
            .padding(ratatui::widgets::Padding::uniform(1));
        let inside = block.inner(window);
        frame.render_widget(Clear, window);
        frame.render_widget(block, window);

        let mut rows: Vec<Constraint> = self.fields.iter().map(|_| Constraint::Length(3)).collect();
        rows.extend([Constraint::Length(1), Constraint::Length(1)]);
        let areas = Layout::vertical(rows).split(inside);

        for (index, field) in self.fields.iter().enumerate() {
            let focused = index == self.focus;
            let text = match (field.file, field.value.is_empty()) {
                (true, true) => Line::from("Press enter to choose a file").dim(),
                (true, false) => Line::from(field.value.clone()),
                (false, _) => views::typed(field.label, &field.value, focused),
            };
            views::input(frame, areas[index], field.label, text, focused);
        }

        let buttons = views::buttons(&[
            ("Add", self.focus == self.add_button()),
            ("Cancel", self.focus == self.cancel_button()),
        ]);
        frame.render_widget(Paragraph::new(buttons), areas[self.fields.len() + 1]);
    }
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Page, Kind::Picture, Kind::Section];

    fn fields(self) -> Vec<Field> {
        match self {
            Kind::Picture => vec![Field::new("Caption"), Field::file("Image")],
            _ => vec![Field::new("Title")],
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Page => "page",
            Kind::Picture => "picture",
            Kind::Section => "section",
        }
    }
}

impl Field {
    fn new(label: &'static str) -> Field {
        Field {
            label,
            value: String::new(),
            file: false,
        }
    }

    fn file(label: &'static str) -> Field {
        Field {
            label,
            value: String::new(),
            file: true,
        }
    }
}
