use crossterm::event::KeyCode;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{Tui, body, json, not_found, reply};
use crate::scratch::TempDir;

fn library(write: impl Fn(&super::Request, &mut Vec<Value>) -> Option<super::Reply> + Send + Sync + 'static) -> Tui {
    let leaves = Mutex::new(vec![
        json!({ "id": 10, "type": "Section", "title": "Basics" }),
        json!({ "id": 11, "type": "Page", "title": "Welcome", "word_count": 4 }),
        json!({ "id": 12, "type": "Page", "title": "Setup", "word_count": 9 }),
    ]);

    Tui::new(move |request| {
        let mut leaves = leaves.lock().unwrap();

        if let Some(answer) = write(request, &mut leaves) {
            answer
        } else {
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/books.json") => {
                    json(json!([{ "id": 1, "title": "Handbook", "author": "Ops", "editable": true }]))
                }
                ("GET", "/books/1/manifest.json") => json(json!({
                    "book": { "title": "Handbook", "slug": "handbook", "published": false, "editable": true },
                    "leaves": *leaves
                })),
                ("GET", "/books/1/edit.json") => json(json!({
                    "book": { "id": 1, "title": "Handbook", "subtitle": "How we work", "author": "Ops", "theme": "blue", "everyone_access": false },
                    "users": [{ "id": 5, "name": "Ana", "role": "reader" }]
                })),
                _ => not_found(),
            }
        }
    })
}

fn open_book(tui: &mut Tui) {
    tui.press(KeyCode::Enter);
    tui.wait_for("Handbook");
    tui.press(KeyCode::Enter);
    tui.wait_for("Basics");
}

fn open_edit_form(tui: &mut Tui) {
    open_book(tui);
    tui.press(KeyCode::Char('e'));
    tui.wait_for("Subtitle");
}

fn has_part(request: &super::Request, name: &str, value: &str) -> bool {
    request.body.contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n"))
}

fn publications(tui: &Tui) -> Vec<Value> {
    tui.server
        .requests()
        .iter()
        .filter(|it| it.path == "/books/1/publication.json")
        .map(body)
        .collect()
}

fn picture_folder(file: &str) -> TempDir {
    let folder = TempDir::new();
    std::fs::write(folder.join(file), "image bytes").unwrap();
    std::fs::write(folder.join("notes.txt"), "not an image").unwrap();
    folder
}

#[test]
fn creates_a_book_from_the_form_and_opens_it() {
    let created = AtomicBool::new(false);
    let mut tui = Tui::new(move |request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") if created.load(Ordering::SeqCst) => json(json!([
            { "id": 1, "title": "Handbook", "editable": true },
            { "id": 2, "title": "Field Guide", "editable": true }
        ])),
        ("GET", "/books.json") => json(json!([{ "id": 1, "title": "Handbook", "editable": true }])),
        ("GET", "/books/new.json") => json(json!({ "book": {}, "users": [] })),
        ("POST", "/books.json") => {
            created.store(true, Ordering::SeqCst);
            json(json!({ "id": 2 }))
        }
        ("GET", "/books/2/manifest.json") => json(json!({
            "book": { "title": "Field Guide", "slug": "field-guide", "published": false, "editable": true },
            "leaves": []
        })),
        _ => not_found(),
    });

    tui.press(KeyCode::Enter);
    tui.wait_for("Handbook");
    tui.press(KeyCode::Char('n'));
    let screen = tui.wait_for("Subtitle");
    assert!(screen.contains("books > Books > New book"), "{screen}");

    tui.type_text("Field Guide");
    tui.press(KeyCode::Tab);
    tui.type_text("Birds nearby");
    tui.press(KeyCode::Tab);
    tui.type_text("Sam");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    tui.ctrl('s');

    let screen = tui.wait_for("This book has no pages yet.");
    assert!(screen.contains("books > Books > Field Guide"), "{screen}");

    let request = tui.wait_for_request("POST", "/books.json");
    assert!(
        request
            .header("Content-Type")
            .is_some_and(|it| it.starts_with("multipart/form-data"))
    );
    assert!(has_part(&request, "book[title]", "Field Guide"), "{}", request.body);
    assert!(has_part(&request, "book[subtitle]", "Birds nearby"), "{}", request.body);
    assert!(has_part(&request, "book[author]", "Sam"), "{}", request.body);
    assert!(has_part(&request, "book[theme]", "green"), "{}", request.body);
    assert!(has_part(&request, "book[everyone_access]", "false"), "{}", request.body);
    tui.wait_for_request_count(5);
    let mut reloads = tui.paths().split_off(3);
    reloads.sort();
    assert_eq!(
        tui.paths()[..3],
        ["GET /books.json", "GET /books/new.json", "POST /books.json"]
    );
    assert_eq!(
        reloads,
        ["GET /books.json", "GET /books/2/manifest.json"].map(String::from)
    );
}

