//! An instance's people, as on Writebook's settings page: the link that
//! invites more, and everyone with access, whom administrators can promote,
//! demote or remove. Nobody changes their own role or removes themselves.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::data::{self, People, Person};
use super::{Id, Loading, Message, Reply, Request, next_id, views};

pub struct SettingsPage {
    pub id: Id,
    pub host: String,
    pub people: Loading<People>,
    invite_resets: u64,
    selected: usize,
    offset: usize,
    showing_qr: bool,
}

pub enum Action {
    Nothing,
    Back,
    Copy(String),
    ResetInvite,
    SetRole(u64, bool),
    Remove(Person),
    Profile,
}

impl SettingsPage {
    pub fn new(host: String) -> SettingsPage {
        SettingsPage {
            id: next_id(),
            host,
            people: Loading::Pending,
            invite_resets: 0,
            selected: 0,
            offset: 0,
            showing_qr: false,
        }
    }

    pub fn load(&self) -> Request {
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || Message::To(id, Reply::PeopleLoaded(data::people(&host))))
    }

    /// The people as loaded, the selection kept on one of them.
    pub fn loaded(&mut self, people: anyhow::Result<People>) {
        self.people = Loading::from(people);
        self.selected = self.selected.min(self.count().saturating_sub(1));
    }

    pub fn set_role(&self, person_id: u64, administrator: bool) -> Request {
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || {
            Message::To(
                id,
                Reply::PeopleChanged(data::set_role(&host, person_id, administrator)),
            )
        })
    }

    pub fn remove(&self, person_id: u64) -> Request {
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || Message::To(id, Reply::PeopleChanged(data::remove_person(&host, person_id))))
    }

    pub fn reset_invite(&mut self) -> Request {
        self.invite_resets += 1;
        let reset = self.invite_resets;
        let id = self.id;
        let host = self.host.clone();
        Box::new(move || Message::To(id, Reply::InviteReset(reset, data::reset_invite(&host))))
    }

    /// Show the link from a reset, unless another reset was asked for since.
    pub fn invite_reset(&mut self, reset: u64, url: String) {
        if reset == self.invite_resets
            && let Loading::Ready(people) = &mut self.people
        {
            people.join_url = Some(url);
        }
    }

    pub fn selected(&self) -> Option<&Person> {
        self.people.ready()?.people.get(self.selected)
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Back,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Action::Nothing
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.count().saturating_sub(1));
                Action::Nothing
            }
            KeyCode::Left | KeyCode::Right if self.changeable() => match self.selected() {
                Some(person) => Action::SetRole(person.id, !person.administrator()),
                None => Action::Nothing,
            },
            KeyCode::Char('d') if self.changeable() => match self.selected() {
                Some(person) => Action::Remove(person.clone()),
                None => Action::Nothing,
            },
            KeyCode::Enter if self.selected().is_some_and(|it| it.current) => Action::Profile,
            KeyCode::Char('c') => match self.invite() {
                Some(invite) => Action::Copy(invite),
                None => Action::Nothing,
            },
            KeyCode::Char('q') if self.invite().is_some() => {
                self.showing_qr = !self.showing_qr;
                Action::Nothing
            }
            KeyCode::Char('r') if self.invite().is_some() && self.administering() => Action::ResetInvite,
            _ => Action::Nothing,
        }
    }

    pub fn bindings(&self) -> Vec<(&'static str, &'static str)> {
        let mut bindings = vec![("↑↓", "select")];

        if self.selected().is_some_and(|it| it.current) {
            bindings.push(("enter", "your profile"));
        }

        if self.changeable() {
            bindings.extend([("←→", "role"), ("d", "remove")]);
        }

        if self.invite().is_some() {
            bindings.extend([("c", "copy invite"), ("q", "QR code")]);
        }

        if self.invite().is_some() && self.administering() {
            bindings.push(("r", "new invite link"));
        }

        bindings.extend([("esc", "books"), ("ctrl+c", "quit")]);
        bindings
    }

    /// The people below breadcrumbs from the instance, named by `instance`.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, instance: &str) {
        let area = views::breadcrumbs(frame, area, &[instance, "Settings"]);

        let people = match &self.people {
            Loading::Ready(people) => people,
            Loading::Pending => return views::placeholder(frame, area, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, area, error),
        };

        let width = area.width.min(64);
        let area = Rect {
            x: area.x + (area.width - width) / 2,
            width,
            ..area
        };
        let invite_height = match (&people.join_url, self.showing_qr) {
            (None, _) => 0,
            (Some(url), true) => 6 + views::qr_lines(url, &self.host).len() as u16,
            (Some(_), false) => 5,
        };
        let [invite, _, list] = Layout::vertical([
            Constraint::Length(invite_height),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .areas(area);

        if let Some(url) = &people.join_url {
            let mut lines = vec![
                Line::from("Share to invite more people").bold().centered(),
                Line::default(),
                Line::from(url.clone()).fg(Color::Cyan).centered(),
            ];

            if self.showing_qr {
                lines.push(Line::default());
                lines.extend(views::qr_lines(url, &self.host).into_iter().map(Line::centered));
            }

            let block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::DarkGray));
            frame.render_widget(Paragraph::new(lines).block(block), invite);
        }

        views::scroll_into_view(&mut self.offset, self.selected, usize::from(list.height).max(1));

        let lines: Vec<Line> = people
            .people
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(usize::from(list.height))
            .map(|(index, person)| person_line(person, index == self.selected, people.can_administer, list.width))
            .collect();
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Left), list);
    }

    fn administering(&self) -> bool {
        self.people.ready().is_some_and(|it| it.can_administer)
    }

    fn changeable(&self) -> bool {
        self.administering() && self.selected().is_some_and(|it| !it.current)
    }

    fn invite(&self) -> Option<String> {
        self.people.ready()?.join_url.clone()
    }

    fn count(&self) -> usize {
        self.people.count(|it| it.people.len())
    }
}

