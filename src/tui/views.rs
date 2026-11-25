//! Drawing the screens and overlays.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use super::data::{Book, BookCard, Instance, Leaf};
use super::images::Images;
use super::{Add, BookPage, Books, Confirm, Loading, Publishing, View, theme};

const BREADCRUMBS_HEIGHT: u16 = 4;
const BOOK_TILE_WIDTH: u16 = 24;
const BOOK_TILE_HEIGHT: u16 = 18;
const LEAF_TILE_WIDTH: u16 = 24;
const LEAF_TILE_HEIGHT: u16 = 14;

/// The rounded window listing the instances, centered in the area.
pub fn instances(frame: &mut Frame, area: Rect, instances: &Loading<Vec<Instance>>, selected: usize) {
    let lines: Vec<Line> = match instances {
        Loading::Pending => vec![Line::from("Loading…").dim()],
        Loading::Failed(error) => vec![Line::from(error.clone()).red()],
        Loading::Ready(instances) if instances.is_empty() => {
            vec![Line::from("No instances yet."), Line::from("Add one with `writebook instance add <url>`.").dim()]
        }
        Loading::Ready(instances) => instances
            .iter()
            .enumerate()
            .flat_map(|(index, instance)| {
                let chosen = index == selected;
                let marker = if chosen { "› " } else { "  " };
                let name = Span::styled(
                    format!("{marker}{}", instance.alias),
                    if chosen { Style::default().fg(Color::Blue).bold() } else { Style::default().bold() },
                );

                [Line::from(name), Line::from(format!("  {} · {}", instance.url, instance.status)).dim()]
            })
            .collect(),
    };

    let width = lines.iter().map(Line::width).max().unwrap_or(0).max(36) as u16 + 6;
    let window = centered(area, width, lines.len() as u16 + 4);

    frame.render_widget(Clear, window);
    frame.render_widget(
        Paragraph::new(lines).block(window_block(" Writebook ").padding(ratatui::widgets::Padding::uniform(1))),
        window,
    );
}

/// "Writebook" centered on the top row. Returns the room below it, inset
/// from the edges.
pub fn masthead(frame: &mut Frame, area: Rect) -> Rect {
    let [top, rest] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from("Writebook").bold()).alignment(Alignment::Center), top);
    rest.inner(ratatui::layout::Margin { horizontal: 2, vertical: 1 })
}

pub fn books(frame: &mut Frame, area: Rect, screen: &mut Books, view: View, images: &mut Images) {
    let [heading, body] = Layout::vertical([Constraint::Length(BREADCRUMBS_HEIGHT), Constraint::Fill(1)]).areas(area);
    breadcrumbs(frame, heading, &[&screen.name, "Books"]);

    let books = match &screen.books {
        Loading::Ready(books) if books.is_empty() => return placeholder(frame, body, "There are no books here yet."),
        Loading::Ready(books) => books,
        Loading::Pending => return placeholder(frame, body, "Loading…"),
        Loading::Failed(error) => return failure(frame, body, error),
    };

    match view {
        View::Gallery => {
            let tiles = grid(body, BOOK_TILE_WIDTH, BOOK_TILE_HEIGHT, books.len(), screen.selected, &mut screen.columns, &mut screen.offset);

            for (index, tile) in tiles {
                let book = &books[index];
                book_tile(frame, tile, book, index == screen.selected, images);
            }
        }
        View::List => {
            let rows: Vec<Row> = books
                .iter()
                .map(|book| Row {
                    left: vec![Span::from(book.title.clone()).bold(), Span::from(byline(book.subtitle.as_deref(), book.author.as_deref())).dim()],
                    right: String::new(),
                    color: theme::color(book.theme.as_deref()),
                    strong: false,
                })
                .collect();
            list(frame, body, &rows, screen.selected, &mut screen.offset);
            screen.columns = 1;
        }
    }
}

