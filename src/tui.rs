//! `writebook tui`: pick an instance, browse its books, and arrange a book's
//! pages.

mod add;
mod book;
mod book_form;
mod books;
mod clipboard;
mod confirm;
mod data;
mod editor;
mod files;
mod help;
mod images;
mod index;
mod instances;
mod leaf;
mod markdown;
mod overlays;
mod profile;
mod publishing;
mod screens;
mod search;
mod settings;
mod starfield;
mod theme;
mod views;

#[cfg(test)]
mod flows;

use anyhow::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
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
use book_form::BookFormPage;
use books::Books;
use confirm::{Confirm, Question};
use data::{BookForm, Instance, Publication};
use files::FilePicker;
use help::Binding;
use images::Images;
use index::Index;
use instances::InstancePicker;
use leaf::LeafPage;
use profile::ProfilePage;
use publishing::Publishing;
use search::Search;
use settings::SettingsPage;
use starfield::Starfield;
use views::View;

pub fn run() -> Result<()> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        anyhow::bail!("writebook tui needs a terminal");
    }

    let mut session = Session {
        keyboard_enhanced: false,
    };
    let mut terminal = ratatui::try_init()?;
    let images = Images::detect();
    crossterm::execute!(std::io::stdout(), EnableBracketedPaste)?;

    // Without it a terminal sends ctrl+i as tab, so both open the instances.
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        crossterm::execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        session.keyboard_enhanced = true;
    }

    restore_terminal_on_panic(session.keyboard_enhanced);

    let app = App::new(images);
    app.spawn(|| Message::Instances(data::instances()));
    app.run(&mut terminal)
}

struct Session {
    keyboard_enhanced: bool,
}

impl Drop for Session {
    fn drop(&mut self) {
        restore_terminal(self.keyboard_enhanced);
    }
}

fn restore_terminal_on_panic(keyboard_enhanced: bool) {
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        restore_terminal(keyboard_enhanced);
        previous(info);
    }));
}

fn restore_terminal(keyboard_enhanced: bool) {
    if keyboard_enhanced {
        report_restore_failure(crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags));
    }

    report_restore_failure(crossterm::execute!(std::io::stdout(), DisableBracketedPaste));
    ratatui::restore();
}

fn report_restore_failure(result: std::io::Result<()>) {
    if let Err(error) = result {
        eprintln!("could not restore the terminal: {error}");
    }
}

pub enum Message {
    Instances(Result<Vec<Instance>>),
    Image(images::Source, Result<image::DynamicImage>),
    /// A reply for the screen or overlay with this id, wherever it is by now.
    To(Id, Reply),
}

/// What a request for a screen or an overlay brought back.
pub enum Reply {
    BooksLoaded(Result<data::Library>),
    BookLoaded(Result<data::Book>),
    LeavesChanged(Result<()>),
    /// For the publishing overlay, and the book page with this id under it.
    Published(Id, Result<Publication>),
    BookFormLoaded(Result<BookForm>),
    BookSaved(Result<u64>),
    BookDeleted(Result<()>),
    /// What a search for a query found.
    Found(String, Result<Vec<data::Found>>),
    LeafLoaded(Result<data::LeafContent>),
    LeafSaved(Result<String>),
    /// The Markdown for a file uploaded into the page.
    Uploaded(Result<String>),
    PeopleLoaded(Result<data::People>),
    PeopleChanged(Result<()>),
    /// The new invite link from the reset with this number.
    InviteReset(u64, Result<String>),
    ProfileLoaded(Result<data::Profile>),
    ProfileSaved(Result<()>),
}

/// Which screen or overlay a reply is for. Each one gets its own when it's
/// made, so a reply doesn't land on another one that took its place.
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

    fn count(&self, count: impl FnOnce(&T) -> usize) -> usize {
        match self {
            Loading::Ready(value) => count(value),
            _ => 0,
        }
    }
}

pub enum Screen {
    Instances(InstancePicker),
    Books(Books),
    Book(Box<BookPage>),
    /// A book's settings, or a new book's, over the screen to go back to.
    BookForm(Box<BookFormPage>, Box<Screen>),
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
    fn previous_overlay(self) -> Option<Overlay> {
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
    files_dir: PathBuf,
    starfield: Starfield,
    images: Images,
    showing_help: bool,
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

        App {
            screen: Screen::Instances(InstancePicker::new(0)),
            overlay: None,
            instances: Loading::Pending,
            files_dir: std::env::current_dir().ok().or_else(dirs::home_dir).unwrap_or_default(),
            starfield: Starfield::new(),
            images,
            showing_help: true,
            notice: None,
            books_view: View::Gallery,
            book_view: View::Gallery,
            sender,
            receiver,
            done: false,
        }
    }

