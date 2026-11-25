//! Handing the overlays keys and drawing them over the screen.

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;
use std::path::PathBuf;

use super::add::{self, Add};
use super::confirm::{Confirm, Question};
use super::data::{self, Found, Publication};
use super::files::{self, FilePicker, Picked};
use super::help::Binding;
use super::screens::Target;
use super::search::Search;
use super::{App, Id, Message, Overlay, Purpose, Reply, Screen, instances, publishing, reader, search};

impl Overlay {
    /// The overlay with this id: this one, or the one the quit question is
    /// asked over.
    pub fn find(&mut self, id: Id) -> Option<Target<'_>> {
        match self {
            Overlay::Publishing(publishing) if publishing.id == id => Some(Target::Publishing(publishing)),
            Overlay::Search(search) if search.id == id => Some(Target::Search(search)),
            Overlay::Confirm(Confirm { question: Question::Quit(Some(previous)), .. }) => previous.find(id),
            _ => None,
        }
    }

    /// Whether keys go into a text field, where ? is just a character.
    pub fn typing(&self) -> bool {
        match self {
            Overlay::Add(add) => add.typing(),
            Overlay::Publishing(publishing) => publishing.typing(),
            Overlay::Files(..) | Overlay::Search(_) => true,
            Overlay::Instances(_) | Overlay::Index(_) | Overlay::Confirm(_) => false,
        }
    }

    pub fn bindings(&self) -> Vec<Binding> {
        match self {
            Overlay::Instances(picker) => picker.overlay_bindings(),
            Overlay::Add(add) => add.bindings(),
            Overlay::Publishing(publishing) => publishing.bindings(),
            Overlay::Files(picker, _) => picker.bindings(),
            Overlay::Search(search) => search.bindings(),
            Overlay::Index(index) => index.bindings(),
            Overlay::Confirm(confirm) => confirm.bindings(),
        }
    }
}

impl App {
    pub fn draw_overlay(&mut self, frame: &mut Frame, area: Rect) {
        match &mut self.overlay {
            Some(Overlay::Instances(picker)) => picker.draw(frame, area, &self.instances),
            Some(Overlay::Add(add)) => add.draw(frame, area),
            Some(Overlay::Publishing(publishing)) => publishing.draw(frame, area),
            Some(Overlay::Files(picker, _)) => picker.draw(frame, area, &mut self.images),
            Some(Overlay::Search(search)) => search.draw(frame, area),
            Some(Overlay::Index(index)) => {
                if let Screen::Leaf(page, _) = &self.screen {
                    index.draw(frame, area, page);
                }
            }
            Some(Overlay::Confirm(confirm)) => confirm.draw(frame, area),
            None => {}
        }
    }

    pub fn overlay_key(&mut self, overlay: Overlay, key: KeyEvent) -> Option<Overlay> {
        match overlay {
            Overlay::Instances(mut picker) => {
                let count = self.instances.ready().map(Vec::len).unwrap_or(0);

                match picker.key(key, count) {
                    instances::Action::Nothing => Some(Overlay::Instances(picker)),
                    instances::Action::Close => None,
                    instances::Action::Open(index) => {
                        self.open_instance(index);
                        None
                    }
                }
            }
            Overlay::Add(mut add) => match add.key(key) {
                add::Action::Nothing => Some(Overlay::Add(add)),
                add::Action::Close => None,
                add::Action::ChooseFile => Some(self.file_picker("Choose a picture", files::IMAGES, Purpose::Picture(Box::new(add)))),
                add::Action::Submit => self.submit(add),
            },
            Overlay::Publishing(mut publishing) => match publishing.key(key) {
                publishing::Action::Nothing => Some(Overlay::Publishing(publishing)),
                publishing::Action::Close => None,
                publishing::Action::Copy(url) => {
                    self.copy(url);
                    Some(Overlay::Publishing(publishing))
                }
                publishing::Action::Request(request) => {
                    self.spawn(request);
                    Some(Overlay::Publishing(publishing))
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
            Overlay::Search(mut search) => match search.key(key) {
                search::Action::Nothing => Some(Overlay::Search(search)),
                search::Action::Close => None,
                search::Action::Search(query) => {
                    self.search(&search, query);
                    Some(Overlay::Search(search))
                }
                search::Action::Open(found) => {
                    self.open_found(found);
                    None
                }
            },
            Overlay::Index(mut index) => match &mut self.screen {
                Screen::Leaf(page, _) => match index.key(key, page) {
                    reader::IndexAction::Nothing => Some(Overlay::Index(index)),
                    reader::IndexAction::Close => None,
                    reader::IndexAction::Turned(request) => {
                        self.spawn(request);
                        None
                    }
                },
                _ => None,
            },
            Overlay::Confirm(mut confirm) => match confirm.answer(key) {
                Some(true) => {
                    self.confirmed(confirm.question);
                    None
                }
                Some(false) => match confirm.question {
                    Question::Quit(previous) => previous.map(|it| *it),
                    _ => None,
                },
                None => Some(Overlay::Confirm(confirm)),
            },
        }
    }

    /// Add the page, picture or section once every field is filled in.
    fn submit(&mut self, add: Add) -> Option<Overlay> {
        match add.new_leaf() {
            Some(leaf) => {
                if let Screen::Book(screen) = &self.screen {
                    self.spawn(screen.add(leaf));
                }
                None
            }
            None => {
                self.notice = Some("Fill in every field first.".to_string());
                Some(Overlay::Add(add))
            }
        }
    }

    fn want_preview(&mut self, picker: &FilePicker) {
        if let Some(file) = picker.selected_file() {
            self.images.want_file(&file, &self.sender);
        }
    }

    /// Put the picked file where it was wanted, and go back there.
    fn picked(&mut self, path: PathBuf, purpose: Purpose) -> Option<Overlay> {
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
                add.use_file(&path);
                Some(Overlay::Add(*add))
            }
        }
    }

