//! Writing a page in Markdown. Like hey's composer, the Markdown stays as
//! typed and styles itself as it goes: **bold** turns bold, *italic* italic,
//! ~~struck~~ struck through, `code` and # headings take their colors, and the
//! markers themselves dim. A span styles only once its markers pair up on one
//! line, so half-typed markup stays plain.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::markdown::{CODE, HEADING};
use super::views;
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
        Editor { title: title.to_string(), lines, row: 0, column: 0, scroll: 0, on_title: false, changed: false }
    }

    pub fn body(&self) -> String {
        self.lines.iter().map(|line| line.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
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
            KeyCode::Char(_) if !views::plain(key) => {}
            KeyCode::Tab | KeyCode::BackTab => self.on_title = !self.on_title,
            _ if self.on_title => self.title_key(key),
            _ => self.body_key(key),
        }

        Action::Nothing
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

    /// Enter after a list item's marker starts the next item, and on an
    /// empty item ends the list instead.
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

    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let [title, _, body] = Layout::vertical([Constraint::Length(3), Constraint::Length(1), Constraint::Fill(1)]).areas(area);

        let title_text = views::typed("Title", &self.title, self.on_title);
        views::input(frame, title, "Title", title_text.patch_style(Modifier::BOLD), self.on_title);

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

        let shown: Vec<Line> = lines.into_iter().skip(self.scroll).take(height).map(|it| it.line).collect();
        frame.render_widget(Paragraph::new(shown), inside);
    }

    /// Each line styled as Markdown, then cut to the width on screen, with
    /// the cursor drawn where it is. A cursor at the end of a full line goes
    /// on a row of its own, so it shows.
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
                    visual.push(VisualLine { line: Line::from(std::mem::take(&mut spans)), cursor });
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
                    visual.push(VisualLine { line: Line::from(std::mem::take(&mut spans)), cursor });
                }

                spans.push(Span::styled(" ", reversed));
                cursor = true;
            }

            visual.push(VisualLine { line: Line::from(spans), cursor });
        }

        visual
    }
}

struct VisualLine {
    line: Line<'static>,
    cursor: bool,
}

/// A line that's an item of a list: its indent, its marker, and its text.
struct ListItem<'a> {
    indent: &'a str,
    bullet: Option<char>,
    number: Option<u64>,
    text: &'a str,
}

impl ListItem<'_> {

    /// The marker for the item after this one.
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
        return Some(ListItem { indent, bullet: Some(bullet), number: None, text });
    }

    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let number = rest[..digits].parse().ok()?;
    let text = rest[digits..].strip_prefix(". ")?;
    Some(ListItem { indent, bullet: None, number: Some(number), text })
}

/// How each character of a line of Markdown is styled as it's written.
pub fn markdown_styles(line: &[char]) -> Vec<Style> {
    let mut styles = vec![Style::default(); line.len()];
    let marker = Style::default().add_modifier(Modifier::DIM);

    let hashes = line.iter().take_while(|it| **it == '#').count();
    if (1..=6).contains(&hashes) && line.get(hashes) == Some(&' ') {
        for (index, style) in styles.iter_mut().enumerate() {
            *style = if index < hashes { marker } else { Style::default().fg(HEADING).add_modifier(Modifier::BOLD) };
        }
        return styles;
    }

    let in_code = mark_code(line, &mut styles, marker);
    mark_links(line, &mut styles, &in_code, marker);
    mark_pairs(line, &mut styles, &in_code, "**", Modifier::BOLD, marker);
    mark_pairs(line, &mut styles, &in_code, "__", Modifier::BOLD, marker);
    mark_pairs(line, &mut styles, &in_code, "~~", Modifier::CROSSED_OUT, marker);
    mark_pairs(line, &mut styles, &in_code, "*", Modifier::ITALIC, marker);
    mark_pairs(line, &mut styles, &in_code, "_", Modifier::ITALIC, marker);

    if line.first() == Some(&'>') {
        styles[0] = marker;
    }

    styles
}

/// [text](address) and ![alt](address): the text blue and underlined, the
/// brackets and address dimmed, once the whole link is typed.
fn mark_links(line: &[char], styles: &mut [Style], in_code: &[bool], marker: Style) {
    let link = Style::default().fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
    let mut index = 0;

    while index < line.len() {
        if line[index] != '[' || in_code[index] {
            index += 1;
            continue;
        }

        let image = index > 0 && line[index - 1] == '!';
        let Some(close) = (index + 1..line.len()).find(|it| line[*it] == ']') else { break };
        let Some(end) = (line.get(close + 1) == Some(&'(')).then(|| (close + 2..line.len()).find(|it| line[*it] == ')')).flatten() else {
            index += 1;
            continue;
        };

        let start = if image { index - 1 } else { index };
        for (position, style) in styles.iter_mut().enumerate().take(end + 1).skip(start) {
            *style = if position > index && position < close { link } else { marker };
        }

        index = end + 1;
    }
}