#[test]
fn a_new_book_without_a_title_or_refused_by_the_server_keeps_the_form() {
    let mut tui = Tui::new(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([{ "id": 1, "title": "Handbook", "editable": true }])),
        ("GET", "/books/new.json") => json(json!({ "book": {}, "users": [] })),
        ("POST", "/books.json") => reply(422, r#"{"error":"Title is already taken"}"#),
        _ => not_found(),
    });

    tui.press(KeyCode::Enter);
    tui.wait_for("Handbook");
    tui.press(KeyCode::Char('n'));
    tui.wait_for("Subtitle");

    tui.ctrl('s');
    let screen = tui.screen();
    assert!(screen.contains("A book needs a title."), "{screen}");
    assert!(screen.contains("books > Books > New book"), "{screen}");
    assert_eq!(tui.paths(), ["GET /books.json", "GET /books/new.json"]);

    tui.type_text("Handbook");
    tui.ctrl('s');
    tui.wait_for_request("POST", "/books.json");
    let screen = tui.wait_for("the server answered 422: Title is already taken");
    assert!(screen.contains("books > Books > New book"), "{screen}");
    assert!(screen.contains("Handbook"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for_gone("New book");
    assert!(screen.contains("books > Books"), "{screen}");
}

#[test]
fn edits_a_book_and_sends_the_changed_fields_and_access() {
    let mut tui = library(|request, _| match (request.method.as_str(), request.path.as_str()) {
        ("PATCH", "/books/1.json") => Some(reply(204, "")),
        _ => None,
    });

    open_edit_form(&mut tui);
    let screen = tui.screen();
    assert!(screen.contains("books > Books > Handbook > Edit"), "{screen}");
    assert!(screen.contains("How we work"), "{screen}");
    assert!(screen.contains("Ops"), "{screen}");
    assert!(
        screen.contains("Everyone ..................................................... ‹ off ›"),
        "{screen}"
    );
    assert!(
        screen.contains("Ana ..................................................... ‹ can read ›"),
        "{screen}"
    );

    tui.type_text(" 2");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Backspace);
    tui.type_text("s");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    let screen = tui.screen();
    assert!(
        screen.contains("Everyone ...................................................... ‹ on ›"),
        "{screen}"
    );
    assert!(
        screen.contains("Ana ..................................................... ‹ can edit ›"),
        "{screen}"
    );

    tui.ctrl('s');
    let request = tui.wait_for_request("PATCH", "/books/1.json");
    assert!(has_part(&request, "book[title]", "Handbook 2"), "{}", request.body);
    assert!(has_part(&request, "book[subtitle]", "How we wors"), "{}", request.body);
    assert!(has_part(&request, "book[author]", "Ops"), "{}", request.body);
    assert!(has_part(&request, "book[theme]", "green"), "{}", request.body);
    assert!(has_part(&request, "book[everyone_access]", "true"), "{}", request.body);
    assert!(has_part(&request, "editor_ids[]", "5"), "{}", request.body);
    assert!(has_part(&request, "reader_ids[]", "5"), "{}", request.body);

    tui.wait_for_request_count(5);
    let screen = tui.wait_for("Basics");
    assert!(screen.contains("books > Books > Handbook"), "{screen}");
    assert_eq!(tui.paths()[3..], ["PATCH /books/1.json", "GET /books/1/manifest.json"]);
}