    fn spawn(&self, work: impl FnOnce() -> Message + Send + 'static) {
        let sender = self.sender.clone();

        std::thread::spawn(move || {
            if let Err(unwanted) = sender.send(work()) {
                drop(unwanted);
            }
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
        let lines = if self.showing_help {
            help::lines(&self.bindings(), self.notice.as_deref(), room)
        } else {
            help::lines(&[], self.notice.as_deref(), room)
        };
        let [main, bar] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(lines.len().max(1) as u16)]).areas(frame.area());

        self.draw_screen(frame, main);
        self.draw_overlay(frame, main);

        let last_line = Rect {
            y: bar.bottom().saturating_sub(1),
            height: 1,
            ..bar
        };
        frame.render_widget(Paragraph::new(lines), bar);
        frame.render_widget(Paragraph::new(toggle).alignment(Alignment::Right), last_line);
    }

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
            self.showing_help = !self.showing_help;
        } else if let Some(overlay) = self.overlay.take() {
            self.overlay = self.overlay_key(overlay, key);
        } else if is_tab_or_ctrl_i(key)
            && let Screen::Leaf(page, _) = &self.screen
            && page.editor.is_none()
        {
            self.overlay = Some(Overlay::Index(Index::new(page)));
        } else if is_tab_or_ctrl_i(key) && self.screen.opens_instances(key) {
            self.overlay = Some(Overlay::Instances(InstancePicker::new(self.current_instance_index())));
        } else {
            self.screen_key(key);
        }
    }

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

    fn ask_to_quit(&mut self) {
        match self.overlay.take() {
            Some(Overlay::Confirm(Confirm {
                question: Question::Quit(_),
                ..
            })) => self.done = true,
            previous => self.overlay = Some(Overlay::Confirm(Confirm::new(Question::Quit(previous.map(Box::new))))),
        }
    }

    fn current_instance_index(&self) -> usize {
        match &self.screen {
            Screen::Instances(picker) => picker.selected,
            _ => self
                .instances
                .ready()
                .and_then(|it| {
                    it.iter()
                        .position(|instance| Some(instance.url.as_str()) == self.screen.host())
                })
                .unwrap_or(0),
        }
    }

    fn receive(&mut self, message: Message) {
        match message {
            Message::Instances(instances) => self.instances = Loading::from(instances),
            Message::Image(source, image) => self.images.arrived(source, image),
            Message::To(id, reply) => self.reply(id, reply),
        }
    }

    fn reply(&mut self, id: Id, reply: Reply) {
        match reply {
            Reply::BooksLoaded(books) => self.books_loaded(id, books),
            Reply::BookLoaded(book) => self.book_loaded(id, book),
            Reply::LeavesChanged(result) => self.leaves_changed(id, result),
            Reply::Published(book_page_id, result) => self.published(id, book_page_id, result),
            Reply::BookFormLoaded(form) => self.book_form_loaded(id, form),
            Reply::BookSaved(result) => self.book_saved(id, result),
            Reply::BookDeleted(result) => self.book_deleted(id, result),
            Reply::Found(query, found) => self.found(id, query, found),
            Reply::LeafLoaded(content) => self.leaf_loaded(id, content),
            Reply::LeafSaved(result) => self.leaf_saved(id, result),
            Reply::Uploaded(result) => self.uploaded(id, result),
            Reply::PeopleLoaded(people) => self.people_loaded(id, people),
            Reply::PeopleChanged(result) => self.people_changed(id, result),
            Reply::InviteReset(reset, result) => self.invite_reset(id, reset, result),
            Reply::ProfileLoaded(profile) => self.profile_loaded(id, profile),
            Reply::ProfileSaved(result) => self.profile_saved(id, result),
        }
    }

    fn show_error(&mut self, error: anyhow::Error) {
        self.notice = Some(format!("{error:#}"));
    }

    fn copy_link(&mut self, url: String) {
        let host = self.screen.host().unwrap_or_default();

        let copied = clipboard::shareable(&url, host).and_then(|()| Ok(clipboard::copy(&url, &mut std::io::stdout())?));

        match copied {
            Ok(()) => self.notice = Some(format!("Copied {url}")),
            Err(error) => self.show_error(error),
        }
    }
}

fn is_tab_or_ctrl_i(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab || (key.code == KeyCode::Char('i') && key.modifiers.contains(KeyModifiers::CONTROL))
}
