//! The instances to pick from: the first screen, and a window over the others.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};

use super::Loading;
use super::data::Instance;
use super::help::Binding;
use super::views;

pub struct InstancePicker {
    pub selected: usize,
}

pub enum Action {
    Nothing,
    Open(usize),
    Close,
}

impl InstancePicker {
    pub fn new(selected: usize) -> InstancePicker {
        InstancePicker { selected }
    }

    pub fn key(&mut self, key: KeyEvent, count: usize) -> Action {
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Enter => return Action::Open(self.selected),
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(count.saturating_sub(1)),
            _ => {}
        }

        Action::Nothing
    }

    pub fn bindings(&self) -> Vec<Binding> {
        vec![("↑↓", "select"), ("enter", "open"), ("ctrl+c", "quit")]
    }

    pub fn overlay_bindings(&self) -> Vec<Binding> {
        vec![("↑↓", "select"), ("enter", "open"), ("esc", "close")]
    }

    /// The rounded window listing the instances, centered in the area.
    pub fn draw(&self, frame: &mut Frame, area: Rect, instances: &Loading<Vec<Instance>>) {
        let lines: Vec<Line> = match instances {
            Loading::Pending => vec![Line::from("Loading…").dim()],
            Loading::Failed(error) => vec![Line::from(error.clone()).red()],
            Loading::Ready(instances) if instances.is_empty() => {
                vec![Line::from("No instances yet."), Line::from("Add one with `writebook instance add <url>`.").dim()]
            }
            Loading::Ready(instances) => instances.iter().enumerate().flat_map(|(index, instance)| instance_lines(instance, index == self.selected)).collect(),
        };

        let width = lines.iter().map(Line::width).max().unwrap_or(0).max(36) as u16 + 6;
        views::window(frame, area, " Writebook ", lines, width);
    }
}

fn instance_lines(instance: &Instance, chosen: bool) -> [Line<'static>; 2] {
    let marker = if chosen { "› " } else { "  " };
    let style = if chosen { Style::default().fg(Color::Blue).bold() } else { Style::default().bold() };
    let name = Span::styled(format!("{marker}{}", instance.alias), style);

    [Line::from(name), Line::from(format!("  {} · {}", instance.url, instance.status)).dim()]
}