#[test]
fn a_blank_title_or_server_error_keeps_the_edit_form_and_esc_leaves_without_saving() {
    let mut tui = library(|request, _| match (request.method.as_str(), request.path.as_str()) {
        ("PATCH", "/books/1.json") => Some(reply(422, r#"{"error":"Title is already taken"}"#)),
        _ => None,
    });

    open_edit_form(&mut tui);
    for _ in 0.."Handbook".len() {
        tui.press(KeyCode::Backspace);
    }
    tui.ctrl('s');
    let screen = tui.screen();
    assert!(screen.contains("A book needs a title."), "{screen}");
    assert!(!tui.paths().iter().any(|it| it.starts_with("PATCH")));

    tui.type_text("Recipes");
    tui.ctrl('s');
    let screen = tui.wait_for("the server answered 422: Title is already taken");
    assert!(screen.contains("books > Books > Recipes > Edit"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("Basics");
    assert!(!screen.contains("Edit"), "{screen}");
    assert_eq!(tui.paths().iter().filter(|it| it.starts_with("PATCH")).count(), 1);
}

#[test]
fn deletes_a_book_after_confirming_and_returns_to_the_books() {
    let mut tui = library(|request, _| match (request.method.as_str(), request.path.as_str()) {
        ("DELETE", "/books/1.json") => Some(reply(204, "")),
        _ => None,
    });

    open_edit_form(&mut tui);
    tui.press(KeyCode::Up);
    tui.press(KeyCode::Up);
    let screen = tui.screen();
    assert!(screen.contains("delete book"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("Delete “Handbook” and all its pages?"), "{screen}");
    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(!screen.contains("all its pages"), "{screen}");
    assert!(screen.contains("books > Books > Handbook > Edit"), "{screen}");
    assert!(!tui.paths().iter().any(|it| it.starts_with("DELETE")));

    tui.press(KeyCode::Enter);
    tui.wait_for("all its pages");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);

    tui.wait_for_request("DELETE", "/books/1.json");
    let screen = tui.wait_for_gone("Edit");
    assert!(screen.contains("books > Books"), "{screen}");
    assert!(!screen.contains("books > Books > Handbook"), "{screen}");
}

#[test]
fn picks_a_cover_with_the_file_picker_and_uploads_it_with_the_book() {
    let folder = picture_folder("cover.png");
    let mut tui = library(|request, _| match (request.method.as_str(), request.path.as_str()) {
        ("PATCH", "/books/1.json") => Some(reply(204, "")),
        _ => None,
    });
    tui.app.files_dir = folder.to_path_buf();

    open_edit_form(&mut tui);
    tui.press(KeyCode::Up);
    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("Choose a cover"), "{screen}");
    assert!(screen.contains("cover.png"), "{screen}");
    assert!(!screen.contains("notes.txt"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(!screen.contains("Choose a cover"), "{screen}");
    assert!(!screen.contains("New: cover.png"), "{screen}");

    tui.press(KeyCode::Enter);
    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("New: cover.png"), "{screen}");

    tui.ctrl('s');
    let request = tui.wait_for_request("PATCH", "/books/1.json");
    assert!(
        request.body.contains("name=\"book[cover]\"; filename=\"cover.png\""),
        "{}",
        request.body
    );
    assert!(request.body.contains("\r\n\r\nimage bytes\r\n"), "{}", request.body);
    assert!(!request.body.contains("remove_cover"), "{}", request.body);
}

#[test]
fn removes_the_cover_of_a_book() {
    let mut tui = Tui::new(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([{ "id": 1, "title": "Handbook", "editable": true }])),
        ("GET", "/books/1/manifest.json") => json(json!({
            "book": { "title": "Handbook", "slug": "handbook", "published": false, "editable": true },
            "leaves": [{ "id": 10, "type": "Section", "title": "Basics" }]
        })),
        ("GET", "/books/1/edit.json") => json(json!({
            "book": { "id": 1, "title": "Handbook", "theme": "blue", "cover": { "url": "/covers/handbook.png" } },
            "users": []
        })),
        ("PATCH", "/books/1.json") => reply(204, ""),
        _ => not_found(),
    });

    open_edit_form(&mut tui);
    tui.press(KeyCode::Up);
    tui.press(KeyCode::Char('x'));
    let screen = tui.screen();
    assert!(screen.contains("Cover removed"), "{screen}");

    tui.ctrl('s');
    let request = tui.wait_for_request("PATCH", "/books/1.json");
    assert!(has_part(&request, "remove_cover", "true"), "{}", request.body);
    assert!(!request.body.contains("book[cover]"), "{}", request.body);
}

#[test]
fn adds_a_page_and_a_section_and_checks_the_form() {
    let mut tui = library(
        |request, leaves| match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/books/1/pages.json") => {
                leaves.push(json!({ "id": 20, "type": "Page", "title": body(request)["leaf"]["title"] }));
                Some(reply(201, "{}"))
            }
            ("POST", "/books/1/sections.json") => {
                leaves.push(json!({ "id": 21, "type": "Section", "title": body(request)["leaf"]["title"] }));
                Some(reply(201, "{}"))
            }
            _ => None,
        },
    );
    open_book(&mut tui);
    tui.press(KeyCode::Char('v'));

    tui.press(KeyCode::Char('a'));
    let screen = tui.screen();
    assert!(screen.contains("Page"), "{screen}");
    assert!(screen.contains("Picture"), "{screen}");
    assert!(screen.contains("Section"), "{screen}");
    tui.press(KeyCode::Esc);
    tui.wait_for_gone("Picture");

    tui.press(KeyCode::Char('a'));
    tui.press(KeyCode::Enter);
    tui.wait_for("Add a page");
    tui.ctrl('s');
    let screen = tui.screen();
    assert!(screen.contains("Fill in every field first."), "{screen}");
    assert!(screen.contains("Add a page"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(screen.contains("Picture"), "{screen}");
    assert!(!screen.contains("Add a page"), "{screen}");
    tui.press(KeyCode::Enter);
    tui.type_text("Install");
    tui.ctrl('s');

    let request = tui.wait_for_request("POST", "/books/1/pages.json");
    assert_eq!(
        body(&request),
        json!({ "leaf": { "title": "Install" }, "page": { "body": "" } })
    );
    let screen = tui.wait_for("Install");
    assert!(!screen.contains("Add a page"), "{screen}");

    tui.press(KeyCode::Char('a'));
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    tui.wait_for("Add a section");
    tui.type_text("Appendix");
    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("add"), "enter on the Add button adds\n{screen}");
    tui.press(KeyCode::Enter);

    let request = tui.wait_for_request("POST", "/books/1/sections.json");
    assert_eq!(
        body(&request),
        json!({ "leaf": { "title": "Appendix" }, "section": { "body": "" } })
    );
    tui.wait_for("Appendix");
    assert_eq!(
        tui.paths()
            .iter()
            .filter(|it| *it == "GET /books/1/manifest.json")
            .count(),
        3
    );

    tui.press(KeyCode::Char('a'));
    tui.press(KeyCode::Enter);
    tui.wait_for("Add a page");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Add a page");
    assert!(!screen.contains("Picture"), "{screen}");
}

#[test]
fn adds_a_picture_with_a_caption_and_a_file_from_the_picker() {
    let folder = picture_folder("floor-plan.png");
    let mut tui = library(
        |request, leaves| match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/books/1/pictures.json") => {
                leaves.push(json!({ "id": 22, "type": "Picture", "title": "Floor plan" }));
                Some(reply(201, "{}"))
            }
            _ => None,
        },
    );
    tui.app.files_dir = folder.to_path_buf();
    open_book(&mut tui);

    tui.press(KeyCode::Char('a'));
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Add a picture");
    assert!(screen.contains("Press enter to choose a file"), "{screen}");

    tui.type_text("Floor plan");
    tui.ctrl('s');
    let screen = tui.screen();
    assert!(screen.contains("Fill in every field first."), "{screen}");

    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("Choose a picture"), "{screen}");
    assert!(screen.contains("floor-plan.png"), "{screen}");
    assert!(!screen.contains("notes.txt"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("Add a picture"), "{screen}");
    assert!(screen.contains(&*folder.to_string_lossy()), "{screen}");
    assert!(!screen.contains("Press enter to choose a file"), "{screen}");

    tui.ctrl('s');
    let request = tui.wait_for_request("POST", "/books/1/pictures.json");
    assert!(has_part(&request, "leaf[title]", "Floor plan"), "{}", request.body);
    assert!(has_part(&request, "picture[caption]", "Floor plan"), "{}", request.body);
    assert!(
        request
            .body
            .contains("name=\"picture[image]\"; filename=\"floor-plan.png\""),
        "{}",
        request.body
    );
    assert!(request.body.contains("\r\n\r\nimage bytes\r\n"), "{}", request.body);
    tui.wait_for("Floor plan");
}

