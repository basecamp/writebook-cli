//! Widgets the screens and overlays share: the masthead, breadcrumbs, fields,
//! buttons, windows, and lists and grids of tiles.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use super::clipboard;
use super::data::Leaf;
use super::theme;

/// Books and pages show as a gallery of tiles or as a list.
#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Gallery,
    List,
}

impl View {
    pub fn toggled(self) -> View {
        match self {
            View::Gallery => View::List,
            View::List => View::Gallery,
        }
    }

    /// The arrows that move the selection: all four in a grid, up and down in a list.
    pub fn arrows(self) -> &'static str {
        match self {
            View::Gallery => "←↑↓→",
            View::List => "↑↓",
        }
    }

    /// What toggling switches to.
    pub fn other_name(self) -> &'static str {
        match self {
            View::Gallery => "list view",
            View::List => "gallery view",
        }
    }
}

/// "Writebook" centered on the top row. Returns the room below it, inset
/// from the edges.
pub fn masthead(frame: &mut Frame, area: Rect) -> Rect {
    let [top, rest] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from("Writebook").bold()).alignment(Alignment::Center), top);
    rest.inner(ratatui::layout::Margin { horizontal: 2, vertical: 1 })
}

/// A field framed with a rounded border and its label on the border, blue
/// when focused, as on the book form.
pub fn input(frame: &mut Frame, area: Rect, label: &str, text: Line<'static>, focused: bool) {
    let border = if focused { Color::Blue } else { Color::DarkGray };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::from(format!(" {label} ")).dim())
        .padding(ratatui::widgets::Padding::horizontal(1));

    frame.render_widget(Paragraph::new(text).block(block), area);
}

/// Whether a key types its character: not with ctrl or alt held, unless both
/// are, which is how Windows sends AltGr.
pub fn plain(key: KeyEvent) -> bool {
    let held = key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT);
    held.is_empty() || held == KeyModifiers::CONTROL | KeyModifiers::ALT
}

/// Pasted text for a field of one line: its lines joined by spaces, without
/// anything that would move the terminal's cursor.
pub fn one_line(text: &str) -> String {
    text.split(char::is_control).filter(|it| !it.is_empty()).collect::<Vec<_>>().join(" ")
}

/// What's typed in a field, with the cursor when focused, or its label
/// dimmed while it's empty.
pub fn typed(label: &str, value: &str, focused: bool) -> Line<'static> {
    match (value.is_empty(), focused) {
        (true, false) => Line::from(label.to_string()).dim(),
        (_, true) => Line::from(format!("{value}▏")),
        (false, false) => Line::from(value.to_string()),
    }
}

