//! Stars flying towards the viewer behind the instance picker, drawn with
//! braille dots, as in the ONCE installer.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

const STARS: usize = 100;
const SPEED: f64 = 0.03;
const NEAREST: f64 = 0.1;
const FARTHEST: f64 = 3.0;

/// Braille dots by row, top first, for the left and right column of a cell.
const LEFT_DOTS: [u32; 4] = [0x01, 0x02, 0x04, 0x40];
const RIGHT_DOTS: [u32; 4] = [0x08, 0x10, 0x20, 0x80];

#[derive(Default)]
pub struct Starfield {
    area: Rect,
    stars: Vec<Star>,
    seed: u64,
}

#[derive(Clone, Copy)]
struct Star {
    x: f64,
    y: f64,
    z: f64,
}

impl Starfield {
    pub fn new() -> Starfield {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|it| it.as_nanos() as u64).unwrap_or(1);
        Starfield { seed: seed | 1, ..Starfield::default() }
    }

    pub fn step(&mut self) {
        let (center_x, center_y) = self.center();
        let (width, height) = self.dots();

        for index in 0..self.stars.len() {
            let star = &mut self.stars[index];
            star.z -= SPEED;

            let x = center_x + star.x / star.z;
            let y = center_y + star.y / star.z;

            if star.z <= NEAREST || x < 0.0 || x >= width || y < 0.0 || y >= height {
                self.stars[index] = self.random_star();
            }
        }
    }

    pub fn render(&mut self, area: Rect, buffer: &mut Buffer) {
        if area != self.area {
            self.resize(area);
        }

        let (center_x, center_y) = self.center();
        let (width, height) = self.dots();

        for star in &self.stars {
            let x = center_x + star.x / star.z;
            let y = center_y + star.y / star.z;

            if x < 0.0 || x >= width || y < 0.0 || y >= height {
                continue;
            }

            let (x, y) = (x as u16, y as u16);
            let dot = if x % 2 == 0 { LEFT_DOTS[(y % 4) as usize] } else { RIGHT_DOTS[(y % 4) as usize] };
            let color = if star.z < FARTHEST / 2.0 { Color::White } else { Color::DarkGray };
            let cell = &mut buffer[(area.x + x / 2, area.y + y / 4)];

            let existing = cell.symbol().chars().next().map(|it| it as u32).filter(|it| (0x2800..0x2900).contains(it)).unwrap_or(0x2800);
            let symbol = char::from_u32(existing | dot).unwrap_or(' ');
            let brighter = cell.fg == Color::White;

            cell.set_char(symbol);
            cell.set_style(Style::default().fg(if brighter { Color::White } else { color }));
        }
    }

    fn resize(&mut self, area: Rect) {
        self.area = area;
        self.stars = (0..STARS).map(|_| self.random_star()).collect();
    }

    /// The field in braille dots: two across and four down per cell.
    fn dots(&self) -> (f64, f64) {
        (f64::from(self.area.width) * 2.0, f64::from(self.area.height) * 4.0)
    }

    fn center(&self) -> (f64, f64) {
        let (width, height) = self.dots();
        (width / 2.0, height / 2.0)
    }

    fn random_star(&mut self) -> Star {
        let spread = f64::from(self.area.width.max(self.area.height));

        Star {
            x: (self.random() - 0.5) * spread,
            y: (self.random() - 0.5) * spread,
            z: NEAREST + self.random() * (FARTHEST - NEAREST),
        }
    }

    /// xorshift, which is plenty for scattering stars.
    fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / (1u64 << 53) as f64
    }
}