#[test]
fn deletes_a_leaf_only_after_answering_yes() {
    let mut tui = library(
        |request, leaves| match (request.method.as_str(), request.path.as_str()) {
            ("DELETE", "/books/1/pages/11.json") => {
                leaves.retain(|it| it["id"] != 11);
                Some(reply(204, ""))
            }
            ("DELETE", "/books/1/sections/10.json") => {
                leaves.retain(|it| it["id"] != 10);
                Some(reply(204, ""))
            }
            _ => None,
        },
    );
    open_book(&mut tui);

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Char('d'));
    let screen = tui.screen();
    assert!(screen.contains("Delete “Welcome”?"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.screen();
    assert!(
        !screen.contains("Delete “Welcome”?"),
        "enter alone answers No\n{screen}"
    );
    assert!(screen.contains("Welcome"), "{screen}");

    tui.press(KeyCode::Char('d'));
    tui.press(KeyCode::Esc);
    tui.wait_for_gone("Delete “Welcome”?");
    assert!(!tui.paths().iter().any(|it| it.starts_with("DELETE")));

    tui.press(KeyCode::Char('d'));
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    tui.wait_for_request("DELETE", "/books/1/pages/11.json");
    let screen = tui.wait_for_gone("Welcome");
    assert!(screen.contains("Setup"), "{screen}");
    assert_eq!(tui.paths().last().unwrap(), "GET /books/1/manifest.json");
}

#[test]
fn moves_a_leaf_and_sends_the_new_order_when_enter_saves_it() {
    let mut tui = library(
        |request, leaves| match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/books/1/leaves/moves.json") => {
                let order = body(request)["id"].as_array().unwrap().clone();
                leaves.sort_by_key(|leaf| order.iter().position(|id| *id == leaf["id"]));
                Some(reply(204, ""))
            }
            _ => None,
        },
    );
    open_book(&mut tui);

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Char('m'));
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Esc);
    assert!(!tui.paths().iter().any(|it| it.contains("moves")));

    tui.press(KeyCode::Char('m'));
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);

    let request = tui.wait_for_request("POST", "/books/1/leaves/moves.json");
    assert_eq!(body(&request), json!({ "id": [10, 12, 11], "position": 0 }));
    tui.wait_for_request_count(4);
    let screen = tui.wait_for("Welcome");
    assert!(
        screen.find("Setup").unwrap() < screen.find("Welcome").unwrap(),
        "{screen}"
    );
}

