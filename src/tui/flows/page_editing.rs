use crossterm::event::KeyCode;
use serde_json::json;
use std::sync::{Arc, Mutex};

use super::{Gate, Tui, body, json, not_found, reply};
use crate::scratch::TempDir;

#[derive(Clone)]
enum Save {
    Works,
    Conflicts,
    Fails,
    Slow(Gate),
}

fn handbook(save: Save) -> Tui {
    let saves = Arc::new(Mutex::new(0));

    Tui::new(move |request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([
            { "id": 1, "title": "Handbook", "editable": true },
            { "id": 2, "title": "Recipes", "editable": false }
        ])),
        ("GET", "/books/1/manifest.json") => json(json!({
            "book": { "title": "Handbook", "slug": "handbook", "published": false, "editable": true },
            "leaves": [
                { "id": 10, "type": "Section", "title": "Basics" },
                { "id": 11, "type": "Page", "title": "Welcome", "word_count": 4 },
                { "id": 12, "type": "Picture", "title": "Team photo" }
            ]
        })),
        ("GET", "/books/2/manifest.json") => json(json!({
            "book": { "title": "Recipes", "slug": "recipes", "published": false, "editable": false },
            "leaves": [{ "id": 20, "type": "Page", "title": "Soup", "word_count": 2 }]
        })),
        ("GET", "/books/1/sections/10.json") => {
            json(json!({ "title": "Basics", "body": "Start here.", "fingerprint": "s1" }))
        }
        ("GET", "/books/1/pages/11.json") => json(
            json!({ "title": "Welcome", "body": "Hello from the handbook.", "fingerprint": "f1", "record_gid": "gid://writebook/Page/11" }),
        ),
        ("GET", "/books/1/pictures/12.json") => json(
            json!({ "title": "Team photo", "caption": "Everyone at lunch", "fingerprint": "p1", "image": { "url": "/photo.png" } }),
        ),
        ("GET", "/books/2/pages/20.json") => {
            json(json!({ "title": "Soup", "body": "Boil water.", "fingerprint": "r1" }))
        }
        ("PATCH", path) if path.starts_with("/books/1/") => match &save {
            Save::Works | Save::Slow(_) => {
                if let Save::Slow(gate) = &save {
                    gate.hold();
                }
                let mut saves = saves.lock().unwrap();
                *saves += 1;
                json(json!({ "fingerprint": format!("saved{saves}") }))
            }
            Save::Conflicts => reply(409, r#"{"error":"conflict"}"#),
            Save::Fails => reply(500, r#"{"error":"boom"}"#),
        },
        ("POST", "/uploads") => json(json!({ "fileUrl": "https://example.com/rails/notes.txt" })),
        _ => not_found(),
    })
}

fn open_book(tui: &mut Tui, title: &str) {
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    if title == "Recipes" {
        tui.press(KeyCode::Down);
    }
    tui.press(KeyCode::Enter);
}

fn open_welcome(tui: &mut Tui) {
    open_book(tui, "Handbook");
    tui.wait_for("4 words");
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.wait_for("Hello from the handbook.");
}

fn start_editing(tui: &mut Tui) -> String {
    tui.type_text("e");
    tui.wait_for("stop editing")
}

#[test]
fn edits_the_title_and_text_and_saves_with_the_fingerprint_it_was_read_with() {
    let mut tui = handbook(Save::Works);
    open_welcome(&mut tui);

    let screen = start_editing(&mut tui);
    assert!(screen.contains("ctrl+s"), "{screen}");

    tui.press(KeyCode::End);
    tui.type_text(" Enjoy!");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::End);
    tui.type_text(" 2");
    tui.ctrl('s');

    let screen = tui.wait_for_gone("stop editing");
    assert!(screen.contains("Hello from the handbook. Enjoy!"), "{screen}");
    assert!(screen.contains("Welcome 2"), "{screen}");

    let request = tui.wait_for_request("PATCH", "/books/1/pages/11.json");
    assert_eq!(
        body(&request),
        json!({ "leaf": { "title": "Welcome 2" }, "page": { "body": "Hello from the handbook. Enjoy!" }, "base_fingerprint": "f1" })
    );
    assert_eq!(request.header("Authorization"), Some("Bearer wb_test_token"));

    tui.type_text("e");
    tui.wait_for("stop editing");
    tui.press(KeyCode::End);
    tui.type_text(" More.");
    tui.ctrl('s');
    tui.wait_for_gone("stop editing");

    let patches: Vec<_> = tui
        .server
        .requests()
        .into_iter()
        .filter(|it| it.method == "PATCH")
        .collect();
    assert_eq!(patches.len(), 2);
    assert_eq!(
        body(&patches[1]),
        json!({ "leaf": { "title": "Welcome 2" }, "page": { "body": "Hello from the handbook. Enjoy! More." }, "base_fingerprint": "saved1" })
    );
}