    fn upload(&mut self, path: PathBuf) {
        if let Screen::Leaf(page, _) = &mut self.screen {
            let uploading = format!("Uploading {}…", path.display());

            match page.upload(path) {
                Some(request) => {
                    self.notice = Some(uploading);
                    self.spawn(request);
                }
                None => self.notice = Some("Files can only go into a page.".to_string()),
            }
        }
    }

    fn search(&self, search: &Search, query: String) {
        if let Some(host) = self.screen.host() {
            let id = search.id;
            let book_id = search.book_id;
            let host = host.to_string();

            self.spawn(move || {
                let found = data::search(&host, book_id, &query);
                Message::To(id, Reply::Found(query, found))
            });
        }
    }

    /// Show the page a search found: select it in this book, or open its book.
    fn open_found(&mut self, found: Found) {
        let Some(book_id) = found.book_id else { return };

        match &mut self.screen {
            Screen::Book(screen) if screen.book_id == book_id => {
                if let Some(leaf_id) = found.leaf_id {
                    screen.select_leaf(leaf_id);
                }
            }
            Screen::Books(_) => {
                self.open_book(book_id);

                if let Screen::Book(screen) = &mut self.screen {
                    screen.pending_leaf = found.leaf_id;
                }
            }
            _ => {}
        }
    }

    fn confirmed(&mut self, question: Question) {
        match question {
            Question::DeleteLeaf { book_id, leaf } => {
                if matches!(&self.screen, Screen::Leaf(page, _) if page.book_id == book_id) {
                    self.close_leaf();
                }

                if let Screen::Book(screen) = &self.screen
                    && screen.book_id == book_id
                {
                    self.spawn(screen.delete(leaf));
                }
            }
            Question::DeleteBook { book_id, .. } => {
                if let Screen::Edit(page, _) = &mut self.screen
                    && page.book_id == Some(book_id)
                {
                    let request = page.delete(book_id);
                    self.spawn(request);
                }
            }
            Question::Discard => {
                if let Screen::Leaf(page, _) = &mut self.screen {
                    page.editor = None;
                }
            }
            Question::RemovePerson(person) => {
                if let Screen::Settings(page, _) = &self.screen {
                    self.spawn(page.remove(person.id));
                }
            }
            Question::ResetInvite => {
                if let Screen::Settings(page, _) = &mut self.screen {
                    let request = page.reset_invite();
                    self.spawn(request);
                }
            }
            Question::Quit(_) => self.done = true,
        }
    }

    pub fn file_picker(&mut self, title: &'static str, kinds: &'static [&'static str], purpose: Purpose) -> Overlay {
        let picker = FilePicker::new(title, kinds, self.files_dir.clone());
        self.want_preview(&picker);
        Overlay::Files(Box::new(picker), purpose)
    }

    pub fn published(&mut self, id: Id, book_page: Id, result: Result<Publication>) {
        if let Some(Target::Publishing(publishing)) = self.overlay.as_mut().and_then(|it| it.find(id)) {
            publishing.finished(&result);
        }

        match result {
            Ok(publication) => {
                if let Some(Target::Book(screen)) = self.screen.find(book_page) {
                    screen.published(publication);
                }
            }
            Err(error) => self.show_error(error),
        }
    }

    pub fn found(&mut self, id: Id, query: String, found: Result<Vec<Found>>) {
        if let Some(Target::Search(search)) = self.overlay.as_mut().and_then(|it| it.find(id)) {
            search.arrived(&query, found);
        }
    }
}