fn person_line(person: &Person, selected: bool, can_administer: bool, width: u16) -> Line<'static> {
    let name = if person.current {
        format!("{} (you)", person.name)
    } else {
        person.name.clone()
    };
    let role = if person.administrator() {
        "administrator"
    } else {
        "member"
    };
    let role = if can_administer && !person.current {
        format!("‹ {role} ›")
    } else {
        role.to_string()
    };

    let marker = if selected { "› " } else { "  " };
    let used = marker.width() + name.width() + role.width() + 2;
    let dots = ".".repeat(usize::from(width).saturating_sub(used));

    let name = Span::from(format!("{marker}{name}"));
    let role = Span::from(role);
    let (name, role) = if selected {
        (name.fg(Color::Blue).bold(), role.fg(Color::Blue).bold())
    } else {
        (name, role.dim())
    };

    Line::from(vec![name, Span::from(format!(" {dots} ")).fg(Color::DarkGray), role])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn page(can_administer: bool) -> SettingsPage {
        let mut page = SettingsPage::new("https://books.example.com".to_string());
        page.people = Loading::Ready(People {
            join_url: Some("https://books.example.com/join/abc".to_string()),
            can_administer,
            people: vec![
                Person {
                    id: 1,
                    name: "Me".to_string(),
                    role: "administrator".to_string(),
                    current: true,
                },
                Person {
                    id: 2,
                    name: "Kevin".to_string(),
                    role: "member".to_string(),
                    current: false,
                },
            ],
        });
        page
    }

    #[test]
    fn dots_fill_a_row_to_the_width_with_wide_characters_in_the_name() {
        let person = Person {
            id: 3,
            name: "山田太郎".to_string(),
            role: "member".to_string(),
            current: false,
        };

        assert_eq!(person_line(&person, false, false, 30).width(), 30);
    }

    #[test]
    fn keeps_the_selected_person_in_view_and_on_someone_after_a_reload() {
        let mut page = page(true);
        let everyone: Vec<Person> = (1..=40)
            .map(|id| Person {
                id,
                name: format!("Person {id}"),
                role: "member".to_string(),
                current: id == 1,
            })
            .collect();
        page.loaded(Ok(People {
            join_url: None,
            can_administer: true,
            people: everyone.clone(),
        }));

        for _ in 0..39 {
            page.key(press(KeyCode::Down));
        }

        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).unwrap();
        terminal
            .draw(|frame| page.draw(frame, frame.area(), "books.example.com"))
            .unwrap();
        assert!(
            views::screen_text(&terminal)
                .iter()
                .any(|it| it.starts_with("› Person 40"))
        );

        page.loaded(Ok(People {
            join_url: None,
            can_administer: true,
            people: everyone[..3].to_vec(),
        }));
        assert_eq!(page.selected().map(|it| it.id), Some(3));
    }

    #[test]
    fn nobody_changes_their_own_role_or_removes_themselves() {
        let mut page = page(true);

        assert!(matches!(page.key(press(KeyCode::Right)), Action::Nothing));
        assert!(matches!(page.key(press(KeyCode::Char('d'))), Action::Nothing));

        page.key(press(KeyCode::Down));
        assert!(matches!(page.key(press(KeyCode::Right)), Action::SetRole(2, true)));
        assert!(matches!(
            page.key(press(KeyCode::Char('d'))),
            Action::Remove(Person { id: 2, .. })
        ));
    }

    #[test]
    fn only_administrators_change_people_and_reset_the_invite() {
        let mut page = page(false);
        page.key(press(KeyCode::Down));

        assert!(matches!(page.key(press(KeyCode::Right)), Action::Nothing));
        assert!(matches!(page.key(press(KeyCode::Char('d'))), Action::Nothing));
        assert!(
            matches!(page.key(press(KeyCode::Char('c'))), Action::Copy(_)),
            "everyone can share the invite, as on the web"
        );
        assert!(matches!(page.key(press(KeyCode::Char('r'))), Action::Nothing));
    }

    #[test]
    fn copies_and_resets_the_invite_link() {
        let mut page = page(true);

        assert!(
            matches!(page.key(press(KeyCode::Char('c'))), Action::Copy(url) if url == "https://books.example.com/join/abc")
        );
        assert!(matches!(page.key(press(KeyCode::Char('r'))), Action::ResetInvite));
    }
}
