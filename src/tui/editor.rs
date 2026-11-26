//! Writing a page in Markdown, styled as it's typed.

mod styles;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::views;
use styles::markdown_styles;
use unicode_width::UnicodeWidthChar;

pub struct Editor {
    pub title: String,
    lines: Vec<Vec<char>>,
    row: usize,
    column: usize,
    scroll: usize,
    on_title: bool,
    changed: bool,
}

pub enum Action {
    Nothing,
    Save,
    Upload,
    Leave,
}

impl Editor {
    pub fn new(title: &str, body: &str) -> Editor {
        // Browsers send text with \r\n line endings, so pages written on the
        // web have them. A \r left in a line would move the terminal's cursor.
        let body = body.replace("\r\n", "\n").replace('\r', "\n");
        let lines = body.split('\n').map(|line| line.chars().collect()).collect();
        Editor {
            title: title.to_string(),
            lines,
            row: 0,
            column: 0,
            scroll: 0,
            on_title: false,
            changed: false,
        }
    }

    pub fn body(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Put text in at the cursor.
    pub fn insert(&mut self, text: &str) {
        for character in text.chars() {
            self.type_character(character);
        }
    }

    /// Put pasted text in as it is, without starting list items, in the
    /// title as one line.
    pub fn paste(&mut self, text: &str) {
        if self.on_title {
            self.title.push_str(&views::one_line(text));
        } else {
            let text = text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
            self.insert(&text.replace(|it: char| it.is_control() && it != '\n', ""));
        }

        self.changed = true;
    }

    /// Put a block, like the Markdown for an uploaded file, on a line of
    /// its own at the cursor.
    pub fn insert_block(&mut self, text: &str) {
        if self.column > 0 {
            self.type_character('\n');
        }

        self.insert(text);

        if self.column < self.lines[self.row].len() {
            self.type_character('\n');
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Char('s') if control => return Action::Save,
            KeyCode::Char('u') if control => return Action::Upload,
            KeyCode::Esc => return Action::Leave,
            KeyCode::Char(_) if !views::types_character(key) => {}
            KeyCode::Tab | KeyCode::BackTab => self.on_title = !self.on_title,
            _ if self.on_title => self.title_key(key),
            _ => self.body_key(key),
        }

        Action::Nothing
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let [title, _, body] =
            Layout::vertical([Constraint::Length(3), Constraint::Length(1), Constraint::Fill(1)]).areas(area);

        let title_text = views::typed("Title", &self.title, self.on_title);
        views::input(
            frame,
            title,
            "Title",
            title_text.patch_style(Modifier::BOLD),
            self.on_title,
        );

        let border = if self.on_title { Color::DarkGray } else { Color::Blue };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border))
            .title(Span::from(" Markdown ").style(Style::default().add_modifier(Modifier::DIM)))
            .padding(ratatui::widgets::Padding::horizontal(1));
        let inside = block.inner(body);
        frame.render_widget(block, body);

        let lines = self.visual_lines(usize::from(inside.width.max(1)));
        let height = usize::from(inside.height);
        let cursor = lines.iter().position(|it| it.cursor).unwrap_or(0);

        if cursor < self.scroll {
            self.scroll = cursor;
        } else if cursor >= self.scroll + height {
            self.scroll = cursor + 1 - height;
        }

