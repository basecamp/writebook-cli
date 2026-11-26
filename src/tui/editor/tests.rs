use super::*;
use crate::tui::markdown::{CODE, HEADING};

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
    assert!(
        style_at(text, 2).add_modifier.contains(Modifier::DIM),
        "the opening ** dims"
    );
    assert!(style_at(text, 16).add_modifier.contains(Modifier::ITALIC));
    assert!(style_at(text, 26).add_modifier.contains(Modifier::CROSSED_OUT));
    assert!(!style_at(text, 12).add_modifier.contains(Modifier::BOLD));
}

#[test]
fn leaves_half_typed_markup_and_snake_case_plain() {
    assert_eq!(style_at("a **bold", 4), Style::default());
    assert_eq!(style_at("snake_case_name", 6), Style::default());
    assert_eq!(
        style_at("2 * 3 * 4", 2),
        Style::default(),
        "a star between spaces isn't emphasis"
    );
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
    assert!(shown(&mut editor).contains("Title"), "{}", shown(&mut editor));

    editor.key(press(KeyCode::Enter));
    editor.key(press(KeyCode::Char('!')));
    assert_eq!(editor.body(), "!text");
}

#[test]
fn saves_uploads_and_leaves_with_their_shortcuts() {
    let mut editor = Editor::new("", "");

    assert!(matches!(
        editor.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        Action::Save
    ));
    assert!(matches!(
        editor.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        Action::Upload
    ));
    assert!(matches!(editor.key(press(KeyCode::Esc)), Action::Leave));
}

fn drawn(editor: &mut Editor) -> ratatui::Terminal<ratatui::backend::TestBackend> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(14, 10)).unwrap();
    terminal.draw(|frame| editor.draw(frame, frame.area())).unwrap();
    terminal
}

fn shown(editor: &mut Editor) -> String {
    views::screen_text(&drawn(editor)).join("\n")
}

#[test]
fn wraps_by_the_width_on_screen_and_shows_the_cursor_after_a_full_line() {
    let mut editor = Editor::new("", "abcdefghij\n日本語のテキスト");
    editor.key(press(KeyCode::End));

    let terminal = drawn(&mut editor);
    let text = views::screen_text(&terminal);
    assert_eq!(text[5], "│ abcdefghij │");
    assert_eq!(text[6], "│            │");
    assert!(
        terminal.backend().buffer()[(2, 6)]
            .modifier
            .contains(Modifier::REVERSED),
        "the cursor is on a row of its own"
    );
    assert_eq!(text[7].replace(' ', ""), "│日本語のテ│");
    assert_eq!(text[8].replace(' ', ""), "│キスト│");
}

#[test]
fn ctrl_and_alt_with_a_letter_type_nothing() {
    let mut editor = Editor::new("", "");

    editor.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
    editor.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
    editor.key(KeyEvent::new(
        KeyCode::Char('@'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ));
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
    assert_eq!(
        editor.body(),
        "-\n ",
        "an empty item ends the list only from after its marker"
    );
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
    assert!(shown(&mut editor).contains("A title"), "{}", shown(&mut editor));
}
