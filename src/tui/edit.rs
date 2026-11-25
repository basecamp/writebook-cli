//! A book's settings, laid out like Writebook's edit page: the theme and the
//! cover on the left, title, subtitle and author on the right, and who can
//! read and edit it below. The same screen makes a new book.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};

use super::data::{self, BookForm, Role};
use super::images::{Images, Source};
use super::{Loading, Message, Request, theme, views};

const THEMES: [&str; 7] = ["black", "blue", "green", "magenta", "orange", "violet", "white"];

pub struct EditPage {
    pub host: String,
    pub book_id: Option<u64>,
    pub name: String,
    pub form: Loading<BookForm>,
    pub focus: usize,
    pub saving: bool,
}

/// Where the keys go on the screen, top to bottom.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Field {
    Theme,
    Cover,
    Title,
    Subtitle,
    Author,
    Everyone,
    Member(usize),
    Save,
    Delete,
}

/// What a key asks the app to do beyond changing the form.
pub enum Action {
    Nothing,
    Cancel,
    Save,
    /// Ask before deleting the book with this id and title.
    Delete(u64, String),
    ChooseCover,
}

impl EditPage {
    pub fn new(host: String, name: String, book_id: Option<u64>) -> EditPage {
        EditPage { host, book_id, name, form: Loading::Pending, focus: 1, saving: false }
    }

    pub fn load(&self) -> Request {
        let host = self.host.clone();
        let book_id = self.book_id;

        Box::new(move || {
            let form = data::book_form(&host, book_id);
            Message::Form(host, book_id, form)
        })
    }

