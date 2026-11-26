//! A reply that comes back late goes only to the screen or overlay it was
//! asked for, and not to whatever is open by then. The fake Writebook holds
//! the first request to one path until the test lets it go.

use crossterm::event::KeyCode;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{Tui, json, not_found};
use crate::scratch::TempDir;

#[derive(Clone, Default)]
struct Gate {
    held: Arc<AtomicBool>,
    open: Arc<AtomicBool>,
}

impl Gate {
    fn hold(&self) {
        self.held.store(true, Ordering::SeqCst);
        while !self.open.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
    }

    fn wait_until_held(&self) {
        while !self.held.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
    }

    fn release(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

fn desk(held: &'static str, gate: &Gate) -> Tui {
    let gate = gate.clone();
    let asked = AtomicUsize::new(0);
    let resets = AtomicUsize::new(0);

    Tui::new(move |request| {
        let which = format!("{} {}", request.method, request.path);
        let order = if which == "POST /account/join_code.json" {
            resets.fetch_add(1, Ordering::SeqCst)
        } else {
            0
        };
        if which == held && asked.fetch_add(1, Ordering::SeqCst) == 0 {
            gate.hold();
        }
        let host = request.header("Host").unwrap().to_string();

        match which.as_str() {
            "GET /books.json" => json(json!([
                { "id": 1, "title": "Handbook", "editable": true },
                { "id": 2, "title": "Recipes", "editable": true }
            ])),
            "GET /books/1/manifest.json" => json(json!({
                "book": { "title": "Handbook", "slug": "handbook", "published": false, "editable": true },
                "leaves": [{ "id": 10, "type": "Page", "title": "Welcome", "word_count": 4 }]
            })),
            "GET /books/2/manifest.json" => json(json!({
                "book": { "title": "Recipes", "slug": "recipes", "published": false, "editable": true },
                "leaves": [{ "id": 20, "type": "Page", "title": "Soup", "word_count": 2 }]
            })),
            "GET /books/1/pages/10.json" => json(json!({
                "title": "Welcome", "body": "Hello from the handbook.", "fingerprint": "f10",
                "record_gid": "gid://writebook/Page/10"
            })),
            "GET /books/2/pages/20.json" => json(json!({
                "title": "Soup", "body": "Boil water.", "fingerprint": "f20",
                "record_gid": "gid://writebook/Page/20"
            })),
            "GET /books/new.json" => json(json!({ "book": {}, "users": [] })),
            "GET /books/1/edit.json" => json(json!({
                "book": { "id": 1, "title": "Handbook", "theme": "blue", "everyone_access": false },
                "users": []
            })),
            "POST /books.json" => json(json!({ "id": 3 })),
            "POST /uploads" => json(json!({ "fileUrl": "https://example.com/rails/notes.txt" })),
            "PATCH /books/1/publication.json" => json(json!({
                "published": true, "slug": "handbook", "url": format!("http://{host}/1/handbook")
            })),
            "GET /users.json" => json(json!({
                "users": [
                    { "id": 1, "name": "Me", "role": "administrator", "current": true },
                    { "id": 2, "name": "Kevin", "role": "member" }
                ],
                "join_url": format!("http://{host}/join/old"),
                "can_administer": true
            })),
            "POST /account/join_code.json" => {
                let link = if order == 0 { "first" } else { "second" };
                json(json!({ "join_url": format!("http://{host}/join/{link}") }))
            }
            "GET /users/1/profile/edit.json" => json(json!({ "name": "Me", "email_address": "me@example.com" })),
            "PATCH /users/1/profile.json" => json(json!({})),
            "GET /search.json?q=rabbit" => json(json!({ "results": [
                { "title": "Introduction", "url": "https://books.example.com/1/handbook/10/introduction", "snippet": "rabbit", "book": { "title": "Handbook" } }
            ] })),
            _ => not_found(),
        }
    })
}

fn open_books(tui: &mut Tui) {
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
}

fn open_book(tui: &mut Tui, title: &str, first_leaf: &str) {
    if title == "Recipes" {
        tui.press(KeyCode::Down);
    }
    tui.press(KeyCode::Enter);
    tui.wait_for(first_leaf);
}

fn open_settings(tui: &mut Tui) {
    open_books(tui);
    tui.press(KeyCode::Char('s'));
    tui.wait_for("Kevin");
}

#[test]
fn a_form_loading_late_does_not_fill_the_form_opened_after_it() {
    let gate = Gate::default();
    let mut tui = desk("GET /books/new.json", &gate);
    open_books(&mut tui);

    tui.press(KeyCode::Char('n'));
    gate.wait_until_held();
    tui.press(KeyCode::Esc);
    open_book(&mut tui, "Handbook", "4 words");
    tui.press(KeyCode::Char('e'));
    tui.wait_for("Subtitle");

    gate.release();
    tui.wait_for_request("GET", "/books/new.json");
    tui.wait_for_replies("BookFormLoaded", 2);
    let screen = tui.screen();
    assert!(screen.contains("books > Books > Handbook > Edit"), "{screen}");
    assert!(!screen.contains("New book"), "{screen}");
}

#[test]
fn a_book_form_stays_open_until_its_save_comes_back() {
    let gate = Gate::default();
    let mut tui = desk("POST /books.json", &gate);
    open_books(&mut tui);
    tui.press(KeyCode::Char('n'));
    tui.wait_for("Subtitle");
    tui.type_text("Field Guide");
    tui.ctrl('s');
    gate.wait_until_held();

    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(screen.contains("books > Books > New book"), "{screen}");

    gate.release();
    let screen = tui.wait_for_gone("New book");
    assert!(screen.contains("books > Books"), "{screen}");
}

#[test]
fn a_page_loading_late_does_not_replace_the_page_opened_after_it() {
    let gate = Gate::default();
    let mut tui = desk("GET /books/1/pages/10.json", &gate);
    open_books(&mut tui);
    open_book(&mut tui, "Handbook", "4 words");
    tui.press(KeyCode::Enter);
    gate.wait_until_held();
    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    open_book(&mut tui, "Recipes", "2 words");
    tui.press(KeyCode::Enter);
    tui.wait_for("Boil water.");

    gate.release();
    tui.wait_for_request("GET", "/books/1/pages/10.json");
    tui.wait_for_replies("LeafLoaded", 2);
    let screen = tui.screen();
    assert!(screen.contains("Boil water."), "{screen}");
    assert!(!screen.contains("Hello from the handbook."), "{screen}");
}

#[test]
fn an_upload_finishing_late_does_not_go_into_the_page_edited_after_it() {
    let gate = Gate::default();
    let mut tui = desk("POST /uploads", &gate);
    let folder = TempDir::new();
    std::fs::write(folder.join("notes.txt"), "the notes").unwrap();
    tui.app.files_dir = folder.to_path_buf();
    open_books(&mut tui);
    open_book(&mut tui, "Handbook", "4 words");
    tui.press(KeyCode::Enter);
    tui.wait_for("Hello from the handbook.");
    tui.type_text("e");
    tui.wait_for("stop editing");
    tui.ctrl('u');
    tui.wait_for("notes.txt");
    tui.press(KeyCode::Enter);
    gate.wait_until_held();

    tui.press(KeyCode::Esc);
    tui.wait_for_gone("stop editing");
    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    open_book(&mut tui, "Recipes", "2 words");
    tui.press(KeyCode::Enter);
    tui.wait_for("Boil water.");
    tui.type_text("e");
    tui.wait_for("stop editing");

    gate.release();
    let screen = tui.wait_for("Uploaded, but the page it was for isn't being edited any more.");
    assert!(!screen.contains("notes.txt"), "{screen}");
    assert!(screen.contains("Boil water."), "{screen}");
}

#[test]
fn books_loading_late_arrive_under_the_settings_opened_over_them() {
    let gate = Gate::default();
    let mut tui = desk("GET /books.json", &gate);

    tui.press(KeyCode::Enter);
    gate.wait_until_held();
    tui.press(KeyCode::Char('s'));
    tui.wait_for("Kevin");

    gate.release();
    tui.wait_for_request("GET", "/books.json");
    tui.wait_for_replies("BooksLoaded", 1);
    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("Recipes");
    assert!(screen.contains("Handbook"), "{screen}");
}

#[test]
fn a_publishing_reply_reaches_its_overlay_under_the_quit_question_and_blocks_editing_the_address_meanwhile() {
    let gate = Gate::default();
    let mut tui = desk("PATCH /books/1/publication.json", &gate);
    open_books(&mut tui);
    open_book(&mut tui, "Handbook", "4 words");
    tui.press(KeyCode::Char('p'));
    tui.wait_for("Only people with access can read it.");

    tui.press(KeyCode::Down);
    gate.wait_until_held();
    tui.press(KeyCode::Char('e'));
    let screen = tui.screen();
    assert!(!screen.contains("Address"), "{screen}");

    tui.ctrl('c');
    tui.wait_for("Quit writebook tui?");
    gate.release();
    tui.wait_for_request("PATCH", "/books/1/publication.json");
    tui.wait_for_replies("Published", 1);
    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(screen.contains(&format!("{}/1/handbook", tui.server.url)), "{screen}");
    assert!(screen.contains("Publishing"), "{screen}");

    tui.press(KeyCode::Char('e'));
    let screen = tui.screen();
    assert!(screen.contains("Address"), "{screen}");
}

#[test]
fn search_results_arrive_under_the_quit_question() {
    let gate = Gate::default();
    let mut tui = desk("GET /search.json?q=rabbit", &gate);
    open_books(&mut tui);
    tui.press(KeyCode::Char('/'));
    tui.type_text("rabbit");
    tui.press(KeyCode::Enter);
    gate.wait_until_held();

    tui.ctrl('c');
    tui.wait_for("Quit writebook tui?");
    gate.release();
    tui.wait_for_request("GET", "/search.json?q=rabbit");
    tui.wait_for_replies("Found", 1);
    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(screen.contains("Search all books"), "{screen}");
    assert!(screen.contains("Introduction"), "{screen}");
}

#[test]
fn only_the_last_invite_reset_shows_its_link() {
    let gate = Gate::default();
    let mut tui = desk("POST /account/join_code.json", &gate);
    open_settings(&mut tui);

    for _ in 0..2 {
        tui.press(KeyCode::Char('r'));
        tui.wait_for("Make a new invite link?");
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Enter);
        if !gate.held.load(Ordering::SeqCst) {
            gate.wait_until_held();
        } else {
            tui.wait_for("/join/second");
        }
    }

    gate.release();
    tui.wait_for_request("POST", "/account/join_code.json");
    tui.wait_for_replies("InviteReset", 2);
    let screen = tui.screen();
    assert!(screen.contains("/join/second"), "{screen}");
    assert!(!screen.contains("/join/first"), "{screen}");
}

#[test]
fn a_profile_saved_late_leaves_the_profile_opened_after_it_alone() {
    let gate = Gate::default();
    let mut tui = desk("PATCH /users/1/profile.json", &gate);
    open_settings(&mut tui);
    tui.press(KeyCode::Enter);
    tui.wait_for("books > Settings > Me");
    tui.ctrl('s');
    gate.wait_until_held();

    tui.press(KeyCode::Esc);
    tui.wait_for_gone("books > Settings > Me");
    tui.press(KeyCode::Enter);
    tui.wait_for("books > Settings > Me");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.paste("secret");

    gate.release();
    tui.wait_for_request("PATCH", "/users/1/profile.json");
    tui.wait_for_replies("ProfileSaved", 1);
    let screen = tui.screen();
    assert!(screen.contains("••••••"), "{screen}");
    assert!(!screen.contains("Saved."), "{screen}");
}