pub fn book(frame: &mut Frame, area: Rect, screen: &mut BookPage, view: View, images: &mut Images) {
    let [heading, area] = Layout::vertical([Constraint::Length(BREADCRUMBS_HEIGHT), Constraint::Fill(1)]).areas(area);
    let title = screen.book.ready().map(|it| it.title.as_str()).unwrap_or("…");
    breadcrumbs(frame, heading, &[&screen.books.name, "Books", title]);

    let book = match &screen.book {
        Loading::Ready(book) => book,
        Loading::Pending => return placeholder(frame, area, "Loading…"),
        Loading::Failed(error) => return failure(frame, area, error),
    };

    let [header, body] = Layout::vertical([Constraint::Length(12), Constraint::Fill(1)]).areas(area);

    book_header(frame, header, book, images);

    let leaves = screen.moving.as_ref().map(|it| &it.order).unwrap_or(&book.leaves);
    let color = theme::color(book.theme.as_deref());

    if leaves.is_empty() {
        let lines: &[&str] = if book.editable { &["This book has no pages yet.", "Press \"a\" to add some."] } else { &["This book has no pages yet."] };
        return placeholder_lines(frame, body, lines);
    }

    match view {
        View::Gallery => {
            let tiles = grid(body, LEAF_TILE_WIDTH, LEAF_TILE_HEIGHT, leaves.len(), screen.selected, &mut screen.columns, &mut screen.offset);

            for (index, tile) in tiles {
                let moving = screen.moving.is_some() && index == screen.selected;
                leaf_tile(frame, tile, &leaves[index], index == screen.selected, moving, color, images);
            }
        }
        View::List => {
            let rows: Vec<Row> = leaves
                .iter()
                .map(|leaf| Row {
                    left: vec![Span::from(leaf_title(leaf))],
                    right: leaf.word_count.map(words).unwrap_or_default(),
                    color: if screen.moving.is_some() { Color::Yellow } else { color },
                    strong: leaf.is_section(),
                })
                .collect();
            list(frame, body, &rows, screen.selected, &mut screen.offset);
            screen.columns = 1;
        }
    }
}

pub fn add(frame: &mut Frame, area: Rect, add: &Add) {
    let Some(form) = &add.form else {
        let lines = ["Page", "Picture", "Section"].iter().enumerate().map(|(index, choice)| choice_line(choice, index == add.choice)).collect();
        return window(frame, area, " Add ", lines, 44);
    };

    let height = form.fields.len() as u16 * 3 + 2;
    let window = centered(area, 50, height + 4);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Blue))
        .title(format!(" Add a {} ", form.kind.name()).bold())
        .padding(ratatui::widgets::Padding::uniform(1));
    let inside = block.inner(window);
    frame.render_widget(Clear, window);
    frame.render_widget(block, window);

    let mut rows: Vec<Constraint> = form.fields.iter().map(|_| Constraint::Length(3)).collect();
    rows.extend([Constraint::Length(1), Constraint::Length(1)]);
    let areas = Layout::vertical(rows).split(inside);

    for (index, field) in form.fields.iter().enumerate() {
        let focused = index == form.focus;
        let text = match (field.file, field.value.is_empty()) {
            (true, true) => Line::from("Press enter to choose a file").dim(),
            (true, false) => Line::from(field.value.clone()),
            (false, _) => typed(field.label, &field.value, focused),
        };
        input(frame, areas[index], field.label, text, focused);
    }

    let buttons = buttons(&[("Add", form.focus == form.add_button()), ("Cancel", form.focus == form.cancel_button())]);
    frame.render_widget(Paragraph::new(buttons), areas[form.fields.len() + 1]);
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

/// What's typed in a field, with the cursor when focused, or its label
/// dimmed while it's empty.
pub fn typed(label: &str, value: &str, focused: bool) -> Line<'static> {
    match (value.is_empty(), focused) {
        (true, false) => Line::from(label.to_string()).dim(),
        (_, true) => Line::from(format!("{value}▏")),
        (false, false) => Line::from(value.to_string()),
    }
}

pub fn delete(frame: &mut Frame, area: Rect, leaf: &Leaf, confirm: Confirm) {
    let lines = vec![Line::from(format!("Delete “{}”?", leaf_title(leaf))), Line::from(""), choices(confirm)];
    window(frame, area, " Delete ", lines, 44);
}

pub fn delete_book(frame: &mut Frame, area: Rect, title: &str, confirm: Confirm) {
    let lines = vec![
        Line::from(format!("Delete “{title}” and all its pages?")),
        Line::from("This can't be undone.").dim(),
        Line::from(""),
        choices(confirm),
    ];
    window(frame, area, " Delete book ", lines, 50);
}

