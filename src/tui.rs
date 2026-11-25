//! `writebook tui`: pick an instance, browse its books, and arrange a book's
//! pages.

mod add;
mod book;
mod books;
mod clipboard;
mod confirm;
mod data;
mod edit;
mod editor;
mod files;
mod help;
mod images;
mod instances;
mod markdown;
mod overlays;
mod profile;
mod publishing;
mod reader;
mod screens;
mod search;
mod settings;
mod starfield;
mod theme;
mod views;

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use add::Add;
use book::BookPage;
use books::Books;
use confirm::{Confirm, Question};
use data::{BookCard, BookForm, Instance, Publication};
use edit::EditPage;
use files::FilePicker;
use help::Binding;
use images::Images;
use instances::InstancePicker;
use profile::ProfilePage;
use publishing::Publishing;
use reader::{Index, LeafPage};
use search::Search;
use settings::SettingsPage;
use starfield::Starfield;
use views::View;

pub fn run() -> Result<()> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        anyhow::bail!("writebook tui needs a terminal");
    }

    let mut session = Session { enhanced: false };
    let mut terminal = ratatui::try_init()?;
    let images = Images::detect();
    crossterm::execute!(std::io::stdout(), EnableBracketedPaste)?;

    // Without it a terminal sends ctrl+i as tab, so both open the instances.
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        crossterm::execute!(std::io::stdout(), PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
        session.enhanced = true;
    }

    leave_on_panic(session.enhanced);
    App::new(images).run(&mut terminal)
}

/// The terminal as the TUI set it up, put back however the TUI ends.
struct Session {
    /// Whether the keyboard enhancement flags were pushed, to pop them.
    enhanced: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        leave(self.enhanced);
    }
}

/// Put the terminal back, keyboard flags and pasting too, before the panic's
/// message is printed. ratatui's own hook only restores what it set up.
fn leave_on_panic(enhanced: bool) {
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        leave(enhanced);
        previous(info);
    }));
}

fn leave(enhanced: bool) {
    if enhanced {
        let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }

    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}

pub enum Message {
    Instances(Result<Vec<Instance>>),
    Image(images::Source, Result<image::DynamicImage>),
    /// A reply for the screen or overlay with this id, wherever it is by now.
    To(Id, Reply),
}

/// What a request for a screen or an overlay brought back.
pub enum Reply {
    Books(Result<Vec<BookCard>>),
    Book(Result<data::Book>),
    Changed(Result<()>),
    /// For the publishing overlay, and the book page with this id under it.
    Published(Id, Result<Publication>),
    Form(Result<BookForm>),
    Saved(Result<u64>),
    BookDeleted(Result<()>),
    /// What a search for a query found.
    Found(String, Result<Vec<data::Found>>),
    LeafLoaded(Result<data::LeafContent>),
    LeafSaved(Result<String>),
    /// The Markdown for a file uploaded into the page.
    Uploaded(Result<String>),
    People(Result<data::People>),
    PeopleChanged(Result<()>),
    /// The new invite link from the reset with this number.
    InviteReset(u64, Result<String>),
    ProfileLoaded(Result<data::Profile>),
    ProfileSaved(Result<()>),
}

/// Which screen or overlay a reply is for. Each one gets its own when it's
/// made, so a reply never lands on another one that took its place.
pub type Id = u64;

