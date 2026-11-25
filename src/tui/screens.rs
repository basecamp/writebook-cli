//! Handing the screens keys and the messages for them, and going from one
//! screen to another.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

use super::book::{self, BookPage};
use super::books::{self, Books};
use super::confirm::{Confirm, Question};
use super::data::{Book, BookCard, BookForm, LeafContent, People, Profile};
use super::edit::{self, EditPage};
use super::help::Binding;
use super::instances::{self, InstancePicker};
use super::profile::{self, ProfilePage};
use super::publishing::Publishing;
use super::reader::{self, LeafPage};
use super::search::Search;
use super::settings::{self, SettingsPage};
use super::views::{self, View};
use super::{App, Id, Loading, Overlay, Purpose, Screen, files};

/// A screen or an overlay that a reply is for.
pub enum Target<'a> {
    Books(&'a mut Books),
    Book(&'a mut BookPage),
    Edit(&'a mut EditPage),
    Leaf(&'a mut LeafPage),
    Settings(&'a mut SettingsPage),
    Profile(&'a mut ProfilePage),
    Publishing(&'a mut Publishing),
    Search(&'a mut Search),
}

impl Screen {
    /// The screen with this id: this one, or one set aside under it.
    pub fn find(&mut self, id: Id) -> Option<Target<'_>> {
        match self {
            Screen::Instances(_) => None,
            Screen::Books(books) => find_books(books, id),
            Screen::Book(page) => find_book(page, id),
            Screen::Edit(page, back) => {
                if page.id == id {
                    Some(Target::Edit(page))
                } else {
                    back.find(id)
                }
            }
            Screen::Leaf(page, book) => {
                if page.id == id {
                    Some(Target::Leaf(page))
                } else {
                    find_book(book, id)
                }
            }
            Screen::Settings(page, books) => {
                if page.id == id {
                    Some(Target::Settings(page))
                } else {
                    find_books(books, id)
                }
            }
            Screen::Profile(page, back) => {
                if page.id == id {
                    Some(Target::Profile(page))
                } else {
                    back.find(id)
                }
            }
        }
    }

    pub fn host(&self) -> Option<&str> {
        match self {
            Screen::Instances(_) => None,
            Screen::Books(screen) => Some(&screen.host),
            Screen::Book(screen) => Some(&screen.host),
            Screen::Edit(page, _) => Some(&page.host),
            Screen::Leaf(page, _) => Some(&page.host),
            Screen::Settings(page, _) => Some(&page.host),
            Screen::Profile(page, _) => Some(&page.host),
        }
    }

    /// Whether keys go into a text field, where ? is just a character.
    pub fn typing(&self) -> bool {
        match self {
            Screen::Edit(page, _) => page.typing(),
            Screen::Leaf(page, _) => page.editor.is_some(),
            Screen::Profile(page, _) => page.typing(),
            _ => false,
        }
    }

    /// Ctrl+i or tab opens the instances over most screens. On screens with
    /// fields tab moves between them, so only a real ctrl+i does there, and
    /// moving pages takes every key.
    pub fn opens_instances(&self, key: KeyEvent) -> bool {
        match self {
            Screen::Instances(_) | Screen::Leaf(..) => false,
            Screen::Edit(..) | Screen::Profile(..) => key.code != KeyCode::Tab,
            Screen::Book(screen) => !screen.moving(),
            Screen::Books(_) | Screen::Settings(..) => true,
        }
    }

    pub fn bindings(&self, books_view: View, book_view: View) -> Vec<Binding> {
        match self {
            Screen::Instances(picker) => picker.bindings(),
            Screen::Books(screen) => screen.bindings(books_view),
            Screen::Book(screen) => screen.bindings(book_view),
            Screen::Edit(page, _) => page.bindings(),
            Screen::Leaf(page, _) => page.bindings(),
            Screen::Settings(page, _) => page.bindings(),
            Screen::Profile(page, _) => page.bindings(),
        }
    }
}

impl App {
    pub fn draw_screen(&mut self, frame: &mut Frame, main: Rect) {
        let area = match &self.screen {
            Screen::Instances(_) => {
                self.starfield.render(main, frame.buffer_mut());
                main
            }
            _ => views::masthead(frame, main),
        };

        match &mut self.screen {
            Screen::Instances(picker) => picker.draw(frame, area, &self.instances),
            Screen::Books(screen) => screen.draw(frame, area, self.books_view, &mut self.images),
            Screen::Book(screen) => screen.draw(frame, area, self.book_view, &mut self.images),
            Screen::Edit(page, _) => page.draw(frame, area, &mut self.images),
            Screen::Leaf(page, book) => page.draw(frame, area, &book.books.name, &mut self.images),
            Screen::Settings(page, books) => page.draw(frame, area, &books.name),
            Screen::Profile(page, back) => {
                let instance = match back.as_ref() {
                    Screen::Settings(_, books) => books.name.as_str(),
                    _ => "",
                };
                page.draw(frame, area, instance);
            }
        }
    }

    pub fn screen_key(&mut self, key: KeyEvent) {
        match &mut self.screen {
            Screen::Instances(picker) => {
                let count = self.instances.ready().map(Vec::len).unwrap_or(0);

                if let instances::Action::Open(index) = picker.key(key, count) {
                    self.open_instance(index);
                }
            }
            Screen::Books(screen) => match screen.key(key) {
                books::Action::Nothing => {}
                books::Action::Back => self.screen = Screen::Instances(InstancePicker::new(self.current_instance())),
                books::Action::Open(book_id) => self.open_book(book_id),
                books::Action::ToggleView => self.books_view = self.books_view.toggled(),
                books::Action::NewBook => self.open_editor(None),
                books::Action::Settings => self.open_settings(),
                books::Action::Overlay(overlay) => self.overlay = Some(overlay),
            },
            Screen::Book(screen) => match screen.key(key) {
                book::Action::Nothing => {}
                book::Action::Back => self.close_book(),
                book::Action::OpenLeaf => self.open_leaf(),
                book::Action::ToggleView => self.book_view = self.book_view.toggled(),
                book::Action::EditBook(book_id) => self.open_editor(Some(book_id)),
                book::Action::Overlay(overlay) => self.overlay = Some(overlay),
                book::Action::Request(request) => self.spawn(request),
            },
            Screen::Edit(page, _) => match page.key(key) {
                edit::Action::Nothing => {}
                edit::Action::Cancel => self.close_editor(false),
                edit::Action::Save => {
                    if let Some(problem) = page.problem() {
                        self.notice = Some(problem.to_string());
                    } else if let Some(request) = page.save() {
                        self.spawn(request);
                    }
                }
                edit::Action::Delete(book_id, title) => self.overlay = Some(Overlay::Confirm(Confirm::new(Question::DeleteBook { book_id, title }))),
                edit::Action::ChooseCover => self.overlay = Some(self.file_picker("Choose a cover", files::IMAGES, Purpose::Cover)),
            },
            Screen::Leaf(page, _) => match page.key(key) {
                reader::Action::Nothing => {}
                reader::Action::Back => self.close_leaf(),
                reader::Action::Upload => self.overlay = Some(self.file_picker("Upload a file", files::ANY, Purpose::Upload)),
                reader::Action::Overlay(overlay) => self.overlay = Some(overlay),
                reader::Action::Request(request) => self.spawn(request),
                reader::Action::Notice(notice) => self.notice = Some(notice.to_string()),
            },
            Screen::Settings(page, _) => match page.key(key) {
                settings::Action::Nothing => {}
                settings::Action::Back => self.close_settings(),
                settings::Action::Copy(url) => self.copy(url),
                settings::Action::ResetInvite => self.overlay = Some(Overlay::Confirm(Confirm::new(Question::ResetInvite))),
                settings::Action::Remove(person) => self.overlay = Some(Overlay::Confirm(Confirm::new(Question::RemovePerson(person)))),
                settings::Action::Profile => self.open_profile(),
                settings::Action::SetRole(id, administrator) => {
                    let request = page.set_role(id, administrator);
                    self.spawn(request);
                }
            },
            Screen::Profile(page, _) => match page.key(key) {
                profile::Action::Nothing => {}
                profile::Action::Back => self.close_profile(),
                profile::Action::Save => {
                    if let Some(problem) = page.problem() {
                        self.notice = Some(problem.to_string());
                    } else if let Some(request) = page.save() {
                        self.spawn(request);
                    }
                }
            },
        }
    }

    pub fn open_instance(&mut self, index: usize) {
        if let Some(instance) = self.instances.ready().and_then(|it| it.get(index)) {
            let books = Books::new(instance.url.clone(), instance.alias.clone());
            self.spawn(books.load());
            self.screen = Screen::Books(books);
        }
    }

    /// Open a book, which may not be in the loaded list yet, like a new one.
    pub fn open_book(&mut self, book_id: u64) {
        match self.take_screen() {
            Screen::Books(books) => {
                let page = BookPage::new(books, book_id);
                self.spawn(page.load());
                self.screen = Screen::Book(Box::new(page));
            }
            screen => self.screen = screen,
        }
    }

    /// The screen, with the instances in its place until another is put there.
    fn take_screen(&mut self) -> Screen {
        std::mem::replace(&mut self.screen, Screen::Instances(InstancePicker::new(0)))
    }

    fn close_book(&mut self) {
        match self.take_screen() {
            Screen::Book(page) => {
                self.screen = Screen::Books(page.books);
                self.reload_books();
            }
            screen => self.screen = screen,
        }
    }

    fn reload_books(&self) {
        if let Screen::Books(screen) = &self.screen {
            self.spawn(screen.load());
        }
    }

    /// Open the selected page, section or picture of the book.
    fn open_leaf(&mut self) {
        match self.take_screen() {
            Screen::Book(book) => match book.leaf_page() {
                Some(page) => {
                    self.spawn(page.load());
                    self.screen = Screen::Leaf(Box::new(page), book);
                }
                None => self.screen = Screen::Book(book),
            },
            screen => self.screen = screen,
        }
    }

    /// Back to the book, with the page that was open selected.
    pub fn close_leaf(&mut self) {
        self.return_to_book();
        self.reload_book();
    }

    /// Back from the reader to its book, without reloading the book's pages.
    pub fn return_to_book(&mut self) {
        match self.take_screen() {
            Screen::Leaf(page, mut book) => {
                book.selected = page.index;
                self.screen = Screen::Book(book);
            }
            screen => self.screen = screen,
        }
    }

    fn reload_book(&self) {
        if let Screen::Book(screen) = &self.screen {
            self.spawn(screen.load());
        }
    }

    fn open_settings(&mut self) {
        match self.take_screen() {
            Screen::Books(books) => {
                let page = SettingsPage::new(books.host.clone());
                self.spawn(page.load());
                self.screen = Screen::Settings(Box::new(page), books);
            }
            screen => self.screen = screen,
        }
    }

    fn close_settings(&mut self) {
        self.screen = match self.take_screen() {
            Screen::Settings(_, books) => Screen::Books(books),
            other => other,
        };
    }

    fn open_profile(&mut self) {
        match self.take_screen() {
            Screen::Settings(settings, books) => {
                let page = ProfilePage::new(settings.host.clone());
                self.spawn(page.load());
                self.screen = Screen::Profile(Box::new(page), Box::new(Screen::Settings(settings, books)));
            }
            screen => self.screen = screen,
        }
    }

    /// Back to the settings, with the people reloaded for a changed name.
    fn close_profile(&mut self) {
        self.screen = match self.take_screen() {
            Screen::Profile(_, back) => *back,
            other => other,
        };
        self.load_people();
    }

    fn load_people(&self) {
        if let Screen::Settings(page, _) = &self.screen {
            self.spawn(page.load());
        }
    }

    /// The edit screen for a book, or for a new one without an id.
    fn open_editor(&mut self, book_id: Option<u64>) {
        let books = match &self.screen {
            Screen::Books(screen) => screen,
            Screen::Book(screen) => &screen.books,
            _ => return,
        };

        let page = EditPage::new(books.host.clone(), books.name.clone(), book_id);
        self.spawn(page.load());
        let back = self.take_screen();
        self.screen = Screen::Edit(Box::new(page), Box::new(back));
    }

    /// Back to where the editor was opened, reloading what it may have changed.
    fn close_editor(&mut self, changed: bool) {
        match self.take_screen() {
            Screen::Edit(_, back) => {
                self.screen = *back;

                if changed {
                    self.reload_book();
                    self.reload_books();
                }
            }
            screen => self.screen = screen,
        }
    }

    pub fn books_loaded(&mut self, id: Id, books: Result<Vec<BookCard>>) {
        if let Some(Target::Books(screen)) = self.screen.find(id) {
            screen.loaded(books);

            for url in screen.covers() {
                self.images.want(&screen.host, &url, &self.sender);
            }
        }
    }

    pub fn book_loaded(&mut self, id: Id, book: Result<Book>) {
        if let Some(Target::Book(screen)) = self.screen.find(id) {
            screen.loaded(book);

            for url in screen.pictures() {
                self.images.want(&screen.host, &url, &self.sender);
            }
        }
    }

    /// After a book's pages changed, load it again to show them as they are.
    pub fn book_changed(&mut self, id: Id, result: Result<()>) {
        if let Err(error) = result {
            self.show_error(error);
        }

        if let Some(Target::Book(screen)) = self.screen.find(id) {
            let request = screen.load();
            self.spawn(request);
        }
    }

    pub fn form_loaded(&mut self, id: Id, form: Result<BookForm>) {
        if let Some(Target::Edit(page)) = self.screen.find(id) {
            page.form = Loading::from(form);

            if let Some(cover) = page.form.ready().and_then(|it| it.cover.clone()) {
                self.images.want(&page.host, &cover, &self.sender);
            }
        }
    }

    /// After saving a book, back to it, or on to the new book. The editor is
    /// always the screen on top while it's open.
    pub fn saved(&mut self, id: Id, result: Result<u64>) {
        if let Some(Target::Edit(page)) = self.screen.find(id) {
            page.saving = false;
            let new_book = page.book_id.is_none();

            match result {
                Ok(book_id) if new_book => {
                    self.close_editor(true);
                    self.open_book(book_id);
                }
                Ok(_) => self.close_editor(true),
                Err(error) => self.show_error(error),
            }
        } else if let Err(error) = result {
            self.show_error(error);
        }
    }

    /// After a book is deleted, back to the books it was among.
    pub fn book_deleted(&mut self, id: Id, result: Result<()>) {
        if let Some(Target::Edit(page)) = self.screen.find(id) {
            page.saving = false;

            match result {
                Ok(()) => {
                    self.close_editor(false);
                    self.close_book();
                }
                Err(error) => self.show_error(error),
            }
        } else if let Err(error) = result {
            self.show_error(error);
        }
    }

    pub fn leaf_loaded(&mut self, id: Id, content: Result<LeafContent>) {
        if let Some(Target::Leaf(page)) = self.screen.find(id) {
            page.loaded(content);

            for url in page.images() {
                self.images.want(&page.host, &url, &self.sender);
            }
        }
    }

    /// After saving, show the page as saved.
    pub fn leaf_saved(&mut self, id: Id, result: Result<String>) {
        if let Some(Target::Leaf(page)) = self.screen.find(id) {
            page.saving = false;

            match result {
                Ok(fingerprint) => {
                    page.saved(fingerprint);

                    for url in page.images() {
                        self.images.want(&page.host, &url, &self.sender);
                    }
                }
                Err(error) => self.show_error(error),
            }
        } else if let Err(error) = result {
            self.show_error(error);
        }
    }

    /// Put an uploaded file into the editor of the page it was uploaded for,
    /// if that's still open.
    pub fn uploaded(&mut self, id: Id, result: Result<String>) {
        let editor = match self.screen.find(id) {
            Some(Target::Leaf(page)) => page.upload_finished(),
            _ => None,
        };

        match result {
            Ok(markdown) => match editor {
                Some(editor) => editor.insert_block(&markdown),
                None => self.notice = Some("Uploaded, but the page it was for isn't being edited any more.".to_string()),
            },
            Err(error) => self.show_error(error),
        }
    }

    pub fn people_loaded(&mut self, id: Id, people: Result<People>) {
        if let Some(Target::Settings(page)) = self.screen.find(id) {
            page.loaded(people);
        }
    }

    pub fn people_changed(&mut self, id: Id, result: Result<()>) {
        if let Err(error) = result {
            self.show_error(error);
        }

        if let Some(Target::Settings(page)) = self.screen.find(id) {
            let request = page.load();
            self.spawn(request);
        }
    }

    pub fn invite_reset(&mut self, id: Id, reset: u64, result: Result<String>) {
        match result {
            Ok(url) => {
                if let Some(Target::Settings(page)) = self.screen.find(id) {
                    page.invite_reset(reset, url);
                }
                self.notice = Some("The old invite link no longer works.".to_string());
            }
            Err(error) => self.show_error(error),
        }
    }

    pub fn profile_loaded(&mut self, id: Id, profile: Result<Profile>) {
        if let Some(Target::Profile(page)) = self.screen.find(id) {
            page.loaded(profile);
        }
    }

    pub fn profile_saved(&mut self, id: Id, result: Result<()>) {
        if let Some(Target::Profile(page)) = self.screen.find(id) {
            page.saving = false;

            match result {
                Ok(()) => {
                    page.saved();
                    self.notice = Some("Saved.".to_string());
                }
                Err(error) => self.show_error(error),
            }
        } else if let Err(error) = result {
            self.show_error(error);
        }
    }
}

fn find_books(books: &mut Books, id: Id) -> Option<Target<'_>> {
    if books.id == id { Some(Target::Books(books)) } else { None }
}

fn find_book(page: &mut BookPage, id: Id) -> Option<Target<'_>> {
    if page.id == id { Some(Target::Book(page)) } else { find_books(&mut page.books, id) }
}