pub fn discard(frame: &mut Frame, area: Rect, confirm: Confirm) {
    let lines = vec![Line::from("Throw away your changes to this page?"), Line::from(""), choices(confirm)];
    window(frame, area, " Stop editing ", lines, 50);
}

pub fn remove_person(frame: &mut Frame, area: Rect, name: &str, confirm: Confirm) {
    let lines = vec![Line::from(format!("Remove {name}?")), Line::from("They won't be able to sign in any more.").dim(), Line::from(""), choices(confirm)];
    window(frame, area, " Remove ", lines, 50);
}

pub fn reset_invite(frame: &mut Frame, area: Rect, confirm: Confirm) {
    let lines = vec![Line::from("Make a new invite link?"), Line::from("The current link will stop working.").dim(), Line::from(""), choices(confirm)];
    window(frame, area, " New invite link ", lines, 50);
}

pub fn quit(frame: &mut Frame, area: Rect, confirm: Confirm) {
    let lines = vec![Line::from("Quit writebook tui?"), Line::from(""), choices(confirm)];
    window(frame, area, " Quit ", lines, 44);
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

/// Yes and No side by side, the chosen one filled in.
fn choices(confirm: Confirm) -> Line<'static> {
    buttons(&[("Yes", confirm.yes), ("No", !confirm.yes)])
}

pub fn publishing(frame: &mut Frame, area: Rect, publishing: &Publishing) {
    let mut lines = vec![choice_line("Private", !publishing.published), choice_line("Public", publishing.published), Line::from("")];

    match &publishing.editing {
        Some(slug) => {
            lines.push(Line::from("Address").dim());
            lines.push(Line::from(vec![Span::from(publishing.url_without_slug()).dim(), Span::from(format!("{slug}▏"))]));
        }
        None if publishing.published => lines.push(Line::from(publishing.url.clone()).fg(Color::Cyan)),
        None => lines.push(Line::from("Only people with access can read it.").dim()),
    }

    if publishing.qr && publishing.published {
        lines.push(Line::from(""));
        lines.extend(qr_lines(&publishing.url));
    }

    if publishing.saving {
        lines.push(Line::from(""));
        lines.push(Line::from("Saving…").dim());
    }

    window(frame, area, " Publishing ", lines, 50);
}

struct Row<'a> {
    left: Vec<Span<'a>>,
    right: String,
    color: Color,
    strong: bool,
}

/// Rows with the selected one marked in its color, scrolled to keep it in view.
fn list(frame: &mut Frame, area: Rect, rows: &[Row], selected: usize, offset: &mut usize) {
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
/// one is in view. Remembers the columns for moving up and down.
fn grid(area: Rect, width: u16, height: u16, count: usize, selected: usize, columns: &mut usize, offset: &mut usize) -> Vec<(usize, Rect)> {
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

fn scroll_into_view(offset: &mut usize, selected: usize, visible: usize) {
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + visible {
        *offset = selected + 1 - visible;
    }
}

fn book_tile(frame: &mut Frame, area: Rect, book: &BookCard, selected: bool, images: &mut Images) {
    let color = theme::color(book.theme.as_deref());
    let [cover, text] = Layout::vertical([Constraint::Fill(1), Constraint::Length(4)]).areas(area);
    let inside = selection_box(frame, cover, selected.then_some(color));

    let drawn = match &book.cover {
        Some(cover) => images.draw(frame, inside, &cover.url),
        None => false,
    };

    if !drawn {
        themed_cover(frame, inside, &book.title, book.theme.as_deref());
    }

    let lines = vec![
        Line::from(book.title.clone()).bold(),
        Line::from(book.subtitle.clone().unwrap_or_default()).dim(),
        Line::from(book.author.clone().unwrap_or_default()).italic().dim(),
    ];
    frame.render_widget(Paragraph::new(in_selection_color(lines, selected, color)), beside_gap(text));
}

fn leaf_tile(frame: &mut Frame, area: Rect, leaf: &Leaf, selected: bool, moving: bool, color: Color, images: &mut Images) {
    let border = if moving { Color::Yellow } else { color };
    let [preview, text] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);
    let inside = selection_box(frame, preview, Some(if selected { border } else { Color::DarkGray }));

    let drawn = match (&leaf.image, leaf.is_picture()) {
        (Some(image), true) => images.draw(frame, inside, &image.url),
        _ => false,
    };

    if !drawn {
        let paragraph = Paragraph::new(leaf.preview.clone().unwrap_or_default()).wrap(Wrap { trim: true }).dim();
        let paragraph = if leaf.is_section() { paragraph.alignment(Alignment::Center) } else { paragraph };
        frame.render_widget(paragraph, inside.inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 }));
    }

    let lines = vec![Line::from(leaf_title(leaf)).bold(), Line::from(leaf.word_count.map(words).unwrap_or_default()).dim()];
    frame.render_widget(Paragraph::new(in_selection_color(lines, selected, border)), beside_gap(text));
}

