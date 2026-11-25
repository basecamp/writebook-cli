//! Covers and pictures, drawn with the terminal's own image protocol (kitty,
//! sixel or iTerm2), or with colored half blocks where there is none, as in
//! tmux and other multiplexers.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use super::Message;
use super::data;

pub struct Images {
    picker: Picker,
    images: HashMap<String, Image>,
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

        Images { picker, images: HashMap::new() }
    }

    #[cfg(test)]
    pub fn none() -> Images {
        Images { picker: Picker::halfblocks(), images: HashMap::new() }
    }

    /// Start fetching an image unless it's already here or on its way.
    pub fn want(&mut self, host: &str, path: &str, messages: &Sender<Message>) {
        if !self.images.contains_key(path) {
            self.images.insert(path.to_string(), Image::Loading);

            let (host, path, messages) = (host.to_string(), path.to_string(), messages.clone());
            std::thread::spawn(move || {
                let image = data::image(&host, &path).map(small);
                let _ = messages.send(Message::Image(path, image));
            });
        }
    }

    /// Start reading an image from the disk, for a preview before it's uploaded.
    pub fn want_file(&mut self, path: &std::path::Path, messages: &Sender<Message>) {
        let key = super::files::file_key(path);

        if !self.images.contains_key(&key) {
            self.images.insert(key.clone(), Image::Loading);

            let (path, messages) = (path.to_path_buf(), messages.clone());
            std::thread::spawn(move || {
                let image = image::open(&path).map(small).map_err(anyhow::Error::from);
                let _ = messages.send(Message::Image(key, image));
            });
        }
    }

    pub fn arrived(&mut self, path: String, image: anyhow::Result<image::DynamicImage>) {
        let image = match image {
            Ok(image) => Image::Ready(Box::new(self.picker.new_resize_protocol(image))),
            Err(_) => Image::Failed,
        };

        self.images.insert(path, image);
    }

    pub fn failed(&self, path: &str) -> bool {
        matches!(self.images.get(path), Some(Image::Failed))
    }

    /// Draw the image fitted into the area, centered in it. False when there's nothing to draw yet.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, path: &str) -> bool {
        self.draw_aligned(frame, area, path, true)
    }

    /// Draw the image fitted into the area, against its left edge.
    pub fn draw_left(&mut self, frame: &mut Frame, area: Rect, path: &str) -> bool {
        self.draw_aligned(frame, area, path, false)
    }

    fn draw_aligned(&mut self, frame: &mut Frame, area: Rect, path: &str, centered: bool) -> bool {
        match self.images.get_mut(path) {
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
