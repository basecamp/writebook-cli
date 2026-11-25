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
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};
use std::path::PathBuf;
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

    let mut terminal = ratatui::try_init()?;
    let images = Images::detect();
    let enhanced = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);

    // Without it a terminal sends ctrl+i as tab, so both open the instances.
    if enhanced {
        crossterm::execute!(std::io::stdout(), PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
    }

    let result = App::new(images).run(&mut terminal);

    if enhanced {
        crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
    }
    ratatui::restore();
    result
}

pub enum Message {
    Instances(Result<Vec<Instance>>),
    Books(String, Result<Vec<BookCard>>),
    Book(String, u64, Result<data::Book>),
    Image(String, Result<image::DynamicImage>),
    Changed(Result<()>),
    Published(Result<Publication>),
    /// The form for the book with this id on this host, or for a new book.
    Form(String, Option<u64>, Result<BookForm>),
    Saved(Result<u64>),
    BookDeleted(Result<()>),
    /// What a search on a host, in a book or every book, found for a query.
    Found(String, Option<u64>, String, Result<Vec<data::Found>>),
    LeafLoaded(u64, u64, Result<data::LeafContent>),
    LeafSaved(Result<String>),
    /// The Markdown for a file uploaded into a page on a host and book.
    Uploaded(String, u64, u64, Result<String>),
    People(String, Result<data::People>),
    PeopleChanged(Result<()>),
    InviteReset(Result<String>),
    ProfileLoaded(String, Result<data::Profile>),
    ProfileSaved(Result<()>),
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
                if let Event::Key(key) = event::read()? {
                    self.key(key);
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
            Message::Image(path, image) => self.images.arrived(path, image),
            Message::Books(host, books) => self.books_loaded(host, books),
            Message::Book(host, book_id, book) => self.book_loaded(host, book_id, book),
            Message::Changed(result) => self.book_changed(result),
            Message::Published(result) => self.published(result),
            Message::Form(host, book_id, form) => self.form_loaded(host, book_id, form),
            Message::Saved(result) => self.saved(result),
            Message::BookDeleted(result) => self.book_deleted(result),
            Message::Found(host, book_id, query, found) => self.found(host, book_id, query, found),
            Message::LeafLoaded(book_id, leaf_id, content) => self.leaf_loaded(book_id, leaf_id, content),
            Message::LeafSaved(result) => self.leaf_saved(result),
            Message::Uploaded(host, book_id, leaf_id, result) => self.uploaded(host, book_id, leaf_id, result),
            Message::People(host, people) => self.people_loaded(host, people),
            Message::PeopleChanged(result) => self.people_changed(result),
            Message::InviteReset(result) => self.invite_reset(result),
            Message::ProfileLoaded(host, profile) => self.profile_loaded(host, profile),
            Message::ProfileSaved(result) => self.profile_saved(result),
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

    fn editing_page(leaf_id: u64) -> Screen {
        let books = Books::new(HOST.to_string(), "books.example.com".to_string());
        let book = data::Book {
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
        };
        let index = book.leaves.iter().position(|it| it.id == leaf_id).unwrap();
        let mut page = LeafPage::new(HOST.to_string(), 1, &book, index);
        page.editor = Some(editor::Editor::new("Page", ""));
        Screen::Leaf(Box::new(page), Box::new(BookPage::new(books, 1)))
    }

    fn edited_body(app: &App) -> String {
        match &app.screen {
            Screen::Leaf(page, _) => page.editor.as_ref().unwrap().body(),
            _ => panic!("not on a page"),
        }
    }

    #[test]
    fn a_form_for_another_book_doesnt_fill_in_the_new_book() {
        let mut app = App::new(Images::none());
        let page = EditPage::new(HOST.to_string(), "books.example.com".to_string(), None);
        app.screen = Screen::Edit(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));

        app.receive(Message::Form(HOST.to_string(), Some(9), Ok(form(Some(9), "Someone else's"))));
        app.receive(Message::Form("https://other.example.com".to_string(), None, Ok(form(None, "Elsewhere"))));
        assert!(matches!(&app.screen, Screen::Edit(page, _) if matches!(page.form, Loading::Pending)));

        app.receive(Message::Form(HOST.to_string(), None, Ok(form(None, ""))));
        assert!(matches!(&app.screen, Screen::Edit(page, _) if page.form.ready().is_some_and(|it| it.id.is_none())));
    }

    #[test]
    fn an_upload_goes_only_into_the_page_it_was_for() {
        let mut app = App::new(Images::none());
        app.screen = editing_page(2);

        app.receive(Message::Uploaded(HOST.to_string(), 1, 1, Ok("![a.png](/a.png)".to_string())));
        assert_eq!(edited_body(&app), "");
        assert!(app.notice.is_some());

        app.receive(Message::Uploaded(HOST.to_string(), 1, 2, Ok("![b.png](/b.png)".to_string())));
        assert!(edited_body(&app).contains("![b.png](/b.png)"));
    }

    #[test]
    fn ctrl_i_and_tab_both_open_the_instances() {
        assert!(is_instances_shortcut(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)));
        assert!(is_instances_shortcut(press(KeyCode::Tab)));
        assert!(!is_instances_shortcut(press(KeyCode::Char('i'))));
    }
}
