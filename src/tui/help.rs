//! The key bindings at the bottom of the screen, as hey's TUI shows them:
//! blue, key in bold, separated by dots, wrapping onto more lines as needed,
//! with chorded keys last.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub type Binding = (&'static str, &'static str);

const SEPARATOR: &str = " • ";

/// The key that shows and hides the rest, at the right end of the bar. It's
/// dimmed while typing, when ? goes into the text instead.
pub fn toggle(typing: bool) -> Line<'static> {
    let line = Line::from(vec![
        Span::styled("?", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
        Span::styled(" toggle help", Style::default().fg(Color::Blue)),
    ]);

    if typing { line.patch_style(Modifier::DIM) } else { line }
}

/// The bar's lines for this width, or a notice in their place.
pub fn lines(bindings: &[Binding], notice: Option<&str>, width: u16) -> Vec<Line<'static>> {
    if let Some(notice) = notice {
        return vec![Line::styled(notice.to_string(), Style::default().fg(Color::Red))];
    }

    let mut ordered: Vec<Binding> = bindings.iter().filter(|(key, _)| !key.contains('+')).copied().collect();
    ordered.extend(bindings.iter().filter(|(key, _)| key.contains('+')));

    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;

    for (key, description) in ordered {
        let length = key.chars().count() + 1 + description.chars().count();

        if used > 0 && used + SEPARATOR.chars().count() + length > usize::from(width) {
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }

        if used > 0 {
            spans.push(Span::styled(SEPARATOR, Style::default().fg(Color::Blue)));
            used += SEPARATOR.chars().count();
        }

        spans.push(Span::styled(key, Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {description}"), Style::default().fg(Color::Blue)));
        used += length;
    }

    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect()).collect()
    }

    #[test]
    fn puts_chorded_keys_last_and_wraps_to_the_width() {
        let bindings = [("ctrl+i", "instances"), ("enter", "open"), ("v", "list view"), ("q", "quit")];

        assert_eq!(text(&lines(&bindings, None, 80)), ["enter open • v list view • q quit • ctrl+i instances"]);
        assert_eq!(text(&lines(&bindings, None, 30)), ["enter open • v list view", "q quit • ctrl+i instances"]);
    }

    #[test]
    fn shows_a_notice_instead_of_the_keys() {
        assert_eq!(text(&lines(&[("q", "quit")], Some("Copied"), 80)), ["Copied"]);
    }
}
