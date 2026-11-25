//! Your own profile: your name, and your email and password, which the CLI
//! can only change with your current password.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::data::{self, Profile};
use super::{Loading, Message, Request, views};

pub struct ProfilePage {
    pub host: String,
    pub profile: Loading<Profile>,
    /// Name, email, new password, current password.
    fields: [String; 4],
    original_email: String,
    focus: usize,
    pub saving: bool,
}

const LABELS: [&str; 4] = ["Name", "Email", "New password", "Current password"];
const SAVE: usize = 4;

pub enum Action {
    Nothing,
    Back,
    Save,
}

impl ProfilePage {
    pub fn new(host: String) -> ProfilePage {
        ProfilePage { host, profile: Loading::Pending, fields: Default::default(), original_email: String::new(), focus: 0, saving: false }
    }

    pub fn load(&self) -> Request {
        let host = self.host.clone();

        Box::new(move || {
            let profile = data::profile(&host);
            Message::ProfileLoaded(host, profile)
        })
    }

    pub fn loaded(&mut self, profile: anyhow::Result<Profile>) {
        if let Ok(profile) = &profile {
            self.fields = [profile.name.clone(), profile.email.clone(), String::new(), String::new()];
            self.original_email = profile.email.clone();
        }
        self.profile = Loading::from(profile);
    }

    pub fn name(&self) -> &str {
        self.fields[0].trim()
    }

    pub fn email(&self) -> &str {
        self.fields[1].trim()
    }

    pub fn password(&self) -> &str {
        &self.fields[2]
    }

    pub fn current_password(&self) -> &str {
        &self.fields[3]
    }

    /// Changing the email or password takes the current one.
    pub fn needs_current_password(&self) -> bool {
        self.email() != self.original_email || !self.password().is_empty()
    }

    pub fn typing(&self) -> bool {
        self.focus < SAVE
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.saving || !matches!(self.profile, Loading::Ready(_)) {
            return if key.code == KeyCode::Esc { Action::Back } else { Action::Nothing };
        }

        match key.code {
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => return Action::Save,
            KeyCode::Esc => return Action::Back,
            KeyCode::Tab | KeyCode::Down => self.step(1),
            KeyCode::BackTab | KeyCode::Up => self.step(-1),
            KeyCode::Enter if self.focus == SAVE => return Action::Save,
            KeyCode::Enter => self.step(1),
            KeyCode::Backspace if self.typing() => {
                self.fields[self.focus].pop();
            }
            KeyCode::Char(character) if self.typing() => self.fields[self.focus].push(character),
            _ => {}
        }

        Action::Nothing
    }

    /// The fields to move through: the current password only once it's needed.
    fn stops(&self) -> Vec<usize> {
        let mut stops = vec![0, 1, 2];

        if self.needs_current_password() {
            stops.push(3);
        }

        stops.push(SAVE);
        stops
    }

    fn step(&mut self, by: isize) {
        let stops = self.stops();
        let at = stops.iter().position(|it| *it == self.focus).unwrap_or(0) as isize;
        self.focus = stops[(at + by).rem_euclid(stops.len() as isize) as usize];
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
        let id = self.profile.ready()?.id;
        let host = self.host.clone();
        let name = self.name().to_string();
        let email = self.email().to_string();
        let password = self.password().to_string();
        let current = self.current_password().to_string();
        self.saving = true;

        Some(Box::new(move || Message::ProfileSaved(data::save_profile(&host, id, &name, &email, &password, &current))))
    }

    /// After saving, the passwords are cleared and the email is the saved one.
    pub fn saved(&mut self) {
        self.original_email = self.email().to_string();
        self.fields[2].clear();
        self.fields[3].clear();
        self.focus = 0;
    }

    pub fn bindings(&self) -> Vec<(&'static str, &'static str)> {
        let enter = if self.focus == SAVE { "save" } else { "next" };
        vec![("tab", "next"), ("enter", enter), ("esc", "settings"), ("ctrl+s", "save")]
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
        let area = Rect { x: area.x + (area.width - width) / 2, width, ..area };
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

        self.field(frame, name, 0, &self.fields[0], "Name");
        self.field(frame, email, 1, &self.fields[1], "Email");
        self.field(frame, password, 2, &dots(&self.fields[2]), "Leave empty to keep your current password");

        if self.needs_current_password() {
            self.field(frame, current, 3, &dots(&self.fields[3]), "Needed to change your email or password");
        }

        let label = if self.saving { "Saving…" } else { "Save" };
        frame.render_widget(Paragraph::new(views::buttons(&[(label, self.focus == SAVE)])), save);
    }

    /// A framed field, its placeholder showing while it's empty.
    fn field(&self, frame: &mut Frame, area: Rect, index: usize, value: &str, placeholder: &str) {
        let focused = self.focus == index;
        let text = if value.is_empty() && !focused { Line::from(placeholder.to_string()).dim() } else { views::typed(placeholder, value, focused) };
        views::input(frame, area, LABELS[index], text, focused);
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
        page.loaded(Ok(Profile { id: 1, name: "Stanko".to_string(), email: "stanko@example.com".to_string() }));
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
        assert_eq!(page.current_password(), "y", "the current password field shows up once it's needed");
    }

    #[test]
    fn enter_moves_on_and_saves_from_the_button() {
        let mut page = page();

        for _ in 0..3 {
            assert!(matches!(page.key(press(KeyCode::Enter)), Action::Nothing));
        }
        assert!(matches!(page.key(press(KeyCode::Enter)), Action::Save));
        assert!(matches!(page.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)), Action::Save));
        assert!(matches!(page.key(press(KeyCode::Esc)), Action::Back));
    }
}