pub fn next_id() -> Id {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Work for a background thread, whose result comes back as a message.
pub type Request = Box<dyn FnOnce() -> Message + Send>;

pub enum Loading<T> {
    Pending,
    Ready(T),
    Failed(String),
}

impl<T> Loading<T> {
    fn from(result: Result<T>) -> Loading<T> {
        match result {
            Ok(value) => Loading::Ready(value),
            Err(error) => Loading::Failed(format!("{error:#}")),
        }
    }

    fn ready(&self) -> Option<&T> {
        match self {
            Loading::Ready(value) => Some(value),
            _ => None,
        }
    }
}

pub enum Screen {
    Instances(InstancePicker),
    Books(Books),
    Book(Box<BookPage>),
    /// A book's settings, or a new book's, over the screen to go back to.
    Edit(Box<EditPage>, Box<Screen>),
    /// One page of a book, over the book to go back to.
    Leaf(Box<LeafPage>, Box<BookPage>),
    /// The instance's people and invite link, over the books to go back to.
    Settings(Box<SettingsPage>, Books),
    /// Your own profile, over the settings to go back to.
    Profile(Box<ProfilePage>, Box<Screen>),
}

pub enum Overlay {
    Instances(InstancePicker),
    Add(Add),
    Publishing(Publishing),
    Files(Box<FilePicker>, Purpose),
    Search(Box<Search>),
    /// A book's pages, to jump to one from a page.
    Index(Index),
    Confirm(Confirm),
}

/// What a picked file is for, and what to go back to afterwards.
pub enum Purpose {
    Cover,
    Picture(Box<Add>),
    /// A file to upload into the page being edited.
    Upload,
}

impl Purpose {
    /// Where cancelling the picker goes: back to the form it was opened from.
    fn back(self) -> Option<Overlay> {
        match self {
            Purpose::Cover | Purpose::Upload => None,
            Purpose::Picture(add) => Some(Overlay::Add(*add)),
        }
    }
}

struct App {
    screen: Screen,
    overlay: Option<Overlay>,
    instances: Loading<Vec<Instance>>,
    /// Where the file picker opens: the folder of the last file picked.
    files_dir: PathBuf,
    starfield: Starfield,
    images: Images,
    help: bool,
    notice: Option<String>,
    books_view: View,
    book_view: View,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    done: bool,
}

impl App {
    fn new(images: Images) -> App {
        let (sender, receiver) = channel();
        let app = App {
            screen: Screen::Instances(InstancePicker::new(0)),
            overlay: None,
            instances: Loading::Pending,
            files_dir: std::env::current_dir().ok().or_else(dirs::home_dir).unwrap_or_default(),
            starfield: Starfield::new(),
            images,
            help: true,
            notice: None,
            books_view: View::Gallery,
            book_view: View::Gallery,
            sender,
            receiver,
            done: false,
        };

        app.spawn(|| Message::Instances(data::instances()));
        app
    }

    fn spawn(&self, work: impl FnOnce() -> Message + Send + 'static) {
        let sender = self.sender.clone();

        std::thread::spawn(move || {
            let _ = sender.send(work());
        });
    }

    fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.done {
            terminal.draw(|frame| self.draw(frame))?;

            if event::poll(Duration::from_millis(33))? {
                match event::read()? {
                    Event::Key(key) => self.key(key),
                    Event::Paste(text) => self.paste(&text),
                    _ => {}
                }
            }

            while let Ok(message) = self.receiver.try_recv() {
                self.receive(message);
            }

            if matches!(self.screen, Screen::Instances(_)) {
                self.starfield.step();
            }
        }

        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let toggle = help::toggle(self.typing());
        let room = frame.area().width.saturating_sub(toggle.width() as u16 + 3);
        let lines = if self.help { help::lines(&self.bindings(), self.notice.as_deref(), room) } else { help::lines(&[], self.notice.as_deref(), room) };
        let [main, bar] = Layout::vertical([Constraint::Fill(1), Constraint::Length(lines.len().max(1) as u16)]).areas(frame.area());

        self.draw_screen(frame, main);
        self.draw_overlay(frame, main);

        let last_line = Rect { y: bar.bottom().saturating_sub(1), height: 1, ..bar };
        frame.render_widget(Paragraph::new(lines), bar);
        frame.render_widget(Paragraph::new(toggle).alignment(Alignment::Right), last_line);
    }

    /// Whether keys go into a text field, where ? is just a character.
    fn typing(&self) -> bool {
        match &self.overlay {
            Some(overlay) => overlay.typing(),
            None => self.screen.typing(),
        }
    }

    fn bindings(&self) -> Vec<Binding> {
        match &self.overlay {
            Some(overlay) => overlay.bindings(),
            None => self.screen.bindings(self.books_view, self.book_view),
        }
    }

    fn key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        self.notice = None;

        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.ask_to_quit();
        } else if key.code == KeyCode::Char('?') && !self.typing() {
            self.help = !self.help;
        } else if let Some(overlay) = self.overlay.take() {
            self.overlay = self.overlay_key(overlay, key);
        } else if is_instances_shortcut(key)
            && let Screen::Leaf(page, _) = &self.screen
            && page.editor.is_none()
        {
            self.overlay = Some(Overlay::Index(Index::new(page)));
        } else if is_instances_shortcut(key) && self.screen.opens_instances(key) {
            self.overlay = Some(Overlay::Instances(InstancePicker::new(self.current_instance())));
        } else {
            self.screen_key(key);
        }
    }

    /// Pasted text goes into the page editor as it is, and into any other
    /// field as one line, as if typed.
    fn paste(&mut self, text: &str) {
        self.notice = None;

        if self.overlay.is_none()
            && let Screen::Leaf(page, _) = &mut self.screen
            && page.editor.is_some()
        {
            page.paste(text);
        } else if self.typing() {
            for character in views::one_line(text).chars() {
                let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);

                match self.overlay.take() {
                    Some(overlay) => self.overlay = self.overlay_key(overlay, key),
                    None => self.screen_key(key),
                }
            }
        }
    }

    /// Ctrl+C asks first, and a second one while asking quits.
    fn ask_to_quit(&mut self) {
        match self.overlay.take() {
            Some(Overlay::Confirm(Confirm { question: Question::Quit(_), .. })) => self.done = true,
            previous => self.overlay = Some(Overlay::Confirm(Confirm::new(Question::Quit(previous.map(Box::new))))),
        }
    }

    fn current_instance(&self) -> usize {
        match &self.screen {
            Screen::Instances(picker) => picker.selected,
            _ => self.instances.ready().and_then(|it| it.iter().position(|instance| Some(instance.url.as_str()) == self.screen.host())).unwrap_or(0),
        }
    }

    fn receive(&mut self, message: Message) {
        match message {
            Message::Instances(instances) => self.instances = Loading::from(instances),
            Message::Image(source, image) => self.images.arrived(source, image),
            Message::To(id, reply) => self.reply(id, reply),
        }
    }

    /// Hand a reply to the screen or overlay it's for, which may be set aside
    /// under another by now, or gone.
    fn reply(&mut self, id: Id, reply: Reply) {
        match reply {
            Reply::Books(books) => self.books_loaded(id, books),
            Reply::Book(book) => self.book_loaded(id, book),
            Reply::Changed(result) => self.book_changed(id, result),
            Reply::Published(book_page, result) => self.published(id, book_page, result),
            Reply::Form(form) => self.form_loaded(id, form),
            Reply::Saved(result) => self.saved(id, result),
            Reply::BookDeleted(result) => self.book_deleted(id, result),
            Reply::Found(query, found) => self.found(id, query, found),
            Reply::LeafLoaded(content) => self.leaf_loaded(id, content),
            Reply::LeafSaved(result) => self.leaf_saved(id, result),
            Reply::Uploaded(result) => self.uploaded(id, result),
            Reply::People(people) => self.people_loaded(id, people),
            Reply::PeopleChanged(result) => self.people_changed(id, result),
            Reply::InviteReset(reset, result) => self.invite_reset(id, reset, result),
            Reply::ProfileLoaded(profile) => self.profile_loaded(id, profile),
            Reply::ProfileSaved(result) => self.profile_saved(id, result),
        }
    }

    fn show_error(&mut self, error: anyhow::Error) {
        self.notice = Some(format!("{error:#}"));
    }

    /// Copy a link Writebook gave, once it's sure to be one to this instance.
    fn copy(&mut self, url: String) {
        let host = self.screen.host().unwrap_or_default();

        match clipboard::shareable(&url, host) {
            Ok(()) => {
                clipboard::copy(&url);
                self.notice = Some(format!("Copied {url}"));
            }
            Err(error) => self.show_error(error),
        }
    }
}

