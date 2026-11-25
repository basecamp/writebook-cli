//! A page's Markdown as it reads in the terminal: headings, emphasis, code,
//! lists, quotes and links styled, the text wrapped to the width, and images
//! as blocks of their own to draw the picture in.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// How tall an image in a page is drawn.
pub const IMAGE_HEIGHT: u16 = 14;

pub enum Row {
    Text(Line<'static>),
    Image { url: String },
}

pub fn render(markdown: &str, width: u16) -> Vec<Row> {
    let mut renderer = Renderer { width: usize::from(width.max(10)), ..Renderer::default() };
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;

    for event in Parser::new_ext(markdown, options) {
        renderer.event(event);
    }

    renderer.flush();
    while matches!(renderer.rows.last(), Some(Row::Text(line)) if line.width() == 0) {
        renderer.rows.pop();
    }
    renderer.rows
}

/// Writebook's code color, and its heading color.
pub const CODE: Color = Color::Yellow;
pub const HEADING: Color = Color::Cyan;

#[derive(Default)]
struct Renderer {
    width: usize,
    rows: Vec<Row>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    /// What starts each line of the current block: list bullets, quote bars.
    prefixes: Vec<String>,
    /// The bullet for the next line, replacing its indent once.
    bullet: Option<String>,
    lists: Vec<Option<u64>>,
    code_block: bool,
    image: Option<String>,
}

impl Renderer {
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.code_block => {
                for line in text.lines() {
                    let prefix = format!("{}  ", self.prefixes.concat());
                    self.rows.push(Row::Text(Line::from(vec![Span::raw(prefix), Span::styled(line.to_string(), Style::default().fg(CODE))])));
                }
            }
            Event::Text(_) if self.image.is_some() => {}
            Event::Text(text) => self.push(&text),
            Event::Code(code) => self.spans.push(Span::styled(code.to_string(), self.style().fg(CODE))),
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.rows.push(Row::Text(Line::styled("─".repeat(self.width.min(40)), Style::default().fg(Color::DarkGray))));
                self.blank();
            }
            Event::TaskListMarker(done) => self.push(if done { "[x] " } else { "[ ] " }),
            Event::Html(html) | Event::InlineHtml(html) => self.push(&html),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                let style = Style::default().fg(HEADING).add_modifier(Modifier::BOLD);
                let style = if level == HeadingLevel::H1 { style.add_modifier(Modifier::UNDERLINED) } else { style };
                self.styles.push(style);
            }
            Tag::Paragraph => self.flush(),
            Tag::BlockQuote(_) => {
                self.flush();
                self.prefixes.push("│ ".to_string());
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.code_block = true;
                if let CodeBlockKind::Fenced(language) = kind
                    && !language.is_empty()
                {
                    self.rows.push(Row::Text(Line::styled(format!("{}  {language}", self.prefixes.concat()), Style::default().fg(Color::DarkGray))));
                }
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let bullet = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let bullet = format!("{number}. ");
                        *number += 1;
                        bullet
                    }
                    _ => "• ".to_string(),
                };
                self.bullet = Some(bullet);
                self.prefixes.push("   ".to_string());
            }
            Tag::Emphasis => self.styles.push(self.style().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(self.style().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.styles.push(self.style().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => self.styles.push(self.style().fg(Color::Blue).add_modifier(Modifier::UNDERLINED)),
            Tag::Image { dest_url, .. } => self.image = Some(dest_url.to_string()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.blank();
            }
            TagEnd::Paragraph => {
                self.flush();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.prefixes.pop();
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.code_block = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => {
                self.flush();
                self.prefixes.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::Image => {
                if let Some(url) = self.image.take() {
                    self.flush();
                    self.rows.push(Row::Image { url });
                    self.blank();
                }
            }
            _ => {}
        }
    }

    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push(&mut self, text: &str) {
        self.spans.push(Span::styled(text.to_string(), self.style()));
    }

    fn blank(&mut self) {
        if !matches!(self.rows.last(), Some(Row::Text(line)) if line.width() == 0) {
            self.rows.push(Row::Text(Line::default()));
        }
    }

    /// Wrap what's gathered into lines of the width, the prefixes in front.
    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }

        let indent = self.prefixes.concat();
        let first = match self.bullet.take() {
            Some(bullet) => format!("{}{bullet}", &indent[..indent.len().saturating_sub(3)]),
            None => indent.clone(),
        };
        let spans = std::mem::take(&mut self.spans);

        for line in wrap(spans, self.width, &first, &indent) {
            self.rows.push(Row::Text(line));
        }
    }
}

/// Break styled text into lines no wider than the width, at spaces when it
/// can, the first line after `first` and the rest after `rest`.
pub fn wrap(spans: Vec<Span<'static>>, width: usize, first: &str, rest: &str) -> Vec<Line<'static>> {
    let prefix_style = Style::default().fg(Color::DarkGray);
    let mut lines = Vec::new();
    let mut line: Vec<Span<'static>> = vec![Span::styled(first.to_string(), prefix_style)];
    let mut used = first.chars().count();

    for span in spans {
        let style = span.style;

        for word in split_keeping_spaces(&span.content) {
            let length = word.chars().count();

            if used + length > width && used > rest.chars().count() {
                lines.push(Line::from(std::mem::take(&mut line)));
                line.push(Span::styled(rest.to_string(), prefix_style));
                used = rest.chars().count();

                if word.trim().is_empty() {
                    continue;
                }
            }

            line.push(Span::styled(word.to_string(), style));
            used += length;
        }
    }

    lines.push(Line::from(line));
    lines
}

/// Words with the spaces after them, so wrapping keeps the spacing.
fn split_keeping_spaces(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;

    for (index, character) in text.char_indices() {
        if character == ' ' {
            parts.push(&text[start..index + 1]);
            start = index + 1;
        }
    }

    if start < text.len() {
        parts.push(&text[start..]);
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Text(line) => line.spans.iter().map(|it| it.content.as_ref()).collect(),
                Row::Image { url } => format!("[image {url}]"),
            })
            .collect()
    }

    #[test]
    fn renders_blocks_and_wraps_paragraphs_to_the_width() {
        let rows = render("# On call\n\nPage the person on call when the site is down.\n\n- First\n- Second\n\n![Diagram](/u/diagram-ab12.png)\n\n> Quoted", 24);

        assert_eq!(
            text(&rows),
            [
                "On call",
                "",
                "Page the person on call ",
                "when the site is down.",
                "",
                "• First",
                "• Second",
                "",
                "[image /u/diagram-ab12.png]",
                "",
                "│ Quoted",
            ]
        );
    }

    #[test]
    fn styles_emphasis_code_and_links_without_their_markers() {
        let rows = render("Some **bold**, *italic*, `code` and a [link](https://example.com).", 80);
        let Row::Text(line) = &rows[0] else { panic!("a line") };
        let style_of = |word: &str| line.spans.iter().find(|it| it.content == word).map(|it| it.style).unwrap();

        assert_eq!(text(&rows), ["Some bold, italic, code and a link."]);
        assert!(style_of("bold").add_modifier.contains(Modifier::BOLD));
        assert!(style_of("italic").add_modifier.contains(Modifier::ITALIC));
        assert_eq!(style_of("code").fg, Some(CODE));
        assert!(style_of("link").add_modifier.contains(Modifier::UNDERLINED));
    }
}
