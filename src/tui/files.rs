//! Picking a file from the disk: a folder's folders and the files that fit,
//! narrowed by typing, with a preview of the selected image beside them.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::{Path, PathBuf};

use super::help::Binding;
use super::images::Images;
use super::views;

pub const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// No extensions means any file will do.
pub const ANY: &[&str] = &[];

pub struct FilePicker {
    title: &'static str,
    kinds: &'static [&'static str],
    dir: PathBuf,
    entries: Vec<Entry>,
    filter: String,
    selected: usize,
    offset: usize,
    error: Option<String>,
}

struct Entry {
    name: String,
    folder: bool,
}

pub enum Picked {
    Nothing,
    Cancel,
    File(PathBuf),
}

impl FilePicker {
    /// A picker for files with these extensions, starting in the folder.
    pub fn new(title: &'static str, kinds: &'static [&'static str], dir: PathBuf) -> FilePicker {
        let mut picker = FilePicker { title, kinds, dir, entries: Vec::new(), filter: String::new(), selected: 0, offset: 0, error: None };
        picker.read();
        picker
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(&mut self, key: KeyEvent) -> Picked {
        let count = self.visible().len();

        match key.code {
            KeyCode::Esc if !self.filter.is_empty() => self.set_filter(String::new()),
            KeyCode::Esc => return Picked::Cancel,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.selected = (self.selected + 1).min(count.saturating_sub(1)),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => self.selected = (self.selected + 10).min(count.saturating_sub(1)),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = count.saturating_sub(1),
            KeyCode::Enter | KeyCode::Right => return self.open(key.code == KeyCode::Enter),
            KeyCode::Left => self.up(),
            KeyCode::Backspace if self.filter.is_empty() => self.up(),
            KeyCode::Backspace => {
                let mut filter = self.filter.clone();
                filter.pop();
                self.set_filter(filter);
            }
            KeyCode::Char('~') if self.filter.is_empty() => {
                if let Some(home) = dirs::home_dir() {
                    self.go(home, None);
                }
            }
            KeyCode::Char(character) => {
                let filter = format!("{}{character}", self.filter);
                self.set_filter(filter);
            }
            _ => {}
        }

        Picked::Nothing
    }

    /// The selected file, when it's one that fits, for a preview.
    pub fn selected_file(&self) -> Option<PathBuf> {
        let visible = self.visible();
        let entry = visible.get(self.selected)?;
        (!entry.folder).then(|| self.dir.join(&entry.name))
    }

    pub fn bindings(&self) -> Vec<Binding> {
        vec![("type", "find"), ("↑↓", "select"), ("enter", "open"), ("←", "up a folder"), ("~", "home"), ("esc", "cancel")]
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, images: &mut Images) {
        let inside = views::large_window(frame, area, &format!(" {} ", self.title));

        let [location, filter, _, body] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inside);
        frame.render_widget(Paragraph::new(Line::from(shown_path(&self.dir)).dim()), location);
        frame.render_widget(Paragraph::new(Line::from(vec![Span::from("Find: ").dim(), Span::from(format!("{}▏", self.filter))])), filter);

        let preview_room = body.width >= 60;
        let [list, _, preview] = if preview_room {
            Layout::horizontal([Constraint::Percentage(50), Constraint::Length(2), Constraint::Fill(1)]).areas(body)
        } else {
            [body, Rect::default(), Rect::default()]
        };

        self.draw_list(frame, list);

        if preview_room && let Some(file) = self.selected_file() {
            let key = file_key(&file);

            if !images.draw(frame, preview, &key) {
                let note = if images.failed(&key) { "No preview" } else { "Loading the preview…" };
                frame.render_widget(Paragraph::new(Line::from(note).dim()), preview);
            }
        }
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        let height = usize::from(area.height);

        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }

        let lines: Vec<Line> = match &self.error {
            Some(error) => vec![Line::from(error.clone()).red()],
            None if self.visible().is_empty() && self.filter.is_empty() => vec![Line::from("Nothing here fits.").dim()],
            None if self.visible().is_empty() => vec![Line::from("Nothing matches.").dim()],
            None => self
                .visible()
                .iter()
                .enumerate()
                .skip(self.offset)
                .take(height)
                .map(|(index, entry)| {
                    let chosen = index == self.selected;
                    let marker = if chosen { "› " } else { "  " };
                    let name = if entry.folder { format!("{}/", entry.name) } else { entry.name.clone() };
                    let line = Line::from(format!("{marker}{name}"));
                    let line = if entry.folder { line.fg(Color::Blue) } else { line };
                    if chosen { line.bold().reversed() } else { line }
                })
                .collect(),
        };

        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Open a folder, or pick a file with enter.
    fn open(&mut self, picking: bool) -> Picked {
        let visible = self.visible();
        let Some(entry) = visible.get(self.selected) else { return Picked::Nothing };
        let path = self.dir.join(&entry.name);

        if entry.folder {
            self.go(path, None);
            Picked::Nothing
        } else if picking {
            Picked::File(path)
        } else {
            Picked::Nothing
        }
    }

    /// To the folder above, with the one we were in selected.
    fn up(&mut self) {
        let from = self.dir.file_name().map(|it| it.to_string_lossy().into_owned());

        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.go(parent, from);
        }
    }