fn is_instances_shortcut(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab || (key.code == KeyCode::Char('i') && key.modifiers.contains(KeyModifiers::CONTROL))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_c() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    #[test]
    fn q_doesnt_quit_and_ctrl_c_asks_first() {
        let mut app = App::new(Images::none());

        app.key(press(KeyCode::Char('q')));
        assert!(!app.done);
        assert!(app.overlay.is_none());

        app.key(ctrl_c());
        assert!(matches!(app.overlay, Some(Overlay::Confirm(Confirm { question: Question::Quit(_), .. }))));
        assert!(!app.done);

        app.key(press(KeyCode::Esc));
        assert!(app.overlay.is_none());
        assert!(!app.done);

        app.key(ctrl_c());
        app.key(press(KeyCode::Char('y')));
        assert!(!app.done, "y is no answer; the choice is");

        app.key(press(KeyCode::Right));
        app.key(press(KeyCode::Enter));
        assert!(!app.done, "No was chosen");
        assert!(app.overlay.is_none());

        app.key(ctrl_c());
        app.key(press(KeyCode::Enter));
        assert!(app.done, "Yes is chosen to start with");
    }

    #[test]
    fn a_second_ctrl_c_quits_and_staying_brings_back_the_open_overlay() {
        let mut app = App::new(Images::none());
        app.overlay = Some(Overlay::Confirm(Confirm::new(Question::Discard)));

        app.key(ctrl_c());
        app.key(press(KeyCode::Esc));
        assert!(matches!(app.overlay, Some(Overlay::Confirm(Confirm { question: Question::Discard, .. }))));

        app.key(ctrl_c());
        app.key(ctrl_c());
        assert!(app.done);
    }

    const HOST: &str = "https://books.example.com";

    fn form(id: Option<u64>, title: &str) -> BookForm {
        BookForm {
            id,
            title: title.to_string(),
            subtitle: String::new(),
            author: String::new(),
            theme: "blue".to_string(),
            everyone: false,
            cover: None,
            new_cover: None,
            remove_cover: false,
            members: Vec::new(),
        }
    }

    fn leaf(id: u64) -> data::Leaf {
        data::Leaf { id, kind: "Page".to_string(), title: format!("Page {id}"), word_count: None, preview: None, image: None }
    }

    fn book() -> data::Book {
        data::Book {
            id: 1,
            title: "Handbook".to_string(),
            subtitle: None,
            author: None,
            theme: None,
            cover: None,
            slug: "handbook".to_string(),
            published: false,
            editable: true,
            leaves: vec![leaf(1), leaf(2)],
        }
    }

    fn books() -> Books {
        let mut books = Books::new(HOST.to_string(), "books.example.com".to_string());
        let card = data::BookCard { id: 1, title: "Handbook".to_string(), subtitle: None, author: None, theme: None, cover: None, editable: true };
        books.books = Loading::Ready(vec![card.clone(), card]);
        books
    }

    fn book_page() -> BookPage {
        let mut page = BookPage::new(books(), 1);
        page.book = Loading::Ready(book());
        page
    }

    fn reading_page(leaf_id: u64) -> LeafPage {
        let book = book();
        let index = book.leaves.iter().position(|it| it.id == leaf_id).unwrap();
        let mut page = LeafPage::new(HOST.to_string(), 1, &book, index);
        page.loaded(Ok(data::LeafContent {
            title: "Page".to_string(),
            body: "# Title\n\nSome text, a list:\n\n- one\n- > two\n\n![a](/a.png)\n\n| a | b |\n|---|---|\n| 1 | 2 |".to_string(),
            image: None,
            fingerprint: "f1".to_string(),
            record_gid: Some("gid://writebook/ActionText::RichText/1".to_string()),
        }));
        page
    }

    fn editing_page(leaf_id: u64) -> Screen {
        let mut page = reading_page(leaf_id);
        page.editor = Some(editor::Editor::new("Page", ""));
        Screen::Leaf(Box::new(page), Box::new(book_page()))
    }

    fn edited_body(app: &App) -> String {
        match &app.screen {
            Screen::Leaf(page, _) => page.editor.as_ref().unwrap().body(),
            _ => panic!("not on a page"),
        }
    }

    fn leaf_page(app: &mut App) -> &mut LeafPage {
        match &mut app.screen {
            Screen::Leaf(page, _) => page,
            _ => panic!("not on a page"),
        }
    }

    fn publication(published: bool) -> Publication {
        Publication { published, slug: "handbook".to_string(), url: format!("{HOST}/1/handbook") }
    }

    #[test]
    fn a_form_goes_only_into_the_editor_it_was_loaded_for() {
        let mut app = App::new(Images::none());
        let page = EditPage::new(HOST.to_string(), "books.example.com".to_string(), None);
        let id = page.id;
        app.screen = Screen::Edit(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));

        app.receive(Message::To(next_id(), Reply::Form(Ok(form(Some(9), "Someone else's")))));
        assert!(matches!(&app.screen, Screen::Edit(page, _) if matches!(page.form, Loading::Pending)));

        app.receive(Message::To(id, Reply::Form(Ok(form(None, "")))));
        assert!(matches!(&app.screen, Screen::Edit(page, _) if page.form.ready().is_some_and(|it| it.id.is_none())));
    }

    #[test]
    fn a_save_or_delete_finishing_leaves_another_editor_alone() {
        let mut app = App::new(Images::none());
        let mut page = EditPage::new(HOST.to_string(), "books.example.com".to_string(), Some(1));
        page.form = Loading::Ready(form(Some(1), "Handbook"));
        let id = page.id;
        app.screen = Screen::Edit(Box::new(page), Box::new(Screen::Book(Box::new(book_page()))));

        app.receive(Message::To(next_id(), Reply::Saved(Ok(7))));
        app.receive(Message::To(next_id(), Reply::BookDeleted(Ok(()))));
        assert!(matches!(&app.screen, Screen::Edit(page, _) if page.id == id));

        let Screen::Edit(page, _) = &mut app.screen else { panic!("the editor closed") };
        page.saving = true;
        app.receive(Message::To(id, Reply::Saved(Err(anyhow::anyhow!("offline")))));
        assert!(matches!(&app.screen, Screen::Edit(page, _) if !page.saving));
        assert_eq!(app.notice.as_deref(), Some("offline"));
    }

    #[test]
    fn a_page_loads_only_into_the_page_it_was_loaded_for() {
        let mut app = App::new(Images::none());
        app.screen = Screen::Leaf(Box::new(reading_page(1)), Box::new(book_page()));
        let id = leaf_page(&mut app).id;
        let content = |body: &str| data::LeafContent { title: "Page".to_string(), body: body.to_string(), image: None, fingerprint: "f2".to_string(), record_gid: None };

        app.receive(Message::To(next_id(), Reply::LeafLoaded(Ok(content("Elsewhere")))));
        assert_eq!(leaf_page(&mut app).content.ready().unwrap().fingerprint, "f1");

        app.receive(Message::To(id, Reply::LeafLoaded(Ok(content("Here")))));
        assert_eq!(leaf_page(&mut app).content.ready().unwrap().body, "Here");
    }

    #[test]
    fn an_upload_goes_only_into_the_page_it_was_for() {
        let mut app = App::new(Images::none());
        app.screen = editing_page(2);
        let id = leaf_page(&mut app).id;
        let _request = leaf_page(&mut app).upload(PathBuf::from("b.png")).unwrap();

        app.receive(Message::To(next_id(), Reply::Uploaded(Ok("![a.png](/a.png)".to_string()))));
        assert_eq!(edited_body(&app), "");
        assert!(app.notice.is_some());

        app.receive(Message::To(id, Reply::Uploaded(Ok("![b.png](/b.png)".to_string()))));
        assert!(edited_body(&app).contains("![b.png](/b.png)"));
    }

    #[test]
    fn a_page_waits_for_its_uploads_before_saving() {
        let mut app = App::new(Images::none());
        app.screen = editing_page(1);
        let id = leaf_page(&mut app).id;
        let _request = leaf_page(&mut app).upload(PathBuf::from("a.png")).unwrap();

        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!leaf_page(&mut app).saving);
        assert_eq!(app.notice.as_deref(), Some("Wait for the upload to finish, then save."));

        app.receive(Message::To(id, Reply::Uploaded(Ok("![a.png](/a.png)".to_string()))));
        assert!(edited_body(&app).contains("![a.png](/a.png)"));
    }

    #[test]
    fn pasting_while_a_page_saves_changes_nothing() {
        let mut app = App::new(Images::none());
        app.screen = editing_page(1);
        leaf_page(&mut app).saving = true;

        app.paste("too late");
        assert_eq!(edited_body(&app), "");
    }

    #[test]
    fn books_load_while_settings_are_open_over_them() {
        let mut app = App::new(Images::none());
        let mut books = Books::new(HOST.to_string(), "books.example.com".to_string());
        let id = books.id;
        books.books = Loading::Pending;
        app.screen = Screen::Settings(Box::new(SettingsPage::new(HOST.to_string())), books);

        app.receive(Message::To(id, Reply::Books(Ok(Vec::new()))));
        assert!(matches!(&app.screen, Screen::Settings(_, books) if matches!(books.books, Loading::Ready(_))));
    }

    #[test]
    fn publishing_goes_only_to_the_overlay_it_was_for_even_under_the_quit_question() {
        let mut app = App::new(Images::none());
        let page = book_page();
        let book_page_id = page.id;
        app.screen = Screen::Book(Box::new(page));
        let earlier = next_id();
        app.overlay = Some(Overlay::Publishing(Publishing::new(HOST, book_page_id, &book())));

        app.receive(Message::To(earlier, Reply::Published(next_id(), Ok(publication(true)))));
        let Some(Overlay::Publishing(publishing)) = &mut app.overlay else { panic!("the overlay closed") };
        assert!(matches!(publishing.key(press(KeyCode::Char('c'))), publishing::Action::Nothing), "still private");

        let Some(Overlay::Publishing(publishing)) = &mut app.overlay else { panic!("the overlay closed") };
        let id = publishing.id;
        assert!(matches!(publishing.key(press(KeyCode::Down)), publishing::Action::Request(_)));
        assert!(matches!(publishing.key(press(KeyCode::Down)), publishing::Action::Nothing), "saving");

        app.key(ctrl_c());
        app.receive(Message::To(id, Reply::Published(book_page_id, Ok(publication(true)))));
        app.key(press(KeyCode::Esc));

        let Some(Overlay::Publishing(publishing)) = &mut app.overlay else { panic!("the overlay is gone") };
        assert!(matches!(publishing.key(press(KeyCode::Char('c'))), publishing::Action::Copy(_)));
        assert!(matches!(&app.screen, Screen::Book(page) if page.book.ready().unwrap().published));
    }

    #[test]
    fn publishing_blocks_editing_the_address_while_saving() {
        let mut publishing = Publishing::new(HOST, next_id(), &book());

        assert!(matches!(publishing.key(press(KeyCode::Down)), publishing::Action::Request(_)));
        publishing.key(press(KeyCode::Char('e')));
        assert!(!publishing.typing());
    }

    #[test]
    fn search_results_arrive_under_the_quit_question() {
        let mut app = App::new(Images::none());
        let search = Search::new(None, "every book".to_string());
        let id = search.id;
        app.overlay = Some(Overlay::Search(Box::new(search)));
        app.paste("rabbit");
        app.key(press(KeyCode::Enter));

        app.key(ctrl_c());
        app.receive(Message::To(id, Reply::Found("rabbit".to_string(), Ok(Vec::new()))));
        app.key(press(KeyCode::Esc));

        assert!(matches!(&app.overlay, Some(Overlay::Search(search)) if matches!(search.results, Some(Loading::Ready(_)))));
    }

    #[test]
    fn only_the_last_invite_reset_shows_its_link() {
        let mut app = App::new(Images::none());
        let mut page = SettingsPage::new(HOST.to_string());
        page.people = Loading::Ready(data::People { join_url: Some(format!("{HOST}/join/old")), can_administer: true, people: Vec::new() });
        let id = page.id;
        let _first = page.reset_invite();
        let _second = page.reset_invite();
        let profile = ProfilePage::new(HOST.to_string());
        app.screen = Screen::Profile(Box::new(profile), Box::new(Screen::Settings(Box::new(page), books())));

        app.receive(Message::To(next_id(), Reply::InviteReset(2, Ok(format!("{HOST}/join/elsewhere")))));
        app.receive(Message::To(id, Reply::InviteReset(2, Ok(format!("{HOST}/join/second")))));
        app.receive(Message::To(id, Reply::InviteReset(1, Ok(format!("{HOST}/join/first")))));

        let Screen::Profile(_, back) = &app.screen else { panic!("the profile closed") };
        let Screen::Settings(page, _) = back.as_ref() else { panic!("no settings") };
        assert_eq!(page.people.ready().unwrap().join_url, Some(format!("{HOST}/join/second")));
    }

    #[test]
    fn a_profile_save_finishing_leaves_a_reopened_profile_alone() {
        let mut app = App::new(Images::none());
        let mut page = ProfilePage::new(HOST.to_string());
        page.loaded(Ok(data::Profile { id: 1, name: "Stanko".to_string(), email: "stanko@example.com".to_string() }));
        app.screen = Screen::Profile(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));
        app.key(press(KeyCode::Down));
        app.key(press(KeyCode::Down));
        app.paste("secret");

        app.receive(Message::To(next_id(), Reply::ProfileSaved(Ok(()))));
        assert!(matches!(&app.screen, Screen::Profile(page, _) if page.password() == "secret"));
        assert!(app.notice.is_none());
    }

    #[test]
    fn tab_moves_through_the_profile_and_only_ctrl_i_opens_the_instances() {
        let mut app = App::new(Images::none());
        let mut page = ProfilePage::new(HOST.to_string());
        page.loaded(Ok(data::Profile { id: 1, name: "Stanko".to_string(), email: "stanko@example.com".to_string() }));
        app.screen = Screen::Profile(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));

        app.key(press(KeyCode::Tab));
        app.key(press(KeyCode::Char('!')));
        assert!(app.overlay.is_none());
        assert!(matches!(&app.screen, Screen::Profile(page, _) if page.email() == "stanko@example.com!"));

        app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
        assert!(matches!(app.overlay, Some(Overlay::Instances(_))));
    }

    #[test]
    fn pastes_into_the_field_being_typed_in() {
        let mut app = App::new(Images::none());
        app.overlay = Some(Overlay::Search(Box::new(Search::new(None, "every book".to_string()))));

        app.paste("white\nrabbit?");
        let Some(Overlay::Search(search)) = &mut app.overlay else { panic!("the search closed") };
        assert!(matches!(search.key(press(KeyCode::Enter)), search::Action::Search(query) if query == "white rabbit?"));

        app.screen = editing_page(1);
        app.overlay = None;
        app.paste("- one\n- two");
        assert_eq!(edited_body(&app), "- one\n- two");
    }

    #[test]
    fn draws_every_screen_and_overlay_in_a_terminal_too_small_for_them() {
        let mut settings = SettingsPage::new(HOST.to_string());
        settings.people = Loading::Ready(data::People {
            join_url: Some(format!("{HOST}/join/abc")),
            can_administer: true,
            people: (1..30).map(|id| data::Person { id, name: format!("Person {id}"), role: "member".to_string(), current: id == 1 }).collect(),
        });
        let mut edit = EditPage::new(HOST.to_string(), "books.example.com".to_string(), Some(1));
        edit.form = Loading::Ready(form(Some(1), "Handbook"));
        let mut profile = ProfilePage::new(HOST.to_string());
        profile.loaded(Ok(data::Profile { id: 1, name: "Stanko".to_string(), email: "stanko@example.com".to_string() }));

        let screens: Vec<fn() -> Screen> = vec![
            || Screen::Instances(InstancePicker::new(0)),
            || Screen::Books(books()),
            || Screen::Book(Box::new(book_page())),
            || Screen::Leaf(Box::new(reading_page(1)), Box::new(book_page())),
            || editing_page(1),
        ];
        let mut apps: Vec<App> = Vec::new();

        for screen in screens {
            for view in [View::Gallery, View::List] {
                let mut app = App::new(Images::none());
                app.screen = screen();
                app.books_view = view;
                app.book_view = view;
                apps.push(app);
            }
        }

        for screen in [
            Screen::Edit(Box::new(edit), Box::new(Screen::Books(books()))),
            Screen::Settings(Box::new(settings), books()),
            Screen::Profile(Box::new(profile), Box::new(Screen::Books(books()))),
        ] {
            let mut app = App::new(Images::none());
            app.screen = screen;
            apps.push(app);
        }

        let overlays: Vec<fn(&App) -> Overlay> = vec![
            |_| Overlay::Instances(InstancePicker::new(0)),
            |_| Overlay::Add(Add::default()),
            |_| Overlay::Publishing(Publishing::new(HOST, 1, &book())),
            |_| Overlay::Search(Box::new(Search::new(None, "every book".to_string()))),
            |_| Overlay::Confirm(Confirm::new(Question::Quit(None))),
            |app| match &app.screen {
                Screen::Leaf(page, _) => Overlay::Index(Index::new(page)),
                _ => Overlay::Confirm(Confirm::new(Question::Discard)),
            },
            |_| Overlay::Files(Box::new(FilePicker::new("Choose a cover", files::IMAGES, std::env::temp_dir())), Purpose::Cover),
        ];

        for app in &mut apps {
            for overlay in std::iter::once(None).chain(overlays.iter().map(Some)) {
                app.overlay = overlay.map(|it| it(app));

                for (width, height) in [(0, 0), (1, 1), (2, 1), (4, 3), (8, 5), (12, 4), (20, 8), (30, 12)] {
                    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
                    terminal.draw(|frame| app.draw(frame)).unwrap();
                }
            }
        }
    }

    #[test]
    fn ctrl_i_and_tab_both_open_the_instances() {
        assert!(is_instances_shortcut(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)));
        assert!(is_instances_shortcut(press(KeyCode::Tab)));
        assert!(!is_instances_shortcut(press(KeyCode::Char('i'))));
    }
}
