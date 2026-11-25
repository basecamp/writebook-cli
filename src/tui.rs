//! `writebook tui`: pick an instance, browse its books, and arrange a book's
//! pages.

mod data;
mod edit;
mod editor;
mod files;
mod search;
mod help;
mod images;
mod markdown;
mod reader;
mod starfield;
mod theme;
mod views;

use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};
use std::io::Write;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use data::{Book, BookCard, BookForm, Instance, Leaf, NewLeaf, Publication};
use edit::EditPage;
use files::{FilePicker, Picked};
use reader::LeafPage;
use search::Search;
use help::Binding;
use images::Images;
use starfield::Starfield;

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
    Book(String, u64, Result<Book>),
    Image(String, Result<image::DynamicImage>),
    Changed(Result<()>),
    Published(Result<Publication>),
    Form(Result<BookForm>),
    Saved(Result<u64>),
    BookDeleted(Result<()>),
    Found(Result<Vec<data::Found>>),
    LeafLoaded(u64, u64, Result<data::LeafContent>),
    LeafSaved(Result<String>),
    Uploaded(Result<String>),
}

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

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Gallery,
    List,
}

impl View {
    fn toggled(self) -> View {
        match self {
            View::Gallery => View::List,
            View::List => View::Gallery,
        }
    }
}

enum Screen {
    Instances { selected: usize },
    Books(Books),
    Book(Box<BookPage>),
    /// A book's settings, or a new book's, over the screen to go back to.
    Edit(Box<EditPage>, Box<Screen>),
    /// One page of a book, over the book to go back to.
    Leaf(Box<LeafPage>, Box<BookPage>),
}

pub struct Books {
    host: String,
    name: String,
    books: Loading<Vec<BookCard>>,
    selected: usize,
    columns: usize,
    offset: usize,
}

pub struct BookPage {
    host: String,
    book_id: u64,
    book: Loading<Book>,
    selected: usize,
    columns: usize,
    offset: usize,
    moving: Option<Moving>,
    /// The page to select once the book arrives, like one a search found.
    pending_leaf: Option<u64>,
    books: Books,
}

pub struct Moving {
    order: Vec<Leaf>,
}

enum Overlay {
    Instances { selected: usize },
    Add(Add),
    Delete(Confirm),
    DeleteBook(Confirm),
    Publishing(Publishing),
    Files(Box<FilePicker>, Purpose),
    Search(Box<Search>),
    /// A book's pages, to jump to one from a page.
    Index { selected: usize },
    /// Asking whether to throw away the changes in the page editor.
    Discard(Confirm),
    /// Asking whether to quit, over whatever overlay was open.
    Quit { previous: Option<Box<Overlay>>, confirm: Confirm },
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

/// A yes or no question, answered by choosing one and pressing enter.
#[derive(Clone, Copy)]
pub struct Confirm {
    yes: bool,
}

impl Confirm {
    /// Deleting starts on No, so enter alone never deletes anything.
    fn no() -> Confirm {
        Confirm { yes: false }
    }

    fn yes() -> Confirm {
        Confirm { yes: true }
    }

    /// The answer once there is one: enter takes the chosen one, esc is no.
    fn answer(&mut self, key: KeyEvent) -> Option<bool> {
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
}

pub struct Add {
    choice: usize,
    form: Option<Form>,
}

pub struct Form {
    kind: Kind,
    fields: Vec<Field>,
    focus: usize,
}

#[derive(Clone, Copy)]
pub enum Kind {
    Page,
    Picture,
    Section,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Page, Kind::Picture, Kind::Section];

    fn name(self) -> &'static str {
        match self {
            Kind::Page => "page",
            Kind::Picture => "picture",
            Kind::Section => "section",
        }
    }

    fn fields(self) -> Vec<Field> {
        match self {
            Kind::Picture => vec![Field::new("Caption"), Field::file("Image")],
            _ => vec![Field::new("Title")],
        }
    }
}

impl Form {
    /// Every field, then the Add and Cancel buttons.
    fn stops(&self) -> usize {
        self.fields.len() + 2
    }

    fn add_button(&self) -> usize {
        self.fields.len()
    }

    fn cancel_button(&self) -> usize {
        self.fields.len() + 1
    }

    fn field(&self) -> Option<&Field> {
        self.fields.get(self.focus)
    }

    /// The focused field when it takes typing, not a file or a button.
    fn typed_field(&mut self) -> Option<&mut Field> {
        self.fields.get_mut(self.focus).filter(|it| !it.file)
    }
}

pub struct Field {
    label: &'static str,
    value: String,
    /// Filled in with the file picker rather than typed.
    file: bool,
}

impl Field {
    fn new(label: &'static str) -> Field {
        Field { label, value: String::new(), file: false }
    }

    fn file(label: &'static str) -> Field {
        Field { label, value: String::new(), file: true }
    }
}

pub struct Publishing {
    published: bool,
    slug: String,
    url: String,
    editing: Option<String>,
    qr: bool,
    saving: bool,
}

impl Publishing {
    fn url_without_slug(&self) -> String {
        match self.url.rsplit_once('/') {
            Some((start, _)) => format!("{start}/"),
            None => String::new(),
        }
    }
}

struct App {
    screen: Screen,
    overlay: Option<Overlay>,
    instances: Loading<Vec<Instance>>,
    /// Where the file picker opens: the folder of the last file picked.
    files_dir: std::path::PathBuf,
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
            screen: Screen::Instances { selected: 0 },
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

        app.load_instances();
        app
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