    /// Why the book can't be saved as it is.
    pub fn problem(&self) -> Option<&'static str> {
        match self.form.ready() {
            Some(form) if form.title.trim().is_empty() => Some("A book needs a title."),
            _ => None,
        }
    }

    pub fn save(&mut self) -> Option<Request> {
        let form = self.form.ready().cloned()?;
        let host = self.host.clone();
        self.saving = true;

        Some(Box::new(move || Message::Saved(data::save_book(&host, &form))))
    }

    pub fn delete(&mut self, book_id: u64) -> Request {
        let host = self.host.clone();
        self.saving = true;

        Box::new(move || Message::BookDeleted(data::delete_book(&host, book_id)))
    }

    /// The keys for the focused field.
    pub fn bindings(&self) -> Vec<(&'static str, &'static str)> {
        let change = match self.focused() {
            Some(Field::Theme) => Some(("←→", "theme")),
            Some(Field::Cover) => Some(("enter", "choose a file")),
            Some(Field::Everyone) => Some(("←→", "change")),
            Some(Field::Member(_)) => Some(("←→", "change")),
            Some(Field::Save) => Some(("enter", if self.book_id.is_some() { "save" } else { "create" })),
            Some(Field::Delete) => Some(("enter", "delete book")),
            _ => None,
        };

        let mut bindings = vec![("↑↓", "move")];
        bindings.extend(change);

        if self.focused() == Some(Field::Cover) {
            bindings.push(("x", "remove cover"));
        }

        bindings.extend([("esc", "cancel"), ("ctrl+s", "save")]);
        bindings
    }

    /// The file picked as the new cover.
    pub fn use_cover(&mut self, path: std::path::PathBuf) {
        if let Loading::Ready(form) = &mut self.form {
            form.new_cover = Some(path.to_string_lossy().into_owned());
            form.remove_cover = false;
        }
    }

    pub fn fields(&self) -> Vec<Field> {
        let Some(form) = self.form.ready() else { return Vec::new() };
        let mut fields = vec![Field::Cover, Field::Title, Field::Subtitle, Field::Author, Field::Theme, Field::Everyone];
        fields.extend((0..form.members.len()).map(Field::Member));
        fields.push(Field::Save);

        if form.id.is_some() {
            fields.push(Field::Delete);
        }

        fields
    }

    pub fn focused(&self) -> Option<Field> {
        self.fields().get(self.focus).copied()
    }

    /// Whether keys go into text, so ? and the like are just characters.
    pub fn typing(&self) -> bool {
        matches!(self.focused(), Some(Field::Title | Field::Subtitle | Field::Author))
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        if self.saving {
            return Action::Nothing;
        }

        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::Save;
        }

        let fields = self.fields();
        let Some(field) = fields.get(self.focus).copied() else {
            return if key.code == KeyCode::Esc { Action::Cancel } else { Action::Nothing };
        };

        match key.code {
            KeyCode::Esc => return Action::Cancel,
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % fields.len(),
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + fields.len() - 1) % fields.len(),
            KeyCode::Enter if field == Field::Save => return Action::Save,
            KeyCode::Enter if field == Field::Delete => return self.ask_to_delete(),
            KeyCode::Enter if field == Field::Cover => return Action::ChooseCover,
            _ => {
                let Loading::Ready(form) = &mut self.form else { return Action::Nothing };
                change(form, field, key);
            }
        }

        Action::Nothing
    }

    fn ask_to_delete(&self) -> Action {
        match (self.book_id, self.form.ready()) {
            (Some(book_id), Some(form)) => Action::Delete(book_id, form.title.clone()),
            _ => Action::Nothing,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, images: &mut Images) {
        let title = self.form.ready().map(|it| it.title.clone()).filter(|it| !it.is_empty() && self.book_id.is_some());
        let last = if self.book_id.is_some() { "Edit" } else { "New book" };
        let mut steps = vec![self.name.as_str(), "Books"];
        steps.extend(title.as_deref());
        steps.push(last);
        let area = views::breadcrumbs(frame, area, &steps);

        let form = match &self.form {
            Loading::Ready(form) => form,
            Loading::Pending => return views::placeholder(frame, area, "Loading…"),
            Loading::Failed(error) => return views::failure(frame, area, error),
        };

        let focused = self.focused();
        let area = centered_column(area, 76);
        let access_height = form.members.len() as u16 + 3;
        let [top, _, access, _, buttons] = Layout::vertical([
            Constraint::Length(13),
            Constraint::Length(1),
            Constraint::Max(access_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let [cover, _, fields] = Layout::horizontal([Constraint::Length(20), Constraint::Length(2), Constraint::Fill(1)]).areas(top);

        draw_cover(frame, cover, &self.host, form, focused == Some(Field::Cover), images);

        let [title, subtitle, author, theme] = Layout::vertical([Constraint::Length(3); 4]).areas(fields);
        input(frame, title, "Title", &form.title, focused == Some(Field::Title), true);
        input(frame, subtitle, "Subtitle", &form.subtitle, focused == Some(Field::Subtitle), false);
        input(frame, author, "Author", &form.author, focused == Some(Field::Author), false);
        draw_themes(frame, theme, &form.theme, focused == Some(Field::Theme));

        draw_access(frame, access, form, focused);
        draw_buttons(frame, buttons, form.id.is_some(), focused, self.saving);
    }
}

/// What ←, → or a typed character does to the focused field.
fn change(form: &mut BookForm, field: Field, key: KeyEvent) {
    let step = match key.code {
        KeyCode::Left => -1,
        KeyCode::Right => 1,
        _ => 0,
    };

    match field {
        Field::Theme if step != 0 => {
            let index = THEMES.iter().position(|it| *it == form.theme).unwrap_or(1) as i32;
            form.theme = THEMES[(index + step).rem_euclid(THEMES.len() as i32) as usize].to_string();
        }
        Field::Cover if matches!(key.code, KeyCode::Char('x') | KeyCode::Backspace | KeyCode::Delete) => {
            form.new_cover = None;
            form.remove_cover = form.cover.is_some();
        }
        Field::Title => type_into(&mut form.title, key),
        Field::Subtitle => type_into(&mut form.subtitle, key),
        Field::Author => type_into(&mut form.author, key),
        Field::Everyone if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) || step != 0 => form.everyone = !form.everyone,
        Field::Member(index) if step != 0 => {
            let everyone = form.everyone;
            let member = &mut form.members[index];

            if !member.fixed() {
                member.role = next_role(member.role, step, everyone);
            }
        }
        _ => {}
    }
}

fn type_into(text: &mut String, key: KeyEvent) {
    match key.code {
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char(character) if views::plain(key) => text.push(character),
        _ => {}
    }
}

/// No access, reader, editor, in that order. When everyone can read, the
/// choice is only whether someone can edit too.
fn next_role(role: Role, step: i32, everyone: bool) -> Role {
    let roles: &[Role] = if everyone { &[Role::Reader, Role::Editor] } else { &[Role::None, Role::Reader, Role::Editor] };
    let index = roles.iter().position(|it| *it == role).unwrap_or(0) as i32;
    roles[(index + step).clamp(0, roles.len() as i32 - 1) as usize]
}

/// The theme colors in a row, framed like the fields above, the book's
/// own ringed.
fn draw_themes(frame: &mut Frame, area: Rect, current: &str, focused: bool) {
    let mut spans = Vec::new();

    for name in THEMES {
        let swatch = if name == current { "◉" } else { "●" };
        spans.push(Span::from(swatch).fg(theme::color(Some(name))));
        spans.push(Span::from("  "));
    }

    let border = if focused { Color::Blue } else { Color::DarkGray };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::from(" Theme ").dim())
        .padding(ratatui::widgets::Padding::horizontal(1));

    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn draw_cover(frame: &mut Frame, area: Rect, host: &str, form: &BookForm, focused: bool, images: &mut Images) {
    let [picture, caption] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let border = if focused { Color::Blue } else { Color::DarkGray };
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(border));
    let inside = block.inner(picture);
    frame.render_widget(block, picture);

    let shown = match &form.new_cover {
        Some(path) => Some(Source::file(std::path::Path::new(path))),
        None => form.cover.as_deref().filter(|_| !form.remove_cover).map(|it| Source::remote(host, it)),
    };
    let drawn = match shown {
        Some(source) => images.draw(frame, inside, &source),
        None => false,
    };

    if !drawn {
        views::themed_cover(frame, inside, if form.title.is_empty() { "Untitled" } else { &form.title }, Some(&form.theme));
    }

    let note = match &form.new_cover {
        Some(path) => Line::from(format!("New: {}", file_name(path))).dim(),
        None if form.remove_cover => Line::from("Cover removed").dim(),
        None => Line::from(""),
    };
    frame.render_widget(Paragraph::new(note).alignment(Alignment::Center), caption);
}

fn input(frame: &mut Frame, area: Rect, label: &str, value: &str, focused: bool, heading: bool) {
    let text = views::typed(label, value, focused);
    views::input(frame, area, label, if heading { text.bold() } else { text }, focused);
}

fn draw_access(frame: &mut Frame, area: Rect, form: &BookForm, focused: Option<Field>) {
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(Color::DarkGray)).title(" Access ".dim());
    let inside = block.inner(area).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
    frame.render_widget(block, area);

    let mut rows = vec![row(inside.width, "Everyone", if form.everyone { "on" } else { "off" }, focused == Some(Field::Everyone), true)];

    for (index, member) in form.members.iter().enumerate() {
        let role = match member.role {
            Role::Editor => "can edit",
            Role::Reader => "can read",
            Role::None if form.everyone => "can read",
            Role::None => "no access",
        };
        let name = if member.can_administer { format!("♛ {}", member.name) } else { member.name.clone() };
        rows.push(row(inside.width, &name, role, focused == Some(Field::Member(index)), !member.fixed()));
    }

    let scroll = match focused {
        Some(Field::Member(index)) => (index + 1).saturating_sub(usize::from(inside.height).saturating_sub(1)) as u16,
        _ => 0,
    };
    frame.render_widget(Paragraph::new(rows).scroll((scroll, 0)), inside);
}