#[test]
fn publishes_changes_the_address_copies_it_and_unpublishes() {
    let mut tui = library(|request, _| match (request.method.as_str(), request.path.as_str()) {
        ("PATCH", "/books/1/publication.json") => {
            let book = &body(request)["book"];
            let host = request.header("Host").unwrap();
            Some(json(
                json!({ "published": book["published"], "slug": book["slug"], "url": format!("http://{host}/1/{}", book["slug"].as_str().unwrap()) }),
            ))
        }
        _ => None,
    });
    open_book(&mut tui);

    tui.press(KeyCode::Char('p'));
    let screen = tui.wait_for("Only people with access can read it.");
    assert!(screen.contains("Publishing"), "{screen}");

    tui.press(KeyCode::Down);
    let request = tui.wait_for_request("PATCH", "/books/1/publication.json");
    assert_eq!(
        body(&request),
        json!({ "book": { "published": true, "slug": "handbook" } })
    );
    let address = format!("{}/1/handbook", tui.server.url);
    let screen = tui.wait_for(&address);
    assert!(!screen.contains("Only people with access can read it."), "{screen}");

    tui.press(KeyCode::Char('c'));
    let screen = tui.screen();
    assert!(screen.contains(&format!("Copied {address}")), "{screen}");

    tui.press(KeyCode::Char('e'));
    for _ in 0.."handbook".len() {
        tui.press(KeyCode::Backspace);
    }
    tui.type_text("guide");
    tui.press(KeyCode::Enter);
    let address = format!("{}/1/guide", tui.server.url);
    tui.wait_for(&address);
    assert_eq!(
        publications(&tui)[1],
        json!({ "book": { "published": true, "slug": "guide" } })
    );

    tui.press(KeyCode::Down);
    tui.wait_for("Only people with access can read it.");
    assert_eq!(
        publications(&tui)[2],
        json!({ "book": { "published": false, "slug": "guide" } })
    );

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for_gone("Publishing");
    assert!(screen.contains("Basics"), "{screen}");
}