/// Buttons side by side and centered, the focused one filled in.
pub fn buttons(labels: &[(&'static str, bool)]) -> Line<'static> {
    let mut spans = Vec::new();

    for (index, (label, focused)) in labels.iter().enumerate() {
        if index > 0 {
            spans.push(Span::from("   "));
        }

        if *focused {
            spans.push(Span::styled(format!(" {label} "), Style::default().fg(Color::Black).bg(Color::Blue).bold()));
        } else {
            spans.push(Span::from(format!(" {label} ")).dim());
        }
    }

    Line::from(spans).centered()
}

pub struct Row<'a> {
    pub left: Vec<Span<'a>>,
    pub right: String,
    pub color: Color,
    pub strong: bool,
}

/// Rows with the selected one marked in its color, scrolled to keep it in view.
pub fn list(frame: &mut Frame, area: Rect, rows: &[Row], selected: usize, offset: &mut usize) {
    let height = usize::from(area.height);
    scroll_into_view(offset, selected, height);

    for (index, row) in rows.iter().enumerate().skip(*offset).take(height) {
        let line_area = Rect { y: area.y + (index - *offset) as u16, height: 1, ..area };
        let chosen = index == selected;
        let marker = Span::styled(if chosen { "› " } else { "  " }, Style::default().fg(row.color).bold());

        let mut left = vec![marker];
        left.extend(row.left.iter().cloned().map(|span| {
            let span = if chosen { span.fg(row.color).bold() } else { span };
            if row.strong { span.bold() } else { span }
        }));

        let used = left.iter().map(Span::width).sum::<usize>() + row.right.chars().count();
        let room = usize::from(line_area.width).saturating_sub(used + 2);

        left.push(Span::from(format!(" {} ", ".".repeat(room))).fg(Color::DarkGray));
        left.push(Span::from(row.right.clone()).dim());
        frame.render_widget(Paragraph::new(Line::from(left)), line_area);
    }
}

/// Lay tiles out in rows, as many to a row as fit, scrolled so the selected
/// one is in view. Remembers the columns for moving up and down. No room,
/// no tiles.
pub fn grid(area: Rect, width: u16, height: u16, count: usize, selected: usize, columns: &mut usize, offset: &mut usize) -> Vec<(usize, Rect)> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let height = height.min(area.height);
    *columns = usize::from((area.width / width).max(1));
    let visible_rows = usize::from((area.height / height).max(1));
    scroll_into_view(offset, selected / *columns, visible_rows);

    (0..count)
        .filter(|index| (*offset..*offset + visible_rows).contains(&(index / *columns)))
        .map(|index| {
            let row = (index / *columns - *offset) as u16;
            let column = (index % *columns) as u16;
            let tile = Rect { x: area.x + column * width, y: area.y + row * height, width, height };
            (index, tile.intersection(area))
        })
        .collect()
}

/// Move the selection with the arrows: by one across, by a row up and down.
pub fn navigate(key: KeyEvent, selected: &mut usize, columns: usize, count: usize) {
    if count == 0 {
        return;
    }

    match key.code {
        KeyCode::Left | KeyCode::Char('h') => *selected = selected.saturating_sub(1),
        KeyCode::Right | KeyCode::Char('l') => *selected = (*selected + 1).min(count - 1),
        KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(columns),
        KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + columns).min(count - 1),
        _ => {}
    }
}

/// Scroll a list showing `visible` items so the selected one is among them.
pub fn scroll_into_view(offset: &mut usize, selected: usize, visible: usize) {
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + visible {
        *offset = selected + 1 - visible;
    }
}

/// A selected tile's text takes the color of its border.
pub fn in_selection_color(lines: Vec<Line<'static>>, selected: bool, color: Color) -> Vec<Line<'static>> {
    if selected {
        lines.into_iter().map(|line| line.fg(color)).collect()
    } else {
        lines
    }
}

/// The box around a cover or preview: a rounded border in the color, or the
/// same room left blank without one, so nothing shifts as the selection
/// moves. Returns the room inside.
pub fn selection_box(frame: &mut Frame, area: Rect, border: Option<Color>) -> Rect {
    let area = beside_gap(area);
    let inside = Block::bordered().inner(area);

    if let Some(color) = border {
        frame.render_widget(Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(color)), area);
    }

    inside
}

/// Tiles sit two columns apart.
pub fn beside_gap(area: Rect) -> Rect {
    Rect { width: area.width.saturating_sub(2), ..area }
}

/// What Writebook shows for a book without a cover: the title on the theme's color.
pub fn themed_cover(frame: &mut Frame, area: Rect, title: &str, theme: Option<&str>) {
    let color = theme::color(theme);
    let ink = if color == Color::White { Color::Black } else { Color::White };

    frame.render_widget(
        Paragraph::new(Line::from(title.to_string()).bold().fg(ink))
            .wrap(Wrap { trim: true })
            .block(Block::new().padding(ratatui::widgets::Padding::uniform(1)))
            .style(Style::default().bg(color)),
        area,
    );
}