    fn go(&mut self, dir: PathBuf, select: Option<String>) {
        self.dir = dir;
        self.filter.clear();
        self.read();
        self.selected = select.and_then(|name| self.visible().iter().position(|it| it.name == name)).unwrap_or(0);
    }

    fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selected = 0;
        self.offset = 0;
    }

    /// The folder's folders, then the files that fit, each by name.
    fn read(&mut self) {
        self.entries.clear();
        self.offset = 0;

        let listing = match std::fs::read_dir(&self.dir) {
            Ok(listing) => listing,
            Err(error) => {
                self.error = Some(format!("Can't open {}: {error}", shown_path(&self.dir)));
                return;
            }
        };
        self.error = None;

        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let folder = entry.path().is_dir();

            if folder || self.fits(&name) {
                self.entries.push(Entry { name, folder });
            }
        }

        self.entries.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    }

    fn fits(&self, name: &str) -> bool {
        let extension = Path::new(name).extension().map(|it| it.to_string_lossy().to_lowercase()).unwrap_or_default();
        self.kinds.is_empty() || self.kinds.contains(&extension.as_str())
    }

    /// Hidden entries only show when the filter asks for them with a dot.
    fn visible(&self) -> Vec<&Entry> {
        let filter = self.filter.to_lowercase();

        self.entries
            .iter()
            .filter(|it| !it.name.starts_with('.') || filter.starts_with('.'))
            .filter(|it| it.name.to_lowercase().contains(&filter))
            .collect()
    }
}

/// How a file on the disk is known to the image cache.
pub fn file_key(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn shown_path(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn folder() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("writebook-files-{}-{n}", std::process::id()));
        std::fs::create_dir_all(dir.join("Photos")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for name in ["cover.png", "notes.txt", "Banner.JPG", "Photos/cat.webp"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        dir
    }

    fn names(picker: &FilePicker) -> Vec<&str> {
        picker.visible().iter().map(|it| it.name.as_str()).collect()
    }

    #[test]
    fn lists_folders_first_then_the_files_that_fit() {
        let picker = FilePicker::new("Choose", IMAGES, folder());

        assert_eq!(names(&picker), ["Photos", "Banner.JPG", "cover.png"]);
    }

    #[test]
    fn narrows_the_list_as_you_type_and_shows_hidden_folders_for_a_dot() {
        let mut picker = FilePicker::new("Choose", IMAGES, folder());

        picker.key(press(KeyCode::Char('C')));
        picker.key(press(KeyCode::Char('o')));
        assert_eq!(names(&picker), ["cover.png"]);

        picker.key(press(KeyCode::Esc));
        picker.key(press(KeyCode::Char('.')));
        assert_eq!(names(&picker), [".hidden", "Banner.JPG", "cover.png"]);
    }

    #[test]
    fn opens_folders_goes_back_up_and_picks_a_file() {
        let root = folder();
        let mut picker = FilePicker::new("Choose", IMAGES, root.clone());

        assert!(matches!(picker.key(press(KeyCode::Enter)), Picked::Nothing));
        assert_eq!(picker.dir(), root.join("Photos"));
        assert_eq!(names(&picker), ["cat.webp"]);

        picker.key(press(KeyCode::Backspace));
        assert_eq!(picker.dir(), root);
        assert_eq!(picker.selected_file(), None, "the folder we came from is selected");

        picker.key(press(KeyCode::Down));
        picker.key(press(KeyCode::Down));
        assert!(matches!(picker.key(press(KeyCode::Enter)), Picked::File(path) if path == root.join("cover.png")));
        assert!(matches!(picker.key(press(KeyCode::Esc)), Picked::Cancel));
    }
}