        let shown: Vec<Line> = lines
            .into_iter()
            .skip(self.scroll)
            .take(height)
            .map(|it| it.line)
            .collect();
        frame.render_widget(Paragraph::new(shown), inside);
    }

    fn title_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(character) => {
                self.title.push(character);
                self.changed = true;
            }
            KeyCode::Backspace => {
                self.title.pop();
                self.changed = true;
            }
            KeyCode::Enter | KeyCode::Down => self.on_title = false,
            _ => {}
        }
    }

    fn body_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(character) => self.type_character(character),
            KeyCode::Enter => self.new_line(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Up if self.row == 0 => self.on_title = true,
            KeyCode::Up => self.move_to(self.row - 1),
            KeyCode::Down => self.move_to((self.row + 1).min(self.lines.len() - 1)),
            KeyCode::PageUp => self.move_to(self.row.saturating_sub(10)),
            KeyCode::PageDown => self.move_to((self.row + 10).min(self.lines.len() - 1)),
            KeyCode::Home => self.column = 0,
            KeyCode::End => self.column = self.lines[self.row].len(),
            _ => {}
        }
    }

    fn type_character(&mut self, character: char) {
        if character == '\n' {
            let rest = self.lines[self.row].split_off(self.column);
            self.lines.insert(self.row + 1, rest);
            self.row += 1;
            self.column = 0;
        } else {
            self.lines[self.row].insert(self.column, character);
            self.column += 1;
        }

        self.changed = true;
    }

    fn new_line(&mut self) {
        let line: String = self.lines[self.row].iter().collect();
        let item = list_item(&line).filter(|it| self.column >= line[..line.len() - it.text.len()].chars().count());

        match item {
            Some(item) if item.text.trim().is_empty() => {
                self.lines[self.row].clear();
                self.column = 0;
                self.changed = true;
            }
            Some(item) => {
                self.type_character('\n');
                self.insert(&item.next_marker());
            }
            None => self.type_character('\n'),
        }
    }

    fn backspace(&mut self) {
        if self.column > 0 {
            self.column -= 1;
            self.lines[self.row].remove(self.column);
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.lines[self.row].len();
            self.lines[self.row].extend(line);
        } else {
            return;
        }

        self.changed = true;
    }

    fn delete(&mut self) {
        if self.column < self.lines[self.row].len() {
            self.lines[self.row].remove(self.column);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].extend(next);
        } else {
            return;
        }

        self.changed = true;
    }

    fn left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.lines[self.row].len();
        }
    }

    fn right(&mut self) {
        if self.column < self.lines[self.row].len() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }

    fn move_to(&mut self, row: usize) {
        self.row = row;
        self.column = self.column.min(self.lines[row].len());
    }

    fn visual_lines(&self, width: usize) -> Vec<VisualLine> {
        let mut visual = Vec::new();
        let reversed = Style::default().add_modifier(Modifier::REVERSED);

        for (row, line) in self.lines.iter().enumerate() {
            let styles = markdown_styles(line);
            let cursor_at = (!self.on_title && row == self.row).then_some(self.column);
            let mut spans: Vec<Span> = Vec::new();
            let mut used = 0;
            let mut cursor = false;

            for (index, character) in line.iter().enumerate() {
                let character_width = character.width().unwrap_or(0);

                if used + character_width > width && !spans.is_empty() {
                    visual.push(VisualLine {
                        line: Line::from(std::mem::take(&mut spans)),
                        cursor,
                    });
                    used = 0;
                    cursor = false;
                }

                if cursor_at == Some(index) {
                    spans.push(Span::styled(character.to_string(), styles[index].patch(reversed)));
                    cursor = true;
                } else {
                    spans.push(Span::styled(character.to_string(), styles[index]));
                }

                used += character_width;
            }

            if cursor_at == Some(line.len()) {
                if used >= width {
                    visual.push(VisualLine {
                        line: Line::from(std::mem::take(&mut spans)),
                        cursor,
                    });
                }

                spans.push(Span::styled(" ", reversed));
                cursor = true;
            }

            visual.push(VisualLine {
                line: Line::from(spans),
                cursor,
            });
        }

        visual
    }
}

struct VisualLine {
    line: Line<'static>,
    cursor: bool,
}

struct ListItem<'a> {
    indent: &'a str,
    bullet: Option<char>,
    number: Option<u64>,
    text: &'a str,
}

impl ListItem<'_> {
    fn next_marker(&self) -> String {
        match (self.bullet, self.number) {
            (Some(bullet), _) => format!("{}{bullet} ", self.indent),
            (None, Some(number)) => format!("{}{}. ", self.indent, number + 1),
            (None, None) => String::new(),
        }
    }
}

fn list_item(line: &str) -> Option<ListItem<'_>> {
    let indent = &line[..line.len() - line.trim_start().len()];
    let rest = &line[indent.len()..];

    if let Some(bullet) = rest.chars().next().filter(|it| matches!(it, '-' | '*' | '+'))
        && let Some(text) = rest[1..].strip_prefix(' ')
    {
        Some(ListItem {
            indent,
            bullet: Some(bullet),
            number: None,
            text,
        })
    } else {
        numbered_item(indent, rest)
    }
}

fn numbered_item<'a>(indent: &'a str, rest: &'a str) -> Option<ListItem<'a>> {
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let number = rest[..digits].parse().ok()?;
    let text = rest[digits..].strip_prefix(". ")?;
    Some(ListItem {
        indent,
        bullet: None,
        number: Some(number),
        text,
    })
}

#[cfg(test)]
mod tests;