#[test]
fn saves_a_section_under_the_section_key() {
    let mut tui = handbook(Save::Works);
    open_book(&mut tui, "Handbook");
    tui.wait_for("4 words");
    tui.press(KeyCode::Enter);
    tui.wait_for("Start here.");

    start_editing(&mut tui);
    tui.press(KeyCode::End);
    tui.type_text(" Now.");
    tui.ctrl('s');
    let screen = tui.wait_for_gone("stop editing");
    assert!(screen.contains("Start here. Now."), "{screen}");

    let request = tui.wait_for_request("PATCH", "/books/1/sections/10.json");
    assert_eq!(
        body(&request),
        json!({ "leaf": { "title": "Basics" }, "section": { "body": "Start here. Now." }, "base_fingerprint": "s1" })
    );
}

#[test]
fn a_conflict_says_someone_changed_the_page_and_keeps_the_edits() {
    let mut tui = handbook(Save::Conflicts);
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.press(KeyCode::End);
    tui.type_text(" My edit.");
    tui.ctrl('s');

    tui.wait_for_request("PATCH", "/books/1/pages/11.json");
    let screen = tui.wait_for("someone changed this page");
    assert!(screen.contains("so it wasn't saved"), "{screen}");
    assert!(screen.contains("Hello from the handbook. My edit."), "{screen}");
    assert!(screen.contains("Welcome > Edit"), "{screen}");
}

#[test]
fn a_failed_save_keeps_the_editor_open_with_the_text_and_can_be_tried_again() {
    let mut tui = handbook(Save::Fails);
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.press(KeyCode::End);
    tui.type_text(" Draft.");
    tui.ctrl('s');
    tui.wait_for_request("PATCH", "/books/1/pages/11.json");
    tui.wait_for("boom");

    let screen = tui.screen();
    assert!(screen.contains("Hello from the handbook. Draft."), "{screen}");
    assert!(screen.contains("Welcome > Edit"), "{screen}");

    tui.ctrl('s');
    tui.wait_for_request_count(tui.server.requests().len() + 1);
    let patches = tui
        .server
        .requests()
        .into_iter()
        .filter(|it| it.method == "PATCH")
        .count();
    assert_eq!(patches, 2);
}