/// Backtick pairs on the line, styled as code, their insides left alone by
/// the emphasis that follows.
fn mark_code(line: &[char], styles: &mut [Style], marker: Style) -> Vec<bool> {
    let mut inside = vec![false; line.len()];
    let ticks: Vec<usize> = (0..line.len()).filter(|index| line[*index] == '`').collect();

    for pair in ticks.chunks(2) {
        if let [open, close] = pair {
            for index in *open..=*close {
                inside[index] = true;
                styles[index] = if index == *open || index == *close { marker } else { Style::default().fg(CODE) };
            }
        }
    }

    inside
}

/// Openers and closers of a marker paired left to right: an opener is
/// followed by text, a closer follows text, and the pair styles what's
/// between. Markers already used by a longer one, or in code, don't count.
fn mark_pairs(line: &[char], styles: &mut [Style], in_code: &[bool], delimiter: &str, modifier: Modifier, marker: Style) {
    let delimiter: Vec<char> = delimiter.chars().collect();
    let width = delimiter.len();
    let mut open: Option<usize> = None;
    let mut index = 0;

    while index + width <= line.len() {
        let here = line[index..index + width] == delimiter[..]
            && !in_code[index]
            && styles[index].add_modifier.is_empty()
            && line.get(index + width) != Some(&delimiter[0])
            && (index == 0 || line[index - 1] != delimiter[0]);

        if !here {
            index += 1;
            continue;
        }

        let before = index.checked_sub(1).map(|it| line[it]);
        let after = line.get(index + width).copied();
        let word_edge = delimiter[0] != '_' || (before.is_none_or(|it| !it.is_alphanumeric()) && after.is_none_or(|it| !it.is_alphanumeric()))
            || (before.is_some_and(char::is_alphanumeric) != after.is_some_and(char::is_alphanumeric));

        match open {
            Some(start) if before.is_some_and(|it| !it.is_whitespace()) && word_edge => {
                for (position, style) in styles.iter_mut().enumerate().take(index + width).skip(start) {
                    let is_marker = position < start + width || position >= index;
                    *style = if is_marker { marker } else { style.add_modifier(modifier) };
                }
                open = None;
            }
            None if after.is_some_and(|it| !it.is_whitespace()) && word_edge => open = Some(index),
            _ => {}
        }

        index += width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn styled(text: &str) -> Vec<(char, Style)> {
        let line: Vec<char> = text.chars().collect();
        line.iter().copied().zip(markdown_styles(&line)).collect()
    }

    fn style_at(text: &str, at: usize) -> Style {
        styled(text)[at].1
    }

    #[test]
    fn styles_emphasis_and_dims_its_markers() {
        let text = "a **bold** and *it* or ~~gone~~";

        assert!(style_at(text, 4).add_modifier.contains(Modifier::BOLD));
        assert!(style_at(text, 2).add_modifier.contains(Modifier::DIM), "the opening ** dims");
        assert!(style_at(text, 16).add_modifier.contains(Modifier::ITALIC));
        assert!(style_at(text, 26).add_modifier.contains(Modifier::CROSSED_OUT));
        assert!(!style_at(text, 12).add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn leaves_half_typed_markup_and_snake_case_plain() {
        assert_eq!(style_at("a **bold", 4), Style::default());
        assert_eq!(style_at("snake_case_name", 6), Style::default());
        assert_eq!(style_at("2 * 3 * 4", 2), Style::default(), "a star between spaces isn't emphasis");
    }

    #[test]
    fn styles_code_and_headings() {
        assert_eq!(style_at("run `cargo test` now", 6).fg, Some(CODE));
        assert!(style_at("`*not italic*`", 2).add_modifier.is_empty());
        assert_eq!(style_at("## On call", 4).fg, Some(HEADING));
        assert!(style_at("## On call", 0).add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn styles_links_and_images() {
        let text = "See [the guide](https://example.com) and ![a cat](/u/cat.png)";

        assert_eq!(style_at(text, 5).fg, Some(Color::Blue), "the link's text");
        assert!(style_at(text, 4).add_modifier.contains(Modifier::DIM), "its bracket");
        assert!(style_at(text, 20).add_modifier.contains(Modifier::DIM), "its address");
        assert_eq!(style_at(text, 43).fg, Some(Color::Blue), "the image's alt text");
        assert!(style_at(text, 41).add_modifier.contains(Modifier::DIM), "its !");
        assert_eq!(style_at("a [half typed", 3), Style::default());
    }

    #[test]
    fn continues_a_list_and_ends_it_on_an_empty_item() {
        let mut editor = Editor::new("", "- milk");
        editor.key(press(KeyCode::End));
        editor.key(press(KeyCode::Enter));
        editor.insert("eggs");
        editor.key(press(KeyCode::Enter));
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "- milk\n- eggs\n");

        let mut editor = Editor::new("", "  9. ninth");
        editor.key(press(KeyCode::End));
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "  9. ninth\n  10. ");

        let mut editor = Editor::new("", "*emphasis* isn't a list");
        editor.key(press(KeyCode::End));
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "*emphasis* isn't a list\n");
    }

    #[test]
    fn reads_pages_written_on_the_web_with_their_line_endings() {
        let mut editor = Editor::new("", "# Hello\r\n\r\n- one\r\n");
        assert_eq!(editor.body(), "# Hello\n\n- one\n");

        editor.key(press(KeyCode::Down));
        editor.key(press(KeyCode::Down));
        editor.key(press(KeyCode::End));
        editor.key(press(KeyCode::Enter));
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "# Hello\n\n- one\n\n", "the second enter ends the list");
    }

    #[test]
    fn edits_lines_and_joins_them_back() {
        let mut editor = Editor::new("Title", "one\ntwo");

        editor.key(press(KeyCode::End));
        editor.key(press(KeyCode::Enter));
        editor.insert("new");
        assert_eq!(editor.body(), "one\nnew\ntwo");

        editor.key(press(KeyCode::Home));
        editor.key(press(KeyCode::Backspace));
        assert_eq!(editor.body(), "onenew\ntwo");
        assert!(editor.changed());
    }

    #[test]
    fn puts_an_uploaded_file_on_a_line_of_its_own() {
        let mut editor = Editor::new("", "Before after");
        for _ in 0.."Before".len() {
            editor.key(press(KeyCode::Right));
        }

        editor.insert_block("![a.png](/u/a.png)");

        assert_eq!(editor.body(), "Before\n![a.png](/u/a.png)\n after");
    }

    #[test]
    fn moves_between_the_title_and_the_body() {
        let mut editor = Editor::new("Titl", "text");

        editor.key(press(KeyCode::Up));
        editor.key(press(KeyCode::Char('e')));
        assert_eq!(editor.title, "Title");

        editor.key(press(KeyCode::Enter));
        editor.key(press(KeyCode::Char('!')));
        assert_eq!(editor.body(), "!text");
    }

    #[test]
    fn saves_uploads_and_leaves_with_their_shortcuts() {
        let mut editor = Editor::new("", "");

        assert!(matches!(editor.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)), Action::Save));
        assert!(matches!(editor.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)), Action::Upload));
        assert!(matches!(editor.key(press(KeyCode::Esc)), Action::Leave));
    }

    fn drawn(editor: &mut Editor) -> ratatui::Terminal<ratatui::backend::TestBackend> {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(14, 10)).unwrap();
        terminal.draw(|frame| editor.draw(frame, frame.area())).unwrap();
        terminal
    }

    #[test]
    fn wraps_by_the_width_on_screen_and_shows_the_cursor_after_a_full_line() {
        let mut editor = Editor::new("", "abcdefghij\n日本語のテキスト");
        editor.key(press(KeyCode::End));

        let terminal = drawn(&mut editor);
        let text = views::screen_text(&terminal);
        assert_eq!(text[5], "│ abcdefghij │");
        assert_eq!(text[6], "│            │");
        assert!(terminal.backend().buffer()[(2, 6)].modifier.contains(Modifier::REVERSED), "the cursor is on a row of its own");
        assert_eq!(text[7].replace(' ', ""), "│日本語のテ│");
        assert_eq!(text[8].replace(' ', ""), "│キスト│");
    }

    #[test]
    fn ctrl_and_alt_with_a_letter_type_nothing() {
        let mut editor = Editor::new("", "");

        editor.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        editor.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        editor.key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT));
        editor.key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(editor.body(), "@A");
    }

    #[test]
    fn enter_before_a_list_marker_only_breaks_the_line() {
        let mut editor = Editor::new("", "- milk");
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "\n- milk");

        let mut editor = Editor::new("", "  12. eggs");
        editor.key(press(KeyCode::Right));
        editor.key(press(KeyCode::Right));
        editor.key(press(KeyCode::Right));
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "  1\n2. eggs");

        let mut editor = Editor::new("", "- ");
        editor.key(press(KeyCode::Right));
        editor.key(press(KeyCode::Enter));
        assert_eq!(editor.body(), "-\n ", "an empty item ends the list only from after its marker");
    }

    #[test]
    fn pastes_text_as_it_is_without_starting_list_items() {
        let mut editor = Editor::new("", "- ");
        editor.key(press(KeyCode::End));
        editor.paste("one\r\n- two\n\tthree\x1b[2J");
        assert_eq!(editor.body(), "- one\n- two\n    three[2J");
        assert!(editor.changed());

        editor.key(press(KeyCode::Tab));
        editor.paste("A\ntitle\n");
        assert_eq!(editor.title, "A title");
    }
}