/// A name, a dotted line, and its setting at the right, which ←→ changes
/// when it can.
fn row(width: u16, name: &str, value: &str, focused: bool, changeable: bool) -> Line<'static> {
    let value = if changeable { format!("‹ {value} ›") } else { value.to_string() };
    let used = name.chars().count() + value.chars().count() + 4;
    let dots = ".".repeat(usize::from(width).saturating_sub(used));
    let marker = if focused { "› " } else { "  " };

    let name = Span::from(format!("{marker}{name}"));
    let value = Span::from(value);
    let (name, value) = if focused { (name.fg(Color::Blue).bold(), value.fg(Color::Blue).bold()) } else { (name, value.dim()) };

    Line::from(vec![name, Span::from(format!(" {dots} ")).fg(Color::DarkGray), value])
}

fn draw_buttons(frame: &mut Frame, area: Rect, existing: bool, focused: Option<Field>, saving: bool) {
    let button = |label: &str, field: Field, color: Color| {
        if focused == Some(field) {
            Span::styled(format!(" {label} "), Style::default().fg(Color::Black).bg(color).bold())
        } else {
            Span::from(format!(" {label} ")).fg(color)
        }
    };

    let save = if saving { Span::from(" Saving… ").dim() } else { button(if existing { "Save" } else { "Create book" }, Field::Save, Color::Blue) };
    let mut spans = vec![save];

    if existing {
        spans.push(Span::from("   "));
        spans.push(button("Delete book", Field::Delete, Color::Red));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).alignment(Alignment::Center).wrap(Wrap { trim: false }), area);
}

