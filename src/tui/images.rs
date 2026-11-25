//! Covers and pictures, drawn with the terminal's own image protocol (kitty,
//! sixel or iTerm2), or with colored half blocks where there is none, as in
//! tmux and other multiplexers.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use super::Message;
use super::data;

/// How many images are fetched and decoded at once.
const WORKERS: usize = 4;

pub struct Images {
    picker: Picker,
    images: HashMap<Source, Image>,
    /// The images to fetch, for the workers, which start with the first one.
    jobs: Option<Sender<Job>>,
}

/// Where an image comes from: a path or address on an instance's pages, or a
/// file on the disk.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Remote { host: String, path: String },
    File(PathBuf),
}

impl Source {
    pub fn remote(host: &str, path: &str) -> Source {
        Source::Remote { host: host.to_string(), path: path.to_string() }
    }

    pub fn file(path: &Path) -> Source {
        Source::File(path.to_path_buf())
    }
}

struct Job {
    source: Source,
    messages: Sender<Message>,
}

enum Image {
    Loading,
    Ready(Box<StatefulProtocol>),
    Failed,
}

impl Images {
    /// Ask the terminal what it can draw. Must run in raw mode.
    pub fn detect() -> Images {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        release_the_query_reader();

        Images { picker, images: HashMap::new(), jobs: None }
    }

    #[cfg(test)]
    pub fn none() -> Images {
        Images { picker: Picker::halfblocks(), images: HashMap::new(), jobs: None }
    }

    /// Start fetching an image from an instance unless it's here or on its
    /// way. One that failed is tried again.
    pub fn want(&mut self, host: &str, path: &str, messages: &Sender<Message>) {
        let source = Source::remote(host, path);

        if matches!(self.images.get(&source), None | Some(Image::Failed)) {
            self.fetch(source, messages);
        }
    }

    /// Start reading an image from the disk, for a preview before it's
    /// uploaded, unless it's been read already.
    pub fn want_file(&mut self, path: &Path, messages: &Sender<Message>) {
        let source = Source::file(path);

        if !self.images.contains_key(&source) {
            self.fetch(source, messages);
        }
    }

    fn fetch(&mut self, source: Source, messages: &Sender<Message>) {
        self.images.insert(source.clone(), Image::Loading);
        let jobs = self.jobs.get_or_insert_with(start_workers);
        let _ = jobs.send(Job { source, messages: messages.clone() });
    }

    pub fn arrived(&mut self, source: Source, image: anyhow::Result<image::DynamicImage>) {
        let image = match image {
            Ok(image) => Image::Ready(Box::new(self.picker.new_resize_protocol(image))),
            Err(_) => Image::Failed,
        };

        self.images.insert(source, image);
    }

    pub fn failed(&self, source: &Source) -> bool {
        matches!(self.images.get(source), Some(Image::Failed))
    }

    /// Draw the image fitted into the area, centered in it. False when there's nothing to draw yet.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, source: &Source) -> bool {
        self.draw_aligned(frame, area, source, true)
    }

    /// Draw the image fitted into the area, against its left edge.
    pub fn draw_left(&mut self, frame: &mut Frame, area: Rect, source: &Source) -> bool {
        self.draw_aligned(frame, area, source, false)
    }

    fn draw_aligned(&mut self, frame: &mut Frame, area: Rect, source: &Source, centered: bool) -> bool {
        match self.images.get_mut(source) {
            Some(Image::Ready(protocol)) => {
                let size = protocol.size_for(Resize::Fit(None), area.as_size());
                let left = if centered { (area.width.saturating_sub(size.width)) / 2 } else { 0 };
                let centered = Rect {
                    x: area.x + left,
                    y: area.y + (area.height.saturating_sub(size.height)) / 2,
                    width: size.width.min(area.width),
                    height: size.height.min(area.height),
                };
                frame.render_stateful_widget(StatefulImage::default().resize(Resize::Fit(None)), centered, protocol.as_mut());
                true
            }
            _ => false,
        }
    }
}

/// The workers that fetch and decode images, each taking the next job when
/// it's done with one. They stop when the TUI does.
fn start_workers() -> Sender<Job> {
    let (jobs, queue) = channel::<Job>();
    let queue = Arc::new(Mutex::new(queue));

    for _ in 0..WORKERS {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || work(&queue));
    }

    jobs
}

fn work(queue: &Mutex<Receiver<Job>>) {
    while let Some(Job { source, messages }) = next_job(queue) {
        let image = match &source {
            Source::Remote { host, path } => data::image(host, path),
            Source::File(path) => data::image_file(path),
        };
        let _ = messages.send(Message::Image(source, image.map(small)));
    }
}

/// The next job, or none once the TUI is done. The workers wait for one in
/// turn.
fn next_job(queue: &Mutex<Receiver<Job>>) -> Option<Job> {
    let queue = queue.lock().ok()?;
    queue.recv().ok()
}

/// The image query can leave its reader thread waiting on stdin, which then
/// swallows the first key pressed (seen in tmux). Asking for the device
/// attributes, which every terminal answers, gives that thread its answer;
/// when no thread is waiting, crossterm drops the answer.
fn release_the_query_reader() {
    let mut stdout = std::io::stdout();
    let _ = std::io::Write::write_all(&mut stdout, b"\x1b[c");
    let _ = std::io::Write::flush(&mut stdout);
    std::thread::sleep(std::time::Duration::from_millis(50));
}

/// No terminal shows more than this, and full-size photos take seconds to
/// fit to a tile.
fn small(image: image::DynamicImage) -> image::DynamicImage {
    if image.width() > 1200 || image.height() > 1200 {
        image.thumbnail(1200, 1200)
    } else {
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn keeps_the_same_path_on_two_instances_apart() {
        let mut images = Images::none();
        let mine = Source::remote("https://books.example.com", "/u/cover.png");
        let theirs = Source::remote("https://other.example.com", "/u/cover.png");

        images.arrived(mine.clone(), Ok(image::DynamicImage::new_rgb8(4, 4)));
        images.arrived(theirs.clone(), Err(anyhow::anyhow!("not found")));

        let mut terminal = Terminal::new(TestBackend::new(10, 5)).unwrap();
        terminal
            .draw(|frame| {
                assert!(images.draw(frame, frame.area(), &mine));
                assert!(!images.draw(frame, frame.area(), &theirs));
            })
            .unwrap();
        assert!(images.failed(&theirs));
        assert!(!images.failed(&mine));
    }
}