/// Where you are, from the instance down, the last step in bold, between two
/// lines, with a blank row below. Returns the room below them.
pub fn breadcrumbs(frame: &mut Frame, area: Rect, steps: &[&str]) -> Rect {
    let [heading, rest] = Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(area);
    let mut spans = Vec::new();

    for (index, step) in steps.iter().enumerate() {
        if index > 0 {
            spans.push(Span::from(" > ").dim());
        }

        if index == steps.len() - 1 {
            spans.push(Span::from(step.to_string()).bold());
        } else {
            spans.push(Span::from(step.to_string()).dim());
        }
    }

    let lines = Block::new().borders(Borders::TOP | Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(Paragraph::new(Line::from(spans)).block(lines), Rect { height: 3, ..heading });
    rest
}

pub fn window(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, width: u16) {
    let window = centered(area, width, lines.len() as u16 + 4);
    frame.render_widget(Clear, window);
    frame.render_widget(
        Paragraph::new(lines).block(window_block(title).padding(ratatui::widgets::Padding::uniform(1))),
        window,
    );
}

/// A window filling most of the area, like the search and the file picker.
/// Returns the room inside.
pub fn large_window(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    let window = Rect {
        x: area.x + area.width / 12,
        y: area.y + area.height / 10,
        width: area.width - area.width / 6,
        height: area.height - area.height / 5,
    };
    let block = window_block(title).padding(ratatui::widgets::Padding::horizontal(1));
    let inside = block.inner(window);

    frame.render_widget(Clear, window);
    frame.render_widget(block, window);
    inside
}

fn window_block(title: &str) -> Block<'_> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Blue)).title(title.bold())
}

pub fn choice_line(label: &str, chosen: bool) -> Line<'static> {
    if chosen {
        Line::from(format!("● {label}")).fg(Color::Blue).bold()
    } else {
        Line::from(format!("○ {label}"))
    }
}

/// A QR code for a link on the instance at `host`, or why there's none.
pub fn qr_lines(url: &str, host: &str) -> Vec<Line<'static>> {
    use qrcode::render::unicode::Dense1x2;

    if let Err(error) = clipboard::shareable(url, host) {
        return vec![Line::from(error.to_string()).red()];
    }

    match qrcode::QrCode::new(url) {
        Ok(code) => code
            .render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .quiet_zone(true)
            .build()
            .lines()
            .map(|line| Line::from(line.to_string()))
            .collect(),
        Err(_) => vec![Line::from("The address is too long for a QR code.").dim()],
    }
}

pub fn placeholder(frame: &mut Frame, area: Rect, text: &str) {
    placeholder_lines(frame, area, &[text]);
}

pub fn placeholder_lines(frame: &mut Frame, area: Rect, lines: &[&str]) {
    let lines: Vec<Line> = lines.iter().map(|it| Line::from(it.to_string()).dim()).collect();
    let height = lines.len() as u16;
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), centered(area, area.width, height));
}

pub fn failure(frame: &mut Frame, area: Rect, error: &str) {
    frame.render_widget(
        Paragraph::new(Line::from(error.to_string()).red()).alignment(Alignment::Center).wrap(Wrap { trim: true }),
        centered(area, area.width.min(80), 3),
    );
}

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height }
}

pub fn leaf_title(leaf: &Leaf) -> String {
    if leaf.title.is_empty() { format!("Untitled {}", leaf.kind.to_lowercase()) } else { leaf.title.clone() }
}

#[cfg(test)]
pub fn screen_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>().trim_end().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn pasted_lines_join_into_one_without_control_characters() {
        assert_eq!(one_line("~/covers/\r\nsea.png\n"), "~/covers/ sea.png");
        assert_eq!(one_line("a\x1b[2Jb"), "a [2Jb");
    }

    #[test]
    fn draws_a_qr_code_only_for_a_link_on_the_instance() {
        assert!(qr_lines("https://books.example.com/join/abc", "https://books.example.com").len() > 10);
        assert_eq!(
            qr_lines("https://evil.example.org/join/abc", "https://books.example.com"),
            [Line::from("Writebook's link goes to another site, so it isn't shared").red()]
        );
    }

    #[test]
    fn moves_through_a_grid_by_one_across_and_a_row_up_and_down() {
        let mut selected = 0;

        navigate(press(KeyCode::Right), &mut selected, 3, 7);
        assert_eq!(selected, 1);
        navigate(press(KeyCode::Down), &mut selected, 3, 7);
        assert_eq!(selected, 4);
        navigate(press(KeyCode::Down), &mut selected, 3, 7);
        assert_eq!(selected, 6, "the last row is short, so down stops at the last tile");
        navigate(press(KeyCode::Right), &mut selected, 3, 7);
        assert_eq!(selected, 6);
        navigate(press(KeyCode::Up), &mut selected, 3, 7);
        assert_eq!(selected, 3);
        navigate(press(KeyCode::Left), &mut selected, 3, 7);
        assert_eq!(selected, 2);
    }
}