fn centered_column(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width);
    Rect { x: area.x + (area.width - width) / 2, width, ..area }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|it| it.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::Member;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn member(id: u64, role: Role, can_administer: bool) -> Member {
        Member { id, name: format!("Person {id}"), role, can_administer, current: false }
    }

    fn page(book_id: Option<u64>) -> EditPage {
        let mut page = EditPage::new("https://books.example.com".to_string(), "books.example.com".to_string(), book_id);
        page.form = Loading::Ready(BookForm {
            id: book_id,
            title: "Handbook".to_string(),
            subtitle: String::new(),
            author: String::new(),
            theme: "white".to_string(),
            everyone: false,
            cover: Some("/covers/1".to_string()),
            new_cover: None,
            remove_cover: false,
            members: vec![member(1, Role::None, false), member(2, Role::Editor, true)],
        });
        page
    }

    fn form(page: &EditPage) -> &BookForm {
        page.form.ready().unwrap()
    }

    #[test]
    fn starts_in_the_title_and_types_into_it() {
        let mut page = page(Some(4));

        assert_eq!(page.focused(), Some(Field::Title));
        assert!(page.typing());
        page.key(press(KeyCode::Char('!')));
        page.key(press(KeyCode::Backspace));
        page.key(press(KeyCode::Char('s')));
        assert_eq!(form(&page).title, "Handbooks");
    }

    #[test]
    fn cycles_the_theme_round_with_the_arrows() {
        let mut page = page(Some(4));
        page.key(press(KeyCode::Down));
        page.key(press(KeyCode::Down));
        page.key(press(KeyCode::Down));

        assert_eq!(page.focused(), Some(Field::Theme), "the theme comes after the author");
        page.key(press(KeyCode::Right));
        assert_eq!(form(&page).theme, "black");
        page.key(press(KeyCode::Left));
        page.key(press(KeyCode::Left));
        assert_eq!(form(&page).theme, "violet");
    }

    #[test]
    fn changes_who_can_read_and_edit_but_not_administrators() {
        let mut page = page(Some(4));
        let fields = page.fields();
        page.focus = fields.iter().position(|it| *it == Field::Member(0)).unwrap();

        page.key(press(KeyCode::Right));
        assert_eq!(form(&page).members[0].role, Role::Reader);
        page.key(press(KeyCode::Right));
        page.key(press(KeyCode::Right));
        assert_eq!(form(&page).members[0].role, Role::Editor, "editor is as far as it goes");

        page.key(press(KeyCode::Down));
        page.key(press(KeyCode::Left));
        assert_eq!(form(&page).members[1].role, Role::Editor);
    }

    #[test]
    fn everyone_reading_leaves_only_whether_someone_can_edit() {
        assert_eq!(next_role(Role::Reader, -1, true), Role::Reader);
        assert_eq!(next_role(Role::None, 1, true), Role::Editor);
        assert_eq!(next_role(Role::Reader, -1, false), Role::None);
    }

    #[test]
    fn picks_a_new_cover_and_removes_the_old_one() {
        let mut page = page(Some(4));
        page.key(press(KeyCode::Up));
        assert_eq!(page.focused(), Some(Field::Cover));

        page.key(press(KeyCode::Char('x')));
        assert!(form(&page).remove_cover);

        assert!(matches!(page.key(press(KeyCode::Enter)), Action::ChooseCover));
        page.use_cover("/home/someone/cover.png".into());
        assert_eq!(form(&page).new_cover.as_deref(), Some("/home/someone/cover.png"));
        assert!(!form(&page).remove_cover);
    }

    #[test]
    fn only_a_book_that_exists_can_be_deleted() {
        assert_eq!(page(Some(4)).fields().last(), Some(&Field::Delete));
        assert_eq!(page(None).fields().last(), Some(&Field::Save));

        let mut page = page(Some(4));
        page.focus = page.fields().len() - 1;
        assert!(matches!(page.key(press(KeyCode::Enter)), Action::Delete(4, title) if title == "Handbook"));
    }

    #[test]
    fn saves_with_ctrl_s_anywhere_and_esc_cancels() {
        let mut page = page(Some(4));

        assert!(matches!(page.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)), Action::Save));
        assert!(matches!(page.key(press(KeyCode::Esc)), Action::Cancel));
    }
}