#[test]
fn leaving_with_unsaved_changes_asks_first_and_no_keeps_editing_while_yes_drops_them() {
    let mut tui = handbook(Save::Works);
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.press(KeyCode::Esc);
    let screen = tui.wait_for_gone("stop editing");
    assert!(screen.contains("Hello from the handbook."), "{screen}");
    assert!(!screen.contains("Throw away your changes"), "{screen}");

    start_editing(&mut tui);
    tui.press(KeyCode::End);
    tui.type_text(" Lost?");
    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("Throw away your changes");
    assert!(screen.contains("Yes") && screen.contains("No"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Throw away your changes");
    assert!(screen.contains("Hello from the handbook. Lost?"), "{screen}");
    assert!(screen.contains("Welcome > Edit"), "{screen}");

    tui.press(KeyCode::Esc);
    tui.wait_for("Throw away your changes");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("stop editing");
    assert!(screen.contains("Hello from the handbook."), "{screen}");
    assert!(!screen.contains("Lost?"), "{screen}");
    assert!(
        !tui.paths().iter().any(|it| it.starts_with("PATCH")),
        "{:?}",
        tui.paths()
    );
}

#[test]
fn pasted_lines_go_into_the_text_as_they_are_and_into_the_save() {
    let mut tui = handbook(Save::Works);
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.press(KeyCode::End);
    tui.paste("\n- one\n- two\n\n\tindented\r\n- three");
    let screen = tui.screen();
    assert!(screen.contains("- one"), "{screen}");
    assert!(screen.contains("- two"), "{screen}");

    tui.ctrl('s');
    let request = tui.wait_for_request("PATCH", "/books/1/pages/11.json");
    assert_eq!(
        body(&request)["page"]["body"],
        "Hello from the handbook.\n- one\n- two\n\n    indented\n- three"
    );
}

#[test]
fn uploads_a_file_into_the_page_at_the_cursor_and_saves_the_link() {
    let mut tui = handbook(Save::Works);
    let folder = TempDir::new();
    std::fs::write(folder.join("notes.txt"), "the notes").unwrap();
    tui.app.files_dir = folder.to_path_buf();
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.ctrl('u');
    let screen = tui.wait_for("notes.txt");
    assert!(screen.contains("Upload a file"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("[notes.txt](https://example.com/rails/notes.txt)");
    assert!(screen.contains("Welcome > Edit"), "{screen}");

    let upload = tui.wait_for_request("POST", "/uploads");
    assert!(
        upload
            .header("Content-Type")
            .is_some_and(|it| it.starts_with("multipart/form-data; boundary="))
    );
    assert!(
        upload
            .body
            .contains("name=\"record_gid\"\r\n\r\ngid://writebook/Page/11\r\n"),
        "{}",
        upload.body
    );
    assert!(
        upload.body.contains("name=\"attribute_name\"\r\n\r\nbody\r\n"),
        "{}",
        upload.body
    );
    assert!(upload.body.contains("filename=\"notes.txt\""), "{}", upload.body);
    assert!(upload.body.contains("\r\n\r\nthe notes\r\n"), "{}", upload.body);

    tui.ctrl('s');
    let request = tui.wait_for_request("PATCH", "/books/1/pages/11.json");
    assert_eq!(
        body(&request)["page"]["body"],
        "[notes.txt](https://example.com/rails/notes.txt)\nHello from the handbook."
    );
}

#[test]
fn escapes_brackets_and_backslashes_in_the_label_of_an_uploaded_file() {
    let mut tui = handbook(Save::Works);
    let folder = TempDir::new();
    std::fs::write(folder.join("a\\[b].png"), "image bytes").unwrap();
    std::fs::write(folder.join("notes].pdf"), "the notes").unwrap();
    tui.app.files_dir = folder.to_path_buf();
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.ctrl('u');
    tui.wait_for("notes].pdf");
    tui.press(KeyCode::Enter);
    tui.wait_for("![a\\\\\\[b\\].png](https://example.com/rails/notes.txt)");

    tui.ctrl('u');
    tui.wait_for("notes].pdf");
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    tui.wait_for("[notes\\].pdf](https://example.com/rails/notes.txt)");

    tui.ctrl('s');
    let request = tui.wait_for_request("PATCH", "/books/1/pages/11.json");
    let saved = body(&request)["page"]["body"].as_str().unwrap().to_string();
    assert!(
        saved.contains("![a\\\\\\[b\\].png](https://example.com/rails/notes.txt)"),
        "{saved}"
    );
    assert!(
        saved.contains("[notes\\].pdf](https://example.com/rails/notes.txt)"),
        "{saved}"
    );
}

#[test]
fn saving_waits_for_an_upload_in_flight() {
    let mut tui = handbook(Save::Works);
    let folder = TempDir::new();
    std::fs::write(folder.join("notes.txt"), "the notes").unwrap();
    tui.app.files_dir = folder.to_path_buf();
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.ctrl('u');
    tui.wait_for("notes.txt");
    tui.press(KeyCode::Enter);
    tui.ctrl('s');
    let screen = tui.screen();
    assert!(screen.contains("Wait for the upload to finish, then save."), "{screen}");

    tui.wait_for("[notes.txt](https://example.com/rails/notes.txt)");
    assert!(
        !tui.paths().iter().any(|it| it.starts_with("PATCH")),
        "{:?}",
        tui.paths()
    );
}

#[test]
fn keys_typed_while_a_save_is_in_flight_are_dropped() {
    let gate = Gate::default();
    let mut tui = handbook(Save::Slow(gate.clone()));
    open_welcome(&mut tui);

    start_editing(&mut tui);
    tui.press(KeyCode::End);
    tui.type_text(" Saved.");
    tui.ctrl('s');
    gate.wait_until_held();
    tui.type_text(" Extra");
    tui.paste(" pasted");
    tui.press(KeyCode::Esc);
    gate.release();
    tui.wait_for_replies("LeafSaved", 1);

    let screen = tui.wait_for_gone("stop editing");
    assert!(screen.contains("Hello from the handbook. Saved."), "{screen}");
    assert!(!screen.contains("Extra") && !screen.contains("pasted"), "{screen}");

    let patches: Vec<_> = tui
        .server
        .requests()
        .into_iter()
        .filter(|it| it.method == "PATCH")
        .collect();
    assert_eq!(patches.len(), 1);
    assert_eq!(body(&patches[0])["page"]["body"], "Hello from the handbook. Saved.");
}

#[test]
fn a_read_only_book_and_a_picture_cannot_be_edited() {
    let mut tui = handbook(Save::Works);
    open_book(&mut tui, "Recipes");
    tui.wait_for("Soup");
    tui.press(KeyCode::Enter);
    tui.wait_for("Boil water.");
    tui.type_text("e");
    let screen = tui.screen();
    assert!(!screen.contains("stop editing"), "{screen}");
    assert!(!screen.contains("edit"), "{screen}");

    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Up);
    tui.press(KeyCode::Enter);
    tui.wait_for("4 words");
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.wait_for("Everyone at lunch");
    tui.type_text("e");
    let screen = tui.screen();
    assert!(!screen.contains("stop editing"), "{screen}");
    assert!(
        !tui.paths().iter().any(|it| it.starts_with("PATCH")),
        "{:?}",
        tui.paths()
    );
}
