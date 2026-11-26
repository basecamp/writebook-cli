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
    offset: usize,
}

pub enum Action {
    Nothing,
    Open(usize),
    Close,
}

impl InstancePicker {
    pub fn new(selected: usize) -> InstancePicker {
        InstancePicker { selected, offset: 0 }
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

    /// The rounded window listing the instances, centered in the area, as
    /// many as fit, two lines each, around the selected one.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, instances: &Loading<Vec<Instance>>) {
        let lines: Vec<Line> = match instances {
            Loading::Pending => vec![Line::from("Loading…").dim()],
            Loading::Failed(error) => vec![Line::from(error.clone()).red()],
            Loading::Ready(instances) if instances.is_empty() => {
                vec![
                    Line::from("No instances yet."),
                    Line::from("Add one with `writebook instance add <url>`.").dim(),
                ]
            }
            Loading::Ready(instances) => {
                let fit = (usize::from(area.height.saturating_sub(4)) / 2).max(1);
                views::scroll_into_view(&mut self.offset, self.selected, fit);

                instances
                    .iter()
                    .enumerate()
                    .skip(self.offset)
                    .take(fit)
                    .flat_map(|(index, instance)| instance_lines(instance, index == self.selected))
                    .collect()
            }
        };

        let width = lines.iter().map(Line::width).max().unwrap_or(0).max(36) as u16 + 6;
        views::window(frame, area, " Writebook ", lines, width);
    }
}

fn instance_lines(instance: &Instance, chosen: bool) -> [Line<'static>; 2] {
    let marker = if chosen { "› " } else { "  " };
    let style = if chosen {
        Style::default().fg(Color::Blue).bold()
    } else {
        Style::default().bold()
    };
    let name = Span::styled(format!("{marker}{}", instance.alias), style);

    [
        Line::from(name),
        Line::from(format!("  {} · {}", instance.url, instance.status)).dim(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn keeps_the_selected_instance_in_view() {
        let instances: Vec<Instance> = (1..=20)
            .map(|number| Instance {
                alias: format!("books{number}"),
                url: format!("https://books{number}.example.com"),
                status: "signed in".to_string(),
            })
            .collect();
        let mut picker = InstancePicker::new(0);

        for _ in 0..19 {
            picker.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), instances.len());
        }

        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 16)).unwrap();
        terminal
            .draw(|frame| picker.draw(frame, frame.area(), &Loading::Ready(instances)))
            .unwrap();
        assert!(views::screen_text(&terminal).iter().any(|it| it.contains("› books20")));
    }
}
