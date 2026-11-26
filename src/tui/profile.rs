//! Your own profile: your name, and your email and password, which the CLI
//! can only change with your current password.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::data::{self, Profile};
use super::{Id, Loading, Message, Reply, Request, next_id, views};

pub struct ProfilePage {
    pub id: Id,
    pub host: String,
    pub profile: Loading<Profile>,
    name: String,
    email: String,
    new_password: String,
    current_password: String,
    original_email: String,
    focus: Field,
    pub saving: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Name,
    Email,
    NewPassword,
    CurrentPassword,
    Save,
}

pub enum Action {
    Nothing,
    Back,
    Save,
}

impl ProfilePage {
    pub fn new(host: String) -> ProfilePage {
        ProfilePage {
            id: next_id(),
            host,
            profile: Loading::Pending,
            name: String::new(),
            email: String::new(),
            new_password: String::new(),
            current_password: String::new(),
            original_email: String::new(),
            focus: Field::Name,
            saving: false,
        }
    }

    pub fn load(&self) -> Request {
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || Message::To(id, Reply::ProfileLoaded(data::profile(&host))))
    }

    pub fn loaded(&mut self, profile: anyhow::Result<Profile>) {
        if let Ok(profile) = &profile {
            self.name = profile.name.clone();
            self.email = profile.email.clone();
            self.new_password.clear();
            self.current_password.clear();
            self.original_email = profile.email.clone();
        }
        self.profile = Loading::from(profile);
    }

    pub fn name(&self) -> &str {
        self.name.trim()
    }

    pub fn email(&self) -> &str {
        self.email.trim()
    }

    pub fn password(&self) -> &str {
        &self.new_password
    }

    pub fn current_password(&self) -> &str {
        &self.current_password
    }

    /// Changing the email or password takes the current one.
    pub fn needs_current_password(&self) -> bool {
        self.email() != self.original_email || !self.password().is_empty()
    }

    pub fn typing(&self) -> bool {
        self.focus != Field::Save
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.saving || !matches!(self.profile, Loading::Ready(_)) {
            return if key.code == KeyCode::Esc {
                Action::Back
            } else {
                Action::Nothing
            };
        }

        match key.code {
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => return Action::Save,
            KeyCode::Esc => return Action::Back,
            KeyCode::Tab | KeyCode::Down => self.step(1),
            KeyCode::BackTab | KeyCode::Up => self.step(-1),
            KeyCode::Enter if self.focus == Field::Save => return Action::Save,
            KeyCode::Enter => self.step(1),
            KeyCode::Backspace => {
                if let Some(text) = self.focused_text() {
                    text.pop();
                }
            }
            KeyCode::Char(character) if views::types_character(key) => {
                if let Some(text) = self.focused_text() {
                    text.push(character);
                }
            }
            _ => {}
        }

        Action::Nothing
    }

    /// Why the profile can't be saved as it is.
    pub fn problem(&self) -> Option<&'static str> {
        if self.name().is_empty() || self.email().is_empty() {
            Some("Your name and email can't be empty.")
        } else if self.needs_current_password() && self.current_password().is_empty() {
            Some("Fill in your current password to change your email or password.")
        } else {
            None
        }
    }

    pub fn save(&mut self) -> Option<Request> {
        let profile_id = self.profile.ready()?.id;
        let id = self.id;
        let host = self.host.clone();
        let name = self.name().to_string();
        let email = self.email().to_string();
        let password = self.password().to_string();
        let current = self.current_password().to_string();
        self.saving = true;

        Some(Box::new(move || {
            Message::To(
                id,
                Reply::ProfileSaved(data::save_profile(
                    &host, profile_id, &name, &email, &password, &current,
                )),
            )
        }))
    }

    /// After saving, the passwords are cleared and the email is the saved one.
    pub fn saved(&mut self) {
        self.original_email = self.email().to_string();
        self.new_password.clear();
        self.current_password.clear();
        self.focus = Field::Name;
    }

    pub fn bindings(&self) -> Vec<(&'static str, &'static str)> {
        let enter = if self.focus == Field::Save { "save" } else { "next" };
        vec![
            ("tab", "next"),
            ("enter", enter),
            ("esc", "settings"),
            ("ctrl+s", "save"),
        ]
    }

    /// The profile below breadcrumbs from the instance, named by `instance`.
    pub fn draw(&self, frame: &mut Frame, area: Rect, instance: &str) {
        let name = self.profile.ready().map(|it| it.name.clone()).unwrap_or_default();
        let area = views::breadcrumbs(frame, area, &[instance, "Settings", name.as_str()]);
        match &self.profile {
            Loading::Pending => return views::placeholder(frame, area, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, area, error),
            Loading::Ready(_) => {}
        }

        let width = area.width.min(64);
        let area = Rect {
            x: area.x + (area.width - width) / 2,
            width,
            ..area
        };
        let current_height = if self.needs_current_password() { 3 } else { 0 };
        let [name, email, _, password, current, _, save] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(current_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        self.field(frame, name, Field::Name, &self.name, "Name");
        self.field(frame, email, Field::Email, &self.email, "Email");
        self.field(
            frame,
            password,
            Field::NewPassword,
            &dots(&self.new_password),
            "Leave empty to keep your current password",
        );

        if self.needs_current_password() {
            self.field(
                frame,
                current,
                Field::CurrentPassword,
                &dots(&self.current_password),
                "Needed to change your email or password",
            );
        }

        let label = if self.saving { "Saving…" } else { "Save" };
        frame.render_widget(
            Paragraph::new(views::buttons(&[(label, self.focus == Field::Save)])),
            save,
        );
    }

    fn focused_text(&mut self) -> Option<&mut String> {
        match self.focus {
            Field::Name => Some(&mut self.name),
            Field::Email => Some(&mut self.email),
            Field::NewPassword => Some(&mut self.new_password),
            Field::CurrentPassword => Some(&mut self.current_password),
            Field::Save => None,
        }
    }

    fn stops(&self) -> Vec<Field> {
        let mut stops = vec![Field::Name, Field::Email, Field::NewPassword];

        if self.needs_current_password() {
            stops.push(Field::CurrentPassword);
        }

        stops.push(Field::Save);
        stops
    }

    fn step(&mut self, by: isize) {
        let stops = self.stops();
        let at = stops.iter().position(|it| *it == self.focus).unwrap_or(0) as isize;
        self.focus = stops[(at + by).rem_euclid(stops.len() as isize) as usize];
    }

    fn field(&self, frame: &mut Frame, area: Rect, field: Field, value: &str, placeholder: &str) {
        let focused = self.focus == field;
        let text = if value.is_empty() && !focused {
            Line::from(placeholder.to_string()).dim()
        } else {
            views::typed(placeholder, value, focused)
        };
        views::input(frame, area, field.label(), text, focused);
    }
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Email => "Email",
            Field::NewPassword => "New password",
            Field::CurrentPassword => "Current password",
            Field::Save => "Save",
        }
    }
}