#[test]
fn shows_an_error_when_the_server_does_not_say_how_the_book_is_published() {
    let mut tui = library(|request, _| match (request.method.as_str(), request.path.as_str()) {
        ("PATCH", "/books/1/publication.json") => Some(json(json!({}))),
        _ => None,
    });
    open_book(&mut tui);

    tui.press(KeyCode::Char('p'));
    tui.wait_for("Only people with access can read it.");
    tui.press(KeyCode::Down);

    let screen = tui.wait_for("didn't say how the book is published now");
    assert!(screen.contains("Only people with access can read it."), "{screen}");
}

#[test]
fn a_book_that_is_not_editable_offers_no_way_to_change_it() {
    let mut tui = Tui::new(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([{ "id": 2, "title": "Recipes", "editable": false }])),
        ("GET", "/books/2/manifest.json") => json(json!({
            "book": { "title": "Recipes", "slug": "recipes", "published": false, "editable": false },
            "leaves": [{ "id": 30, "type": "Page", "title": "Pasta", "word_count": 5 }]
        })),
        _ => not_found(),
    });

    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Pasta");
    assert!(!screen.contains("add"), "{screen}");
    assert!(!screen.contains("delete"), "{screen}");
    assert!(!screen.contains("publishing"), "{screen}");

    for key in ['a', 'd', 'm', 'p', 'e'] {
        tui.press(KeyCode::Char(key));
    }

    let screen = tui.screen();
    assert!(screen.contains("books > Books > Recipes"), "{screen}");
    assert!(!screen.contains("Add a"), "{screen}");
    assert!(!screen.contains("Publishing"), "{screen}");
    assert!(!screen.contains("Delete"), "{screen}");
    assert!(!screen.contains("Edit"), "{screen}");
    assert_eq!(tui.paths(), ["GET /books.json", "GET /books/2/manifest.json"]);
}
