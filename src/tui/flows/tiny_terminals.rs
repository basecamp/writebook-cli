//! Every screen and overlay drawn in terminals too small for them, which
//! must not panic.

use crossterm::event::KeyCode;
use serde_json::json;

use super::{Tui, json, not_found};
use crate::scratch::TempDir;

fn writebook() -> Tui {
    let people: Vec<_> = (1..30)
        .map(|id| json!({ "id": id, "name": format!("Person {id}"), "role": "member", "current": id == 1 }))
        .collect();

    Tui::new(move |request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([
            { "id": 1, "title": "Handbook", "subtitle": "How we work", "author": "Ops", "theme": "green", "editable": true },
            { "id": 2, "title": "Recipes", "editable": true }
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
            "book": { "title": "Recipes", "slug": "recipes", "published": false, "editable": true },
            "leaves": []
        })),
        ("GET", "/books/1/pages/11.json") => json(json!({
            "title": "Welcome",
            "body": "# Title\n\nSome text, a list:\n\n- one\n- > two\n\n![a](/a.png)\n\n| a | b |\n|---|---|\n| 1 | 2 |",
            "fingerprint": "f11",
            "record_gid": "gid://writebook/Page/11"
        })),
        ("GET", "/books/1/edit.json") => json(json!({
            "book": { "id": 1, "title": "Handbook", "theme": "blue", "everyone_access": false },
            "users": [{ "id": 5, "name": "Ana", "role": "reader" }]
        })),
        ("GET", "/users.json") => json(json!({
            "users": people,
            "join_url": "https://books.example.com/join/abc",
            "can_administer": true
        })),
        ("GET", "/users/1/profile/edit.json") => {
            json(json!({ "name": "Person 1", "email_address": "one@example.com" }))
        }
        _ => not_found(),
    })
}

fn draw_in_small_terminals(tui: &mut Tui) {
    for (width, height) in [(0, 0), (1, 1), (2, 1), (4, 3), (8, 5), (12, 4), (20, 8), (30, 12)] {
        tui.resize(width, height);
        tui.draw();
    }
    tui.resize(100, 40);
}

fn draw_with_each_overlay(tui: &mut Tui, keys: &[KeyCode]) {
    draw_in_small_terminals(tui);

    for key in keys {
        tui.press(*key);
        draw_in_small_terminals(tui);
        tui.press(KeyCode::Esc);
    }
}

#[test]
fn draws_every_screen_and_overlay_in_a_terminal_too_small_for_them() {
    let folder = TempDir::new();
    std::fs::write(folder.join("cover.png"), "image bytes").unwrap();
    let mut tui = writebook();
    tui.app.files_dir = folder.to_path_buf();

    draw_with_each_overlay(&mut tui, &[KeyCode::Tab]);

    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    for _ in 0..2 {
        draw_with_each_overlay(&mut tui, &[KeyCode::Tab, KeyCode::Char('/'), KeyCode::Char('n')]);
        tui.press(KeyCode::Char('v'));
    }

    tui.press(KeyCode::Enter);
    tui.wait_for("4 words");
    for _ in 0..2 {
        draw_with_each_overlay(
            &mut tui,
            &[KeyCode::Tab, KeyCode::Char('a'), KeyCode::Char('p'), KeyCode::Char('d')],
        );
        tui.press(KeyCode::Char('v'));
    }

    tui.press(KeyCode::Char('e'));
    tui.wait_for("Subtitle");
    draw_in_small_terminals(&mut tui);
    tui.press(KeyCode::Up);
    tui.press(KeyCode::Enter);
    tui.wait_for("cover.png");
    draw_in_small_terminals(&mut tui);
    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Esc);
    tui.wait_for("4 words");

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.wait_for("Some text, a list:");
    draw_with_each_overlay(&mut tui, &[KeyCode::Tab]);
    tui.press(KeyCode::Char('e'));
    tui.wait_for("stop editing");
    draw_in_small_terminals(&mut tui);
    tui.ctrl('u');
    tui.wait_for("cover.png");
    draw_in_small_terminals(&mut tui);
    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Esc);
    tui.wait_for_gone("stop editing");
    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");

    tui.press(KeyCode::Char('s'));
    tui.wait_for("Person 12");
    draw_in_small_terminals(&mut tui);
    tui.press(KeyCode::Enter);
    tui.wait_for("one@example.com");
    draw_in_small_terminals(&mut tui);
    tui.ctrl('c');
    draw_in_small_terminals(&mut tui);
}