fn dots(secret: &str) -> String {
    "•".repeat(secret.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn page() -> ProfilePage {
        let mut page = ProfilePage::new("https://books.example.com".to_string());
        page.loaded(Ok(Profile {
            id: 1,
            name: "Stanko".to_string(),
            email: "stanko@example.com".to_string(),
        }));
        page
    }

    #[test]
    fn asks_for_the_current_password_only_to_change_email_or_password() {
        let mut page = page();
        page.key(press(KeyCode::Char('!')));
        assert_eq!(page.name(), "Stanko!");
        assert!(!page.needs_current_password());

        page.key(press(KeyCode::Down));
        page.key(press(KeyCode::Down));
        page.key(press(KeyCode::Char('x')));
        assert_eq!(page.password(), "x");
        assert!(page.needs_current_password());

        page.key(press(KeyCode::Tab));
        page.key(press(KeyCode::Char('y')));
        assert_eq!(
            page.current_password(),
            "y",
            "the current password field shows up once it's needed"
        );
    }

    #[test]
    fn enter_moves_on_and_saves_from_the_button() {
        let mut page = page();

        for _ in 0..3 {
            assert!(matches!(page.key(press(KeyCode::Enter)), Action::Nothing));
        }
        assert!(matches!(page.key(press(KeyCode::Enter)), Action::Save));
        assert!(matches!(
            page.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Action::Save
        ));
        assert!(matches!(page.key(press(KeyCode::Esc)), Action::Back));
    }
}
