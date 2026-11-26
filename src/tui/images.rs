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
use std::sync::{Arc, Mutex, PoisonError};

use super::Message;
use super::data;

const FETCH_WORKERS: usize = 4;

const IMAGES_KEPT: usize = 64;

pub struct Images {
    picker: Picker,
    images: HashMap<Source, Image>,
    used: HashMap<Source, u64>,
    clock: u64,
    jobs: Option<Sender<Job>>,
    messages: Option<Sender<Message>>,
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
        Source::Remote {
            host: host.to_string(),
            path: path.to_string(),
        }
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
    fn new(picker: Picker) -> Images {
        Images {
            picker,
            images: HashMap::new(),
            used: HashMap::new(),
            clock: 0,
            jobs: None,
            messages: None,
        }
    }

    /// Ask the terminal what it can draw. Needs raw mode.
    pub fn detect() -> Images {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        release_the_query_reader();

        Images::new(picker)
    }

    #[cfg(test)]
    pub fn none() -> Images {
        Images::new(Picker::halfblocks())
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

    pub fn arrived(&mut self, source: Source, image: anyhow::Result<image::DynamicImage>) {
        let image = match image {
            Ok(image) => Image::Ready(Box::new(self.picker.new_resize_protocol(image))),
            Err(_) => Image::Failed,
        };

        self.images.insert(source, image);
        self.drop_the_oldest();
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

    fn fetch(&mut self, source: Source, messages: &Sender<Message>) {
        self.messages = Some(messages.clone());
        self.touch(&source);
        self.images.insert(source.clone(), Image::Loading);
        let jobs = self.jobs.get_or_insert_with(start_workers);
        jobs.send(Job {
            source,
            messages: messages.clone(),
        })
        .expect("the workers wait for jobs until the TUI is done");
    }

    fn touch(&mut self, source: &Source) {
        self.clock += 1;
        self.used.insert(source.clone(), self.clock);
    }

    fn drop_the_oldest(&mut self) {
        while self.images.values().filter(|it| !matches!(it, Image::Loading)).count() > IMAGES_KEPT {
            let oldest = self
                .images
                .iter()
                .filter(|(_, image)| !matches!(image, Image::Loading))
                .min_by_key(|(source, _)| self.used.get(*source))
                .map(|(source, _)| source.clone());

            if let Some(source) = oldest {
                self.images.remove(&source);
                self.used.remove(&source);
            }
        }
    }

    fn draw_aligned(&mut self, frame: &mut Frame, area: Rect, source: &Source, centered: bool) -> bool {
        if !self.images.contains_key(source)
            && let Some(messages) = self.messages.clone()
        {
            self.fetch(source.clone(), &messages);
        } else if self.images.contains_key(source) {
            self.touch(source);
        }

        match self.images.get_mut(source) {
            Some(Image::Ready(protocol)) => {
                let size = protocol.size_for(Resize::Fit(None), area.as_size());
                let left = if centered {
                    (area.width.saturating_sub(size.width)) / 2
                } else {
                    0
                };
                let centered = Rect {
                    x: area.x + left,
                    y: area.y + (area.height.saturating_sub(size.height)) / 2,
                    width: size.width.min(area.width),
                    height: size.height.min(area.height),
                };
                frame.render_stateful_widget(
                    StatefulImage::default().resize(Resize::Fit(None)),
                    centered,
                    protocol.as_mut(),
                );
                true
            }
            _ => false,
        }
    }
}

fn start_workers() -> Sender<Job> {
    let (jobs, queue) = channel::<Job>();
    let queue = Arc::new(Mutex::new(queue));

    for _ in 0..FETCH_WORKERS {
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
        if let Err(unwanted) = messages.send(Message::Image(source, image.map(shrink_to_draw))) {
            drop(unwanted);
        }
    }
}

fn next_job(queue: &Mutex<Receiver<Job>>) -> Option<Job> {
    let queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
    queue.recv().ok()
}

fn release_the_query_reader() {
    let mut stdout = std::io::stdout();
    let asked = std::io::Write::write_all(&mut stdout, b"\x1b[c").and_then(|()| std::io::Write::flush(&mut stdout));

    if asked.is_ok() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn shrink_to_draw(image: image::DynamicImage) -> image::DynamicImage {
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

    #[test]
    fn drops_the_image_drawn_longest_ago_and_fetches_it_again_when_drawn() {
        let mut images = Images::none();
        let (messages, fetched) = channel();
        let paths: Vec<PathBuf> = (0..500)
            .map(|it| PathBuf::from(format!("/nonexistent/{it}.png")))
            .collect();
        let sources: Vec<Source> = paths.iter().map(|it| Source::file(it)).collect();
        let newest = sources.len() - 1;
        let oldest_kept = sources.len() - IMAGES_KEPT;

        for path in &paths {
            images.want_file(path, &messages);
            images.arrived(Source::file(path), Ok(image::DynamicImage::new_rgb8(4, 4)));
        }

        let mut terminal = Terminal::new(TestBackend::new(10, 5)).unwrap();
        terminal
            .draw(|frame| {
                assert!(images.draw(frame, frame.area(), &sources[oldest_kept]));
                assert!(!images.draw(frame, frame.area(), &sources[0]));
                assert!(images.draw(frame, frame.area(), &sources[newest]));
            })
            .unwrap();

        let fetches = fetched
            .iter()
            .take(paths.len() + 1)
            .filter(|it| matches!(it, Message::Image(source, _) if *source == sources[0]))
            .count();
        assert_eq!(fetches, 2);
    }
}