            if matches!(self.screen, Screen::Instances { .. }) {
                self.starfield.step();
            }
        }

        Ok(())
    }

    fn load_instances(&self) {
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Message::Instances(data::instances()));
        });
    }

    fn receive(&mut self, message: Message) {
        match message {
            Message::Instances(instances) => self.instances = Loading::from(instances),
            Message::Books(host, books) => {
                if let Screen::Books(screen) = &mut self.screen
                    && screen.host == host
                {
                    screen.books = Loading::from(books);
                    want_covers(&mut self.images, screen, &self.sender);
                }
            }
            Message::Book(host, book_id, book) => {
                if let Screen::Book(screen) = &mut self.screen
                    && screen.host == host
                    && screen.book_id == book_id
                {
                    screen.book = Loading::from(book);

                    if let Some(leaf_id) = screen.pending_leaf.take()
                        && let Some(index) = screen.book.ready().and_then(|it| it.leaves.iter().position(|leaf| leaf.id == leaf_id))
                    {
                        screen.selected = index;
                    }

                    clamp(&mut screen.selected, screen.book.ready().map(|it| it.leaves.len()).unwrap_or(0));
                    want_pictures(&mut self.images, screen, &self.sender);
                }
            }
            Message::Image(path, image) => self.images.arrived(path, image),
            Message::Changed(result) => {
                if let Err(error) = result {
                    self.notice = Some(format!("{error:#}"));
                }
                self.reload_book();
            }
            Message::Published(result) => self.published(result),
            Message::Form(form) => {
                if let Screen::Edit(page, _) = &mut self.screen {
                    page.form = Loading::from(form);

                    if let Some(cover) = page.form.ready().and_then(|it| it.cover.clone()) {
                        self.images.want(&page.host, &cover, &self.sender);
                    }
                }
            }
            Message::Saved(result) => self.saved(result),
            Message::BookDeleted(result) => self.book_deleted(result),
            Message::LeafLoaded(book_id, leaf_id, content) => {
                if let Screen::Leaf(page, _) = &mut self.screen
                    && page.book_id == book_id
                    && page.leaf().id == leaf_id
                {
                    page.content = Loading::from(content);
                    for url in page.images() {
                        self.images.want(&page.host, &url, &self.sender);
                    }
                }
            }
            Message::LeafSaved(result) => self.leaf_saved(result),
            Message::Uploaded(result) => match result {
                Ok(markdown) => {
                    if let Screen::Leaf(page, _) = &mut self.screen
                        && let Some(editor) = &mut page.editor
                    {
                        editor.insert_block(&markdown);
                    }
                }
                Err(error) => self.notice = Some(format!("{error:#}")),
            },
            Message::Found(found) => {
                if let Some(Overlay::Search(search)) = &mut self.overlay {
                    search.results = Some(Loading::from(found));
                }
            }
        }
    }

    fn published(&mut self, result: Result<Publication>) {
        match result {
            Ok(publication) => {
                if let Some(Overlay::Publishing(publishing)) = &mut self.overlay {
                    publishing.published = publication.published;
                    publishing.slug = publication.slug.clone();
                    publishing.url = publication.url.clone();
                    publishing.saving = false;
                }

                if let Screen::Book(screen) = &mut self.screen
                    && let Loading::Ready(book) = &mut screen.book
                {
                    book.published = publication.published;
                    book.slug = publication.slug;
                }
            }
            Err(error) => {
                if let Some(Overlay::Publishing(publishing)) = &mut self.overlay {
                    publishing.saving = false;
                }
                self.notice = Some(format!("{error:#}"));
            }
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
        } else if self.moving() {
            self.move_key(key);
        } else if is_instances_shortcut(key) && self.reading() {
            let selected = if let Screen::Leaf(page, _) = &self.screen { page.index } else { 0 };
            self.overlay = Some(Overlay::Index { selected });
        } else if is_instances_shortcut(key) && !matches!(self.screen, Screen::Instances { .. } | Screen::Leaf(..)) && !self.tab_moves_focus(key) {
            self.overlay = Some(Overlay::Instances { selected: self.current_instance() });
        } else {
            self.screen_key(key);
        }
    }

    /// Ctrl+C asks first, and a second one while asking quits.
    fn ask_to_quit(&mut self) {
        match self.overlay.take() {
            Some(Overlay::Quit { .. }) => self.done = true,
            previous => self.overlay = Some(Overlay::Quit { previous: previous.map(Box::new), confirm: Confirm::yes() }),
        }
    }

    fn screen_key(&mut self, key: KeyEvent) {
        let count = self.instances.ready().map(Vec::len).unwrap_or(0);

        if let Screen::Edit(page, _) = &mut self.screen {
            match page.key(key) {
                edit::Action::Nothing => {}
                edit::Action::Cancel => self.close_editor(false),
                edit::Action::Save => self.save_book(),
                edit::Action::Delete => self.overlay = Some(Overlay::DeleteBook(Confirm::no())),
                edit::Action::ChooseCover => self.overlay = Some(self.file_picker("Choose a cover", Purpose::Cover)),
            }
            return;
        }

        match &mut self.screen {
            Screen::Instances { selected } => match key.code {
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + 1).min(count.saturating_sub(1)),
                KeyCode::Enter => {
                    let selected = *selected;
                    self.open_instance(selected);
                }
                _ => {}
            },
            Screen::Edit(..) => {}
            Screen::Leaf(..) => self.leaf_key(key),
            Screen::Books(screen) => match key.code {
                KeyCode::Enter => self.open_book(),
                KeyCode::Esc => self.screen = Screen::Instances { selected: self.current_instance() },
                KeyCode::Char('v') => self.books_view = self.books_view.toggled(),
                KeyCode::Char('n') => self.open_editor(None),
                KeyCode::Char('/') => self.overlay = Some(Overlay::Search(Box::new(Search::new(None, "all books".to_string())))),
                _ => {
                    let count = screen.books.ready().map(Vec::len).unwrap_or(0);
                    navigate(key, &mut screen.selected, screen.columns, count);
                }
            },
            Screen::Book(screen) => {
                let editable = screen.book.ready().is_some_and(|it| it.editable);
                let has_leaf = screen.book.ready().is_some_and(|it| !it.leaves.is_empty());

                match key.code {
                    KeyCode::Esc => self.close_book(),
                    KeyCode::Enter if has_leaf => self.open_leaf(),
                    KeyCode::Char('v') => self.book_view = self.book_view.toggled(),
                    KeyCode::Char('/') => {
                        let title = screen.book.ready().map(|it| format!("“{}”", it.title)).unwrap_or_else(|| "this book".to_string());
                        self.overlay = Some(Overlay::Search(Box::new(Search::new(Some(screen.book_id), title))));
                    }
                    KeyCode::Char('a') if editable => self.overlay = Some(Overlay::Add(Add { choice: 0, form: None })),
                    KeyCode::Char('d') if editable && has_leaf => self.overlay = Some(Overlay::Delete(Confirm::no())),
                    KeyCode::Char('m') if editable && has_leaf => {
                        let order = screen.book.ready().map(|it| it.leaves.clone()).unwrap_or_default();
                        screen.moving = Some(Moving { order });
                    }
                    KeyCode::Char('p') if editable => self.overlay = self.publishing(),
                    KeyCode::Char('e') if editable => {
                        let book_id = screen.book_id;
                        self.open_editor(Some(book_id));
                    }
                    _ => {
                        let count = screen.book.ready().map(|it| it.leaves.len()).unwrap_or(0);
                        navigate(key, &mut screen.selected, screen.columns, count);
                    }
                }
            }
        }
    }

    fn move_key(&mut self, key: KeyEvent) {
        let Screen::Book(screen) = &mut self.screen else { return };
        let Some(moving) = &mut screen.moving else { return };

        match key.code {
            KeyCode::Esc => screen.moving = None,
            KeyCode::Enter => {
                let order: Vec<u64> = moving.order.iter().map(|it| it.id).collect();
                let (host, book_id, sender) = (screen.host.clone(), screen.book_id, self.sender.clone());

                if let Loading::Ready(book) = &mut screen.book {
                    book.leaves = std::mem::take(&mut moving.order);
                }
                screen.moving = None;

                std::thread::spawn(move || {
                    let _ = sender.send(Message::Changed(data::arrange(&host, book_id, &order)));
                });
            }
            _ => {
                let from = screen.selected;
                navigate(key, &mut screen.selected, screen.columns, moving.order.len());
                let leaf = moving.order.remove(from);
                moving.order.insert(screen.selected, leaf);
            }
        }
    }

    fn overlay_key(&mut self, overlay: Overlay, key: KeyEvent) -> Option<Overlay> {
        match overlay {
            Overlay::Instances { mut selected } => {
                let count = self.instances.ready().map(Vec::len).unwrap_or(0);

                match key.code {
                    KeyCode::Esc => None,
                    KeyCode::Enter => {
                        self.open_instance(selected);
                        None
                    }
                    KeyCode::Up | KeyCode::Char('k') => Some(Overlay::Instances { selected: selected.saturating_sub(1) }),
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = (selected + 1).min(count.saturating_sub(1));
                        Some(Overlay::Instances { selected })
                    }
                    _ => Some(Overlay::Instances { selected }),
                }
            }
            Overlay::Add(add) => self.add_key(add, key),
            Overlay::Delete(mut confirm) => match confirm.answer(key) {
                Some(true) => {
                    if matches!(self.screen, Screen::Leaf(..)) {
                        self.close_leaf();
                    }
                    self.delete_selected();
                    None
                }
                Some(false) => None,
                None => Some(Overlay::Delete(confirm)),
            },
            Overlay::DeleteBook(mut confirm) => match confirm.answer(key) {
                Some(true) => {
                    self.delete_book();
                    None
                }
                Some(false) => None,
                None => Some(Overlay::DeleteBook(confirm)),
            },
            Overlay::Publishing(publishing) => self.publishing_key(publishing, key),
            Overlay::Index { mut selected } => {
                let Screen::Leaf(page, _) = &mut self.screen else { return None };
                let count = page.leaves.len();

                match key.code {
                    KeyCode::Esc => return None,
                    KeyCode::Enter => {
                        if selected != page.index {
                            page.turn_to(selected);
                            self.load_leaf();
                        }
                        return None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(count.saturating_sub(1)),
                    _ => {}
                }

                Some(Overlay::Index { selected })
            }
            Overlay::Discard(mut confirm) => match confirm.answer(key) {
                Some(true) => {
                    if let Screen::Leaf(page, _) = &mut self.screen {
                        page.editor = None;
                    }
                    None
                }
                Some(false) => None,
                None => Some(Overlay::Discard(confirm)),
            },
            Overlay::Search(mut search) => match search.key(key) {
                search::Action::Nothing => Some(Overlay::Search(search)),
                search::Action::Close => None,
                search::Action::Search(query) => {
                    self.search(search.book_id, query);
                    Some(Overlay::Search(search))
                }
                search::Action::Open(found) => {
                    self.open_found(found);
                    None
                }
            },
            Overlay::Files(mut picker, purpose) => match picker.key(key) {
                Picked::Nothing => {
                    self.want_preview(&picker);
                    Some(Overlay::Files(picker, purpose))
                }
                Picked::Cancel => purpose.back(),
                Picked::File(path) => {
                    self.files_dir = picker.dir().to_path_buf();
                    self.picked(path, purpose)
                }
            },
            Overlay::Quit { previous, mut confirm } => match confirm.answer(key) {
                Some(true) => {
                    self.done = true;
                    None
                }
                Some(false) => previous.map(|it| *it),
                None => Some(Overlay::Quit { previous, confirm }),
            },
        }
    }

    fn add_key(&mut self, mut add: Add, key: KeyEvent) -> Option<Overlay> {
        match &mut add.form {
            None => match key.code {
                KeyCode::Esc => return None,
                KeyCode::Up | KeyCode::Char('k') => add.choice = add.choice.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => add.choice = (add.choice + 1).min(Kind::ALL.len() - 1),
                KeyCode::Enter => {
                    let kind = Kind::ALL[add.choice];
                    add.form = Some(Form { kind, fields: kind.fields(), focus: 0 });
                }
                _ => {}
            },
            Some(form) => {
                let saving = key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL);

                match (key.code, form.focus) {
                    _ if saving => return self.submit(add),
                    (KeyCode::Esc, _) => add.form = None,
                    (KeyCode::Tab | KeyCode::Down, _) => form.focus = (form.focus + 1) % form.stops(),
                    (KeyCode::BackTab | KeyCode::Up, _) => form.focus = (form.focus + form.stops() - 1) % form.stops(),
                    (KeyCode::Enter, focus) if focus == form.cancel_button() => return None,
                    (KeyCode::Enter, _) if form.field().is_some_and(|it| it.file) => {
                        return Some(self.file_picker("Choose a picture", Purpose::Picture(Box::new(add))));
                    }
                    (KeyCode::Enter, focus) if focus == form.add_button() => return self.submit(add),
                    (KeyCode::Enter, _) => form.focus += 1,
                    (KeyCode::Backspace, _) => {
                        if let Some(field) = form.typed_field() {
                            field.value.pop();
                        }
                    }
                    (KeyCode::Char(character), _) => {
                        if let Some(field) = form.typed_field() {
                            field.value.push(character);
                        }
                    }
                    _ => {}
                }
            }
        }

        Some(Overlay::Add(add))
    }

    fn search(&self, book_id: Option<u64>, query: String) {
        let host = match &self.screen {
            Screen::Books(screen) => screen.host.clone(),
            Screen::Book(screen) => screen.host.clone(),
            _ => return,
        };

        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Message::Found(data::search(&host, book_id, &query)));
        });
    }

    /// Show the page a search found: select it in this book, or open its book.
    fn open_found(&mut self, found: data::Found) {
        let Some(book_id) = found.book_id else { return };

        match &mut self.screen {
            Screen::Book(screen) if screen.book_id == book_id => {
                if let Some(index) = screen.book.ready().and_then(|it| it.leaves.iter().position(|leaf| Some(leaf.id) == found.leaf_id)) {
                    screen.selected = index;
                }
            }
            Screen::Books(_) => {
                self.open_book_by_id(book_id);

                if let Screen::Book(screen) = &mut self.screen {
                    screen.pending_leaf = found.leaf_id;
                }
            }
            _ => {}
        }
    }

    fn file_picker(&mut self, title: &'static str, purpose: Purpose) -> Overlay {
        self.file_picker_for(title, files::IMAGES, purpose)
    }

    fn file_picker_for(&mut self, title: &'static str, kinds: &'static [&'static str], purpose: Purpose) -> Overlay {
        let picker = FilePicker::new(title, kinds, self.files_dir.clone());
        self.want_preview(&picker);
        Overlay::Files(Box::new(picker), purpose)
    }

    fn want_preview(&mut self, picker: &FilePicker) {
        if let Some(file) = picker.selected_file() {
            self.images.want_file(&file, &self.sender);
        }
    }

    /// Put the picked file where it was wanted, and go back there.
    fn picked(&mut self, path: std::path::PathBuf, purpose: Purpose) -> Option<Overlay> {
        match purpose {
            Purpose::Cover => {
                if let Screen::Edit(page, _) = &mut self.screen {
                    page.use_cover(path);
                }
                None
            }
            Purpose::Upload => {
                self.upload(path);
                None
            }
            Purpose::Picture(mut add) => {
                if let Some(form) = &mut add.form
                    && let Some(field) = form.fields.iter_mut().find(|it| it.file)
                {
                    field.value = path.to_string_lossy().into_owned();
                }
                Some(Overlay::Add(*add))
            }
        }
    }

    /// Add the page, picture or section once every field is filled in.
    fn submit(&mut self, add: Add) -> Option<Overlay> {
        match &add.form {
            Some(form) if form.fields.iter().all(|it| !it.value.trim().is_empty()) => {
                self.add_leaf(form);
                None
            }
            _ => {
                self.notice = Some("Fill in every field first.".to_string());
                Some(Overlay::Add(add))
            }
        }
    }

    fn publishing_key(&mut self, mut publishing: Publishing, key: KeyEvent) -> Option<Overlay> {
        if let Some(slug) = &mut publishing.editing {
            match key.code {
                KeyCode::Esc => publishing.editing = None,
                KeyCode::Backspace => {
                    slug.pop();
                }
                KeyCode::Char(character) => slug.push(character),
                KeyCode::Enter if !slug.trim().is_empty() => {
                    let slug = publishing.editing.take().unwrap_or_default();
                    let published = publishing.published;
                    self.publish(&mut publishing, published, slug);
                }
                _ => {}
            }

            return Some(Overlay::Publishing(publishing));
        }

        match key.code {
            KeyCode::Esc | KeyCode::Enter => return None,
            KeyCode::Up | KeyCode::Down | KeyCode::Char('k') | KeyCode::Char('j') | KeyCode::Char(' ') if !publishing.saving => {
                let (published, slug) = (!publishing.published, publishing.slug.clone());
                self.publish(&mut publishing, published, slug);
            }
            KeyCode::Char('c') if publishing.published => {
                copy(&publishing.url);
                self.notice = Some(format!("Copied {}", publishing.url));
            }
            KeyCode::Char('q') if publishing.published => publishing.qr = !publishing.qr,
            KeyCode::Char('e') => publishing.editing = Some(publishing.slug.clone()),
            _ => {}
        }

        Some(Overlay::Publishing(publishing))
    }

    fn open_instance(&mut self, index: usize) {
        let Some(instance) = self.instances.ready().and_then(|it| it.get(index)) else { return };
        let host = instance.url.clone();

        self.screen = Screen::Books(Books {
            host: host.clone(),
            name: instance.alias.clone(),
            books: Loading::Pending,
            selected: 0,
            columns: 1,
            offset: 0,
        });

        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let books = data::books(&host);
            let _ = sender.send(Message::Books(host, books));
        });
    }

    fn open_book(&mut self) {
        let screen = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });
        let Screen::Books(books) = screen else { return };
        let Some(book_id) = books.books.ready().and_then(|it| it.get(books.selected)).map(|it| it.id) else {
            self.screen = Screen::Books(books);
            return;
        };

        let host = books.host.clone();
        self.screen = Screen::Book(Box::new(BookPage {
            host: host.clone(),
            book_id,
            book: Loading::Pending,
            selected: 0,
            columns: 1,
            offset: 0,
            moving: None,
            pending_leaf: None,
            books,
        }));
        self.load_book(host, book_id);
    }

    fn close_book(&mut self) {
        let screen = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });

        if let Screen::Book(page) = screen {
            self.screen = Screen::Books(page.books);
            self.reload_books();
        }
    }

    /// Open the selected page, section or picture of the book.
    fn open_leaf(&mut self) {
        let screen = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });
        let Screen::Book(book) = screen else {
            self.screen = screen;
            return;
        };
        let Some(details) = book.book.ready() else {
            self.screen = Screen::Book(book);
            return;
        };

        let page = LeafPage {
            host: book.host.clone(),
            book_id: book.book_id,
            book_title: details.title.clone(),
            editable: details.editable,
            leaves: details.leaves.clone(),
            index: book.selected,
            content: Loading::Pending,
            scroll: 0,
            editor: None,
            saving: false,
        };

        self.screen = Screen::Leaf(Box::new(page), book);
        self.load_leaf();
    }

    fn load_leaf(&self) {
        let Screen::Leaf(page, _) = &self.screen else { return };
        let (host, book_id, leaf, sender) = (page.host.clone(), page.book_id, page.leaf().clone(), self.sender.clone());

        std::thread::spawn(move || {
            let content = data::leaf(&host, book_id, &leaf);
            let _ = sender.send(Message::LeafLoaded(book_id, leaf.id, content));
        });
    }

    /// Back to the book, with the page that was open selected.
    fn close_leaf(&mut self) {
        let screen = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });
        let Screen::Leaf(page, mut book) = screen else {
            self.screen = screen;
            return;
        };

        book.selected = page.index;
        self.screen = Screen::Book(book);
        self.reload_book();
    }

    fn leaf_key(&mut self, key: KeyEvent) {
        let Screen::Leaf(page, _) = &mut self.screen else { return };

        if let Some(editor) = &mut page.editor {
            if page.saving {
                return;
            }

            match editor.key(key) {
                editor::Action::Nothing => {}
                editor::Action::Save => self.save_leaf(),
                editor::Action::Upload => self.overlay = Some(self.file_picker_for("Upload a file", files::ANY, Purpose::Upload)),
                editor::Action::Leave if editor.changed() => self.overlay = Some(Overlay::Discard(Confirm::no())),
                editor::Action::Leave => page.editor = None,
            }
            return;
        }

        match page.key(key) {
            reader::Action::Nothing => {}
            reader::Action::Turned => self.load_leaf(),
            reader::Action::Back => self.close_leaf(),
            reader::Action::Delete => self.overlay = Some(Overlay::Delete(Confirm::no())),
            reader::Action::Edit => {
                if let Loading::Ready(content) = &page.content {
                    page.editor = Some(editor::Editor::new(&content.title, &content.body));
                }
            }
        }
    }

    fn save_leaf(&mut self) {
        let Screen::Leaf(page, _) = &mut self.screen else { return };
        let (Some(editor), Loading::Ready(content)) = (&page.editor, &page.content) else { return };

        let (title, body, fingerprint) = (editor.title.clone(), editor.body(), content.fingerprint.clone());
        let (host, book_id, leaf, sender) = (page.host.clone(), page.book_id, page.leaf().clone(), self.sender.clone());
        page.saving = true;

        std::thread::spawn(move || {
            let _ = sender.send(Message::LeafSaved(data::save_leaf(&host, book_id, &leaf, &title, &body, &fingerprint)));
        });
    }

    /// After saving, show the page as saved.
    fn leaf_saved(&mut self, result: Result<String>) {
        let Screen::Leaf(page, _) = &mut self.screen else { return };
        page.saving = false;

        match result {
            Ok(fingerprint) => {
                if let (Some(editor), Loading::Ready(content)) = (page.editor.take(), &mut page.content) {
                    content.title = editor.title.clone();
                    content.body = editor.body();
                    content.fingerprint = fingerprint;
                    page.leaves[page.index].title = editor.title;
                }

                for url in page.images() {
                    self.images.want(&page.host, &url, &self.sender);
                }
            }
            Err(error) => self.notice = Some(format!("{error:#}")),
        }
    }

    fn upload(&mut self, path: std::path::PathBuf) {
        let Screen::Leaf(page, _) = &self.screen else { return };
        let Some(record_gid) = page.content.ready().and_then(|it| it.record_gid.clone()) else {
            self.notice = Some("Files can only go into a page.".to_string());
            return;
        };

        let (host, sender) = (page.host.clone(), self.sender.clone());
        self.notice = Some(format!("Uploading {}…", path.display()));
        std::thread::spawn(move || {
            let _ = sender.send(Message::Uploaded(data::upload(&host, &record_gid, &path)));
        });
    }

    fn reload_books(&self) {
        if let Screen::Books(screen) = &self.screen {
            let (host, sender) = (screen.host.clone(), self.sender.clone());
            std::thread::spawn(move || {
                let books = data::books(&host);
                let _ = sender.send(Message::Books(host, books));
            });
        }
    }

    /// The edit screen for a book, or for a new one without an id.
    fn open_editor(&mut self, book_id: Option<u64>) {
        let (host, name) = match &self.screen {
            Screen::Books(screen) => (screen.host.clone(), screen.name.clone()),
            Screen::Book(screen) => (screen.host.clone(), screen.books.name.clone()),
            _ => return,
        };

        let back = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });
        self.screen = Screen::Edit(Box::new(EditPage::new(host.clone(), name, book_id)), Box::new(back));

        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Message::Form(data::book_form(&host, book_id)));
        });
    }

    /// Back to where the editor was opened, reloading what it may have changed.
    fn close_editor(&mut self, changed: bool) {
        let screen = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });
        let Screen::Edit(_, back) = screen else {
            self.screen = screen;
            return;
        };

        self.screen = *back;

        if changed {
            self.reload_book();
            self.reload_books();
        }
    }

    fn save_book(&mut self) {
        let Screen::Edit(page, _) = &mut self.screen else { return };
        let Some(form) = page.form.ready().cloned() else { return };

        if form.title.trim().is_empty() {
            self.notice = Some("A book needs a title.".to_string());
            return;
        }

        page.saving = true;
        let (host, sender) = (page.host.clone(), self.sender.clone());
        std::thread::spawn(move || {
            let _ = sender.send(Message::Saved(data::save_book(&host, &form)));
        });
    }

    fn saved(&mut self, result: Result<u64>) {
        let Screen::Edit(page, _) = &mut self.screen else { return };
        page.saving = false;

        match result {
            Ok(book_id) if page.book_id.is_none() => {
                self.close_editor(true);
                self.open_book_by_id(book_id);
            }
            Ok(_) => self.close_editor(true),
            Err(error) => self.notice = Some(format!("{error:#}")),
        }
    }

    fn delete_book(&mut self) {
        let Screen::Edit(page, _) = &mut self.screen else { return };
        let Some(book_id) = page.book_id else { return };

        page.saving = true;
        let (host, sender) = (page.host.clone(), self.sender.clone());
        std::thread::spawn(move || {
            let _ = sender.send(Message::BookDeleted(data::delete_book(&host, book_id)));
        });
    }

    /// After a book is deleted, back to the books it was among.
    fn book_deleted(&mut self, result: Result<()>) {
        if let Err(error) = result {
            if let Screen::Edit(page, _) = &mut self.screen {
                page.saving = false;
            }
            self.notice = Some(format!("{error:#}"));
            return;
        }

        self.close_editor(false);
        self.close_book();
    }

    /// Open a book that may not be in the loaded list yet, like a new one.
    fn open_book_by_id(&mut self, book_id: u64) {
        let screen = std::mem::replace(&mut self.screen, Screen::Instances { selected: 0 });
        let Screen::Books(books) = screen else {
            self.screen = screen;
            return;
        };

        let host = books.host.clone();
        self.screen = Screen::Book(Box::new(BookPage {
            host: host.clone(),
            book_id,
            book: Loading::Pending,
            selected: 0,
            columns: 1,
            offset: 0,
            moving: None,
            pending_leaf: None,
            books,
        }));
        self.load_book(host, book_id);
    }

    fn reload_book(&self) {
        if let Screen::Book(screen) = &self.screen {
            self.load_book(screen.host.clone(), screen.book_id);
        }
    }

    fn load_book(&self, host: String, book_id: u64) {
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let book = data::book(&host, book_id);
            let _ = sender.send(Message::Book(host, book_id, book));
        });
    }

    fn add_leaf(&self, form: &Form) {
        let Screen::Book(screen) = &self.screen else { return };
        let value = |index: usize| form.fields[index].value.trim().to_string();
        let leaf = match form.kind {
            Kind::Page => NewLeaf::Page { title: value(0) },
            Kind::Section => NewLeaf::Section { title: value(0) },
            Kind::Picture => NewLeaf::Picture { caption: value(0), path: value(1) },
        };

        let (host, book_id, sender) = (screen.host.clone(), screen.book_id, self.sender.clone());
        std::thread::spawn(move || {
            let _ = sender.send(Message::Changed(data::add(&host, book_id, &leaf)));
        });
    }

    fn delete_selected(&self) {
        let Screen::Book(screen) = &self.screen else { return };
        let Some(leaf) = screen.book.ready().and_then(|it| it.leaves.get(screen.selected)).cloned() else { return };

        let (host, book_id, sender) = (screen.host.clone(), screen.book_id, self.sender.clone());
        std::thread::spawn(move || {
            let _ = sender.send(Message::Changed(data::delete(&host, book_id, &leaf)));
        });
    }

    fn publishing(&self) -> Option<Overlay> {
        let Screen::Book(screen) = &self.screen else { return None };
        let book = screen.book.ready()?;

        Some(Overlay::Publishing(Publishing {
            published: book.published,
            slug: book.slug.clone(),
            url: format!("{}/{}/{}", screen.host, book.id, book.slug),
            editing: None,
            qr: false,
            saving: false,
        }))
    }

    fn publish(&self, publishing: &mut Publishing, published: bool, slug: String) {
        let Screen::Book(screen) = &self.screen else { return };
        let (host, book_id, sender) = (screen.host.clone(), screen.book_id, self.sender.clone());
        publishing.saving = true;

        std::thread::spawn(move || {
            let _ = sender.send(Message::Published(data::publish(&host, book_id, published, &slug)));
        });
    }

    /// Whether keys go into a text field, where ? is just a character.
    fn typing(&self) -> bool {
        match (&self.overlay, &self.screen) {
            (Some(Overlay::Add(add)), _) => add.form.is_some(),
            (Some(Overlay::Publishing(publishing)), _) => publishing.editing.is_some(),
            (Some(Overlay::Files(..) | Overlay::Search(_)), _) => true,
            (None, Screen::Edit(page, _)) => page.typing(),
            (None, Screen::Leaf(page, _)) => page.editor.is_some(),
            _ => false,
        }
    }

    /// On the edit screen tab moves between fields, so only a real ctrl+i
    /// opens the instances there.
    fn tab_moves_focus(&self, key: KeyEvent) -> bool {
        key.code == KeyCode::Tab && matches!(self.screen, Screen::Edit(..))
    }

    /// Reading a page, not editing it.
    fn reading(&self) -> bool {
        matches!(&self.screen, Screen::Leaf(page, _) if page.editor.is_none())
    }

    fn moving(&self) -> bool {
        matches!(&self.screen, Screen::Book(screen) if screen.moving.is_some())
    }

    fn current_instance(&self) -> usize {
        let host = match &self.screen {
            Screen::Instances { selected } => return *selected,
            Screen::Books(screen) => &screen.host,
            Screen::Book(screen) => &screen.host,
            Screen::Edit(page, _) => &page.host,
            Screen::Leaf(page, _) => &page.host,
        };

        self.instances.ready().and_then(|it| it.iter().position(|instance| &instance.url == host)).unwrap_or(0)
    }

    fn draw(&mut self, frame: &mut Frame) {
        let toggle = help::toggle(self.typing());
        let room = frame.area().width.saturating_sub(toggle.width() as u16 + 3);
        let lines = if self.help { help::lines(&self.bindings(), self.notice.as_deref(), room) } else { help::lines(&[], self.notice.as_deref(), room) };
        let [main, bar] = Layout::vertical([Constraint::Fill(1), Constraint::Length(lines.len().max(1) as u16)]).areas(frame.area());

        match &mut self.screen {
            Screen::Instances { selected } => {
                self.starfield.render(main, frame.buffer_mut());
                views::instances(frame, main, &self.instances, *selected);
            }
            Screen::Books(screen) => {
                let area = views::masthead(frame, main);
                views::books(frame, area, screen, self.books_view, &mut self.images);
            }
            Screen::Book(screen) => {
                let area = views::masthead(frame, main);
                views::book(frame, area, screen, self.book_view, &mut self.images);
            }
            Screen::Edit(page, _) => {
                let area = views::masthead(frame, main);
                let [heading, area] = Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(area);
                let title = page.form.ready().map(|it| it.title.clone()).filter(|it| !it.is_empty() && page.book_id.is_some());
                let last = if page.book_id.is_some() { "Edit" } else { "New book" };
                let mut steps = vec![page.name.as_str(), "Books"];
                steps.extend(title.as_deref());
                steps.push(last);
                views::breadcrumbs(frame, heading, &steps);
                page.draw(frame, area, &mut self.images);
            }
            Screen::Leaf(page, book) => {
                let area = views::masthead(frame, main);
                let [heading, area] = Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(area);
                let title = match &page.content {
                    Loading::Ready(content) if !content.title.is_empty() => content.title.clone(),
                    _ => page.leaf().title.clone(),
                };
                let mut steps = vec![book.books.name.as_str(), "Books", page.book_title.as_str(), title.as_str()];
                if page.editor.is_some() {
                    steps.push("Edit");
                }
                views::breadcrumbs(frame, heading, &steps);
                page.draw(frame, area, &mut self.images);
            }
        }

        match &mut self.overlay {
            Some(Overlay::Instances { selected }) => views::instances(frame, main, &self.instances, *selected),
            Some(Overlay::Add(add)) => views::add(frame, main, add),
            Some(Overlay::Delete(confirm)) => match &self.screen {
                Screen::Book(screen) => {
                    if let Some(leaf) = screen.book.ready().and_then(|it| it.leaves.get(screen.selected)) {
                        views::delete(frame, main, leaf, *confirm);
                    }
                }
                Screen::Leaf(page, _) => views::delete(frame, main, page.leaf(), *confirm),
                _ => {}
            },
            Some(Overlay::Index { selected }) => {
                if let Screen::Leaf(page, _) = &self.screen {
                    page.draw_index(frame, main, *selected);
                }
            }
            Some(Overlay::Discard(confirm)) => views::discard(frame, main, *confirm),
            Some(Overlay::Publishing(publishing)) => views::publishing(frame, main, publishing),
            Some(Overlay::Files(picker, _)) => picker.draw(frame, main, &mut self.images),
            Some(Overlay::Search(search)) => search.draw(frame, main),
            Some(Overlay::DeleteBook(confirm)) => {
                if let Screen::Edit(page, _) = &self.screen
                    && let Some(form) = page.form.ready()
                {
                    views::delete_book(frame, main, &form.title, *confirm);
                }
            }
            Some(Overlay::Quit { confirm, .. }) => views::quit(frame, main, *confirm),
            None => {}
        }

        let last_line = Rect { y: bar.bottom().saturating_sub(1), height: 1, ..bar };
        frame.render_widget(Paragraph::new(lines), bar);
        frame.render_widget(Paragraph::new(toggle).alignment(ratatui::layout::Alignment::Right), last_line);
    }

    fn bindings(&self) -> Vec<Binding> {
        match (&self.overlay, &self.screen) {
            (Some(Overlay::Instances { .. }), _) => vec![("↑↓", "select"), ("enter", "open"), ("esc", "close")],
            (Some(Overlay::Search(search)), _) => search.bindings(),
            (Some(Overlay::Files(..)), _) => vec![("type", "find"), ("↑↓", "select"), ("enter", "open"), ("←", "up a folder"), ("~", "home"), ("esc", "cancel")],
            (Some(Overlay::Add(Add { form: None, .. })), _) => vec![("↑↓", "select"), ("enter", "choose"), ("esc", "close")],
            (Some(Overlay::Add(Add { form: Some(form), .. })), _) => {
                let enter = match form.focus {
                    focus if focus == form.add_button() => "add",
                    focus if focus == form.cancel_button() => "cancel",
                    _ if form.field().is_some_and(|it| it.file) => "choose a file",
                    _ => "next",
                };
                vec![("tab", "next"), ("enter", enter), ("esc", "back"), ("ctrl+s", "add")]
            }
            (Some(Overlay::Delete(_) | Overlay::DeleteBook(_)), _) => vec![("←→", "choose"), ("enter", "answer"), ("esc", "cancel")],
            (Some(Overlay::Publishing(publishing)), _) if publishing.editing.is_some() => vec![("enter", "save address"), ("esc", "cancel")],
            (Some(Overlay::Publishing(publishing)), _) if publishing.published => {
                vec![("↑↓", "visibility"), ("c", "copy address"), ("q", "QR code"), ("e", "edit address"), ("enter", "close")]
            }
            (Some(Overlay::Publishing(_)), _) => vec![("↑↓", "visibility"), ("e", "edit address"), ("enter", "close")],
            (Some(Overlay::Quit { .. }), _) => vec![("←→", "choose"), ("enter", "answer"), ("esc", "stay"), ("ctrl+c", "quit")],
            (None, Screen::Instances { .. }) => vec![("↑↓", "select"), ("enter", "open"), ("ctrl+c", "quit")],
            (None, Screen::Books(_)) => {
                let mut bindings = vec![(arrows(self.books_view), "select"), ("enter", "open"), ("v", self.view_name(self.books_view)), ("/", "search"), ("n", "new book")];
                bindings.extend([("esc", "instances"), ("ctrl+i", "instances"), ("ctrl+c", "quit")]);
                bindings
            }
            (None, Screen::Edit(page, _)) => page.bindings(),
            (None, Screen::Leaf(page, _)) if page.editor.is_some() => {
                vec![("tab", "title or text"), ("esc", "stop editing"), ("ctrl+u", "upload a file"), ("ctrl+s", "save")]
            }
            (None, Screen::Leaf(page, _)) => {
                let mut bindings = vec![("←→", "turn pages"), ("↑↓", "scroll")];
                if page.can_edit_text() {
                    bindings.push(("e", "edit"));
                }
                if page.editable {
                    bindings.push(("d", "delete"));
                }
                bindings.extend([("esc", "book"), ("ctrl+i", "index"), ("ctrl+c", "quit")]);
                bindings
            }
            (Some(Overlay::Index { .. }), _) => vec![("↑↓", "select"), ("enter", "go to page"), ("esc", "close")],
            (Some(Overlay::Discard(_)), _) => vec![("←→", "choose"), ("enter", "answer"), ("esc", "keep editing")],
            (None, Screen::Book(screen)) if screen.moving.is_some() => vec![(arrows(self.book_view), "move"), ("enter", "save"), ("esc", "cancel")],
            (None, Screen::Book(screen)) => {
                let mut bindings = vec![(arrows(self.book_view), "select"), ("enter", "open"), ("v", self.view_name(self.book_view)), ("/", "search")];

                if let Some(book) = screen.book.ready().filter(|it| it.editable) {
                    bindings.push(("a", "add"));

                    if !book.leaves.is_empty() {
                        bindings.extend([("d", "delete"), ("m", "move")]);
                    }

                    bindings.extend([("e", "edit book"), ("p", "publishing")]);
                }

                bindings.extend([("esc", "books"), ("ctrl+i", "instances"), ("ctrl+c", "quit")]);
                bindings
            }
        }
    }

    fn view_name(&self, view: View) -> &'static str {
        match view {
            View::Gallery => "list view",
            View::List => "gallery view",
        }
    }
}

