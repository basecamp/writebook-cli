//! A page's Markdown as it reads in the terminal: headings, emphasis, code,
//! lists, quotes and links styled, the text wrapped to the width, and images
//! as blocks of their own to draw the picture in.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    /// What starts each line of the current block: list indents, quote bars.
    prefixes: Vec<String>,
    /// The bullets for the next line, each in place of its item's prefix,
    /// by the prefix's position.
    bullets: Vec<(usize, String)>,
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
                self.bullets.push((self.prefixes.len(), bullet));
                self.prefixes.push("   ".to_string());
            }
            Tag::Emphasis => self.styles.push(self.style().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(self.style().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.styles.push(self.style().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => self.styles.push(self.style().fg(Color::Blue).add_modifier(Modifier::UNDERLINED)),
            Tag::Image { dest_url, .. } => self.image = Some(dest_url.to_string()),
            Tag::Table(_) => self.flush(),
            Tag::TableHead => self.styles.push(self.style().add_modifier(Modifier::BOLD)),
            Tag::TableCell if !self.spans.is_empty() => self.spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray))),
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
            TagEnd::TableHead => {
                self.flush();
                self.styles.pop();
            }
            TagEnd::TableRow => self.flush(),
            TagEnd::Table => self.blank(),
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
        let mut first = self.prefixes.clone();

        for (position, bullet) in self.bullets.drain(..) {
            first[position] = bullet;
        }

        let first = first.concat();
        let spans = std::mem::take(&mut self.spans);

        for line in wrap(spans, self.width, &first, &indent) {
            self.rows.push(Row::Text(line));
        }
    }
}

/// Break styled text into lines no wider than the width on screen, at spaces
/// when it can and inside words too long for a line, the first line after
/// `first` and the rest after `rest`.
pub fn wrap(spans: Vec<Span<'static>>, width: usize, first: &str, rest: &str) -> Vec<Line<'static>> {
    let prefix_style = Style::default().fg(Color::DarkGray);
    let mut lines = Vec::new();
    let mut line: Vec<Span<'static>> = vec![Span::styled(first.to_string(), prefix_style)];
    let mut used = first.width();
    let room = width.saturating_sub(rest.width()).max(1);

    for span in spans {
        let style = span.style;

        for word in split_keeping_spaces(&span.content) {
            for piece in pieces(word, room) {
                let length = piece.width();

                if used + length > width && used > rest.width() {
                    lines.push(Line::from(std::mem::take(&mut line)));
                    line.push(Span::styled(rest.to_string(), prefix_style));
                    used = rest.width();

                    if piece.trim().is_empty() {
                        continue;
                    }
                }

                line.push(Span::styled(piece.to_string(), style));
                used += length;
            }
        }
    }

    lines.push(Line::from(line));
    lines
}

/// A word in pieces no wider than the room, or whole when it fits.
fn pieces(word: &str, room: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut used = 0;

    for (index, character) in word.char_indices() {
        let width = character.width().unwrap_or(0);

        if used + width > room && index > start {
            pieces.push(&word[start..index]);
            start = index;
            used = 0;
        }

        used += width;
    }

    pieces.push(&word[start..]);
    pieces
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
    fn puts_the_bullets_of_nested_items_and_quotes_in_items_on_their_first_line() {
        assert_eq!(text(&render("- > Quoted in a list\n  > still quoted", 40)), ["• │ Quoted in a list still quoted"]);
        assert_eq!(text(&render("- - inner\n  - next\n- outer", 40)), ["• • inner", "   • next", "• outer"]);
        assert_eq!(text(&render("1. > a quote that wraps here", 14)), ["1. │ a quote ", "   │ that ", "   │ wraps ", "   │ here"]);
    }

    #[test]
    fn wraps_by_the_width_on_screen_and_breaks_words_too_long_for_a_line() {
        assert_eq!(text(&render("日本語のテキスト です", 10)), ["日本語のテ", "キスト ", "です"]);
        assert_eq!(text(&render("see https://example.com/a/very/long/path ok", 12)), ["see ", "https://exam", "ple.com/a/ve", "ry/long/path", "ok"]);

        let rows = render("- **abcdefghijklmnop**", 10);
        assert_eq!(text(&rows), ["• abcdefg", "   hijklmn", "   op"]);
        let Row::Text(line) = &rows[1] else { panic!("a line") };
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn renders_a_table_a_row_to_a_line_with_its_cells_apart() {
        let rows = render("| Name | Role |\n|------|------|\n| Ann | Editor |\n| Bob | Reader |\n\nAfter", 40);

        assert_eq!(text(&rows), ["Name │ Role", "Ann │ Editor", "Bob │ Reader", "", "After"]);
        let Row::Text(head) = &rows[0] else { panic!("a line") };
        assert!(head.spans[1].style.add_modifier.contains(Modifier::BOLD));
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