/// A selected tile's text takes the color of its border.
fn in_selection_color(lines: Vec<Line<'static>>, selected: bool, color: Color) -> Vec<Line<'static>> {
    if selected {
        lines.into_iter().map(|line| line.fg(color)).collect()
    } else {
        lines
    }
}

/// The box around a cover or preview: a rounded border in the color, or the
/// same room left blank without one, so nothing shifts as the selection
/// moves. Returns the room inside.
fn selection_box(frame: &mut Frame, area: Rect, border: Option<Color>) -> Rect {
    let area = beside_gap(area);
    let inside = Block::bordered().inner(area);

    if let Some(color) = border {
        frame.render_widget(Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(color)), area);
    }

    inside
}

/// Tiles sit two columns apart.
fn beside_gap(area: Rect) -> Rect {
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

fn book_header(frame: &mut Frame, area: Rect, book: &Book, images: &mut Images) {
    let [cover, _, text] = Layout::horizontal([Constraint::Length(18), Constraint::Length(2), Constraint::Fill(1)]).areas(area);
    let cover = Rect { height: cover.height.saturating_sub(1), ..cover };

    let drawn = match &book.cover {
        Some(path) => images.draw(frame, cover, path),
        None => false,
    };

    if !drawn {
        themed_cover(frame, cover, &book.title, book.theme.as_deref());
    }

    let mut lines = vec![Line::from(book.title.clone()).bold().fg(theme::color(book.theme.as_deref()))];
    lines.extend(book.subtitle.clone().map(Line::from));
    lines.extend(book.author.clone().map(|it| Line::from(it).italic().dim()));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text);
}

/// Where you are, from the instance down, the last step in bold, between two
/// lines, with a blank row below.
pub fn breadcrumbs(frame: &mut Frame, area: Rect, steps: &[&str]) {
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
    let area = Rect { height: 3, ..area };
    frame.render_widget(Paragraph::new(Line::from(spans)).block(lines), area);
}

pub fn window(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line>, width: u16) {
    let window = centered(area, width, lines.len() as u16 + 4);
    frame.render_widget(Clear, window);
    frame.render_widget(
        Paragraph::new(lines).block(window_block(title).padding(ratatui::widgets::Padding::uniform(1))),
        window,
    );
}

fn window_block(title: &str) -> Block<'_> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Blue)).title(title.bold())
}

fn choice_line(label: &str, chosen: bool) -> Line<'static> {
    if chosen {
        Line::from(format!("● {label}")).fg(Color::Blue).bold()
    } else {
        Line::from(format!("○ {label}"))
    }
}

pub fn qr_lines(url: &str) -> Vec<Line<'static>> {
    use qrcode::render::unicode::Dense1x2;

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

fn placeholder_lines(frame: &mut Frame, area: Rect, lines: &[&str]) {
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

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height }
}

fn leaf_title(leaf: &Leaf) -> String {
    if leaf.title.is_empty() { format!("Untitled {}", leaf.kind.to_lowercase()) } else { leaf.title.clone() }
}

fn byline(subtitle: Option<&str>, author: Option<&str>) -> String {
    let parts: Vec<&str> = [subtitle, author].into_iter().flatten().filter(|it| !it.is_empty()).collect();
    if parts.is_empty() { String::new() } else { format!("  {}", parts.join(" · ")) }
}