fn want_covers(images: &mut Images, screen: &Books, sender: &Sender<Message>) {
    for book in screen.books.ready().into_iter().flatten() {
        if let Some(cover) = &book.cover {
            images.want(&screen.host, &cover.url, sender);
        }
    }
}

fn want_pictures(images: &mut Images, screen: &BookPage, sender: &Sender<Message>) {
    let Some(book) = screen.book.ready() else { return };

    for path in book.cover.iter().chain(book.leaves.iter().filter_map(|it| it.image.as_ref().map(|image| &image.url))) {
        images.want(&screen.host, path, sender);
    }
}

/// The arrows that move the selection: all four in a grid, up and down in a list.
fn arrows(view: View) -> &'static str {
    match view {
        View::Gallery => "←↑↓→",
        View::List => "↑↓",
    }
}

fn is_instances_shortcut(key: KeyEvent) -> bool {
    key.code == KeyCode::Tab || (key.code == KeyCode::Char('i') && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Move the selection with the arrows: by one across, by a row up and down.
fn navigate(key: KeyEvent, selected: &mut usize, columns: usize, count: usize) {
    if count == 0 {
        return;
    }

    match key.code {
        KeyCode::Left | KeyCode::Char('h') => *selected = selected.saturating_sub(1),
        KeyCode::Right | KeyCode::Char('l') => *selected = (*selected + 1).min(count - 1),
        KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(columns),
        KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + columns).min(count - 1),
        _ => {}
    }
}

fn clamp(selected: &mut usize, count: usize) {
    *selected = (*selected).min(count.saturating_sub(1));
}

/// Put the text on the clipboard through the terminal (OSC 52), which works
/// over SSH too.
fn copy(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = stdout.flush();
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();

    for chunk in bytes.chunks(3) {
        let group = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));

        for position in 0..4 {
            if position <= chunk.len() {
                encoded.push(ALPHABET[(group >> (18 - 6 * position) & 63) as usize] as char);
            } else {
                encoded.push('=');
            }
        }
    }

    encoded
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
        assert!(matches!(app.overlay, Some(Overlay::Quit { .. })));
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
        app.overlay = Some(Overlay::Delete(Confirm::no()));

        app.key(ctrl_c());
        app.key(press(KeyCode::Esc));
        assert!(matches!(app.overlay, Some(Overlay::Delete(_))));

        app.key(ctrl_c());
        app.key(ctrl_c());
        assert!(app.done);
    }

    #[test]
    fn a_confirmation_is_answered_by_choosing_and_pressing_enter() {
        let mut confirm = Confirm::no();

        assert_eq!(confirm.answer(press(KeyCode::Char('y'))), None);
        assert_eq!(confirm.answer(press(KeyCode::Enter)), Some(false));
        assert_eq!(confirm.answer(press(KeyCode::Left)), None);
        assert_eq!(confirm.answer(press(KeyCode::Enter)), Some(true));
        assert_eq!(confirm.answer(press(KeyCode::Tab)), None);
        assert_eq!(confirm.answer(press(KeyCode::Enter)), Some(false));
        assert_eq!(Confirm::yes().answer(press(KeyCode::Esc)), Some(false));
    }

    #[test]
    fn moves_through_a_grid_by_one_across_and_a_row_up_and_down() {
        let mut selected = 0;

        navigate(press(KeyCode::Right), &mut selected, 3, 7);
        assert_eq!(selected, 1);
        navigate(press(KeyCode::Down), &mut selected, 3, 7);
        assert_eq!(selected, 4);
        navigate(press(KeyCode::Down), &mut selected, 3, 7);
        assert_eq!(selected, 6, "the last row is short, so down stops at the last tile");
        navigate(press(KeyCode::Right), &mut selected, 3, 7);
        assert_eq!(selected, 6);
        navigate(press(KeyCode::Up), &mut selected, 3, 7);
        assert_eq!(selected, 3);
        navigate(press(KeyCode::Left), &mut selected, 3, 7);
        assert_eq!(selected, 2);
    }

    #[test]
    fn ctrl_i_and_tab_both_open_the_instances() {
        assert!(is_instances_shortcut(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL)));
        assert!(is_instances_shortcut(press(KeyCode::Tab)));
        assert!(!is_instances_shortcut(press(KeyCode::Char('i'))));
    }

    #[test]
    fn encodes_the_clipboard_text_as_base64() {
        assert_eq!(base64(b"https://books.example.com/2/guide"), "aHR0cHM6Ly9ib29rcy5leGFtcGxlLmNvbS8yL2d1aWRl");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn edits_only_the_slug_of_the_public_address() {
        let publishing = Publishing {
            published: true,
            slug: "guide".to_string(),
            url: "https://books.example.com/2/guide".to_string(),
            editing: None,
            qr: false,
            saving: false,
        };

        assert_eq!(publishing.url_without_slug(), "https://books.example.com/2/");
    }
}