fn words(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }

    if count == 1 { "1 word".to_string() } else { format!("{grouped} words") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::File;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen_text(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>().trim_end().to_string())
            .collect()
    }

    fn card(id: u64, title: &str, subtitle: Option<&str>) -> BookCard {
        BookCard {
            id,
            title: title.to_string(),
            subtitle: subtitle.map(str::to_string),
            author: Some("Ops".to_string()),
            theme: Some("green".to_string()),
            cover: Some(File { url: "/covers/1".to_string() }),
            editable: true,
        }
    }

    fn books_screen() -> Books {
        Books {
            host: "https://books.example.com".to_string(),
            name: "books.example.com".to_string(),
            books: Loading::Ready(vec![card(1, "Handbook", Some("How we work")), card(2, "Guide", None)]),
            selected: 1,
            columns: 1,
            offset: 0,
        }
    }

    #[test]
    fn frames_only_the_selected_books_cover_and_puts_the_titles_below() {
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut screen = books_screen();
        let mut images = Images::none();

        terminal.draw(|frame| books(frame, frame.area(), &mut screen, View::Gallery, &mut images)).unwrap();
        let text = screen_text(&terminal);
        let buffer = terminal.backend().buffer();

        assert_eq!(screen.columns, 2);
        assert_eq!(text[4], format!("{}╭{}╮", " ".repeat(24), "─".repeat(20)));
        assert_eq!(text[17], format!("{}╰{}╯", " ".repeat(24), "─".repeat(20)));
        assert_eq!(buffer[(24, 4)].fg, Color::Green);
        assert_eq!(text[18..21], ["Handbook                Guide", "How we work", "Ops                     Ops"]);
        assert_eq!(buffer[(24, 18)].fg, Color::Green, "the selected book's title is in its theme's color");
        assert_eq!(buffer[(24, 20)].fg, Color::Green, "and so is its author");
        assert_eq!(buffer[(0, 18)].fg, Color::Reset);

        // Covers that haven't arrived are drawn in the theme's color, with the title.
        assert_eq!(buffer[(1, 5)].bg, Color::Green);
        assert_eq!(&text[6][2..10], "Handbook");
    }

    #[test]
    fn lists_books_with_their_subtitle_and_author() {
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        let mut screen = books_screen();
        let mut images = Images::none();

        terminal.draw(|frame| books(frame, frame.area(), &mut screen, View::List, &mut images)).unwrap();

        assert_eq!(screen_text(&terminal)[..3], ["─".repeat(60), "books.example.com > Books".to_string(), "─".repeat(60)]);
        assert_eq!(
            screen_text(&terminal)[4..6],
            [
                format!("  Handbook  How we work · Ops {}", ".".repeat(29)),
                format!("› Guide  Ops {}", ".".repeat(46)),
            ]
        );
    }

    #[test]
    fn lists_pages_with_their_word_counts_on_the_right() {
        let mut terminal = Terminal::new(TestBackend::new(40, 19)).unwrap();
        let leaf = |id, kind: &str, title: &str, word_count| Leaf {
            id,
            kind: kind.to_string(),
            title: title.to_string(),
            word_count,
            preview: None,
            image: None,
        };
        let mut screen = BookPage {
            host: "https://books.example.com".to_string(),
            book_id: 1,
            book: Loading::Ready(Book {
                id: 1,
                title: "Handbook".to_string(),
                subtitle: None,
                author: Some("Ops".to_string()),
                theme: None,
                cover: None,
                slug: "handbook".to_string(),
                published: false,
                editable: true,
                leaves: vec![leaf(1, "Section", "Part one", None), leaf(2, "Page", "On call", Some(1234)), leaf(3, "Page", "", Some(1))],
            }),
            selected: 1,
            columns: 1,
            offset: 0,
            moving: None,
            pending_leaf: None,
            books: books_screen(),
        };
        let mut images = Images::none();

        terminal.draw(|frame| book(frame, frame.area(), &mut screen, View::List, &mut images)).unwrap();

        let text = screen_text(&terminal);
        assert_eq!(text[1], "books.example.com > Books > Handbook");
        assert_eq!(&text[4][20..], "Handbook", "the cover stays to the left of the title in a list too");
        assert_eq!(&text[5][20..], "Ops");
        assert_eq!(
            text[16..19],
            [
                "  Part one ............................",
                "› On call .................. 1,234 words",
                "  Untitled page ................. 1 word"
            ]
        );
    }
}
