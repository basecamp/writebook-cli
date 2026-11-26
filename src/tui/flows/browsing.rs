use crossterm::event::KeyCode;
use serde_json::json;
use std::time::Duration;

use super::{Instance, Loading, Tui, json, not_found, reply};

fn paragraphs() -> String {
    (1..=60)
        .map(|number| format!("Paragraph {number}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn library() -> Tui {
    Tui::new(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([
            { "id": 1, "title": "Handbook", "subtitle": "How we work", "author": "Ops", "editable": true },
            { "id": 2, "title": "Recipes", "editable": false }
        ])),
        ("GET", "/books/1/manifest.json") => json(json!({
            "book": { "title": "Handbook", "slug": "handbook", "published": false, "editable": true },
            "leaves": [
                { "id": 10, "type": "Section", "title": "Basics" },
                { "id": 11, "type": "Page", "title": "Welcome", "word_count": 4 },
                { "id": 12, "type": "Page", "title": "Long read", "word_count": 120 },
                { "id": 13, "type": "Picture", "title": "Office map" }
            ]
        })),
        ("GET", "/books/2/manifest.json") => json(json!({
            "book": { "title": "Recipes", "slug": "recipes", "published": true, "editable": false },
            "leaves": [{ "id": 20, "type": "Page", "title": "Pancakes", "word_count": 2 }]
        })),
        ("GET", "/books/1/sections/10.json") => {
            json(json!({ "title": "Basics", "body": "Start here.", "fingerprint": "f0" }))
        }
        ("GET", "/books/1/pages/11.json") => {
            json(json!({ "title": "Welcome", "body": "Hello from **the** handbook.", "fingerprint": "f1" }))
        }
        ("GET", "/books/1/pages/12.json") => {
            json(json!({ "title": "Long read", "body": paragraphs(), "fingerprint": "f2" }))
        }
        ("GET", "/books/1/pictures/13.json") => json(
            json!({ "title": "Office map", "caption": "Third floor", "image": { "url": "/pictures/map.png" }, "fingerprint": "f3" }),
        ),
        ("GET", "/books/2/pages/20.json") => {
            json(json!({ "title": "Pancakes", "body": "Mix and fry.", "fingerprint": "f4" }))
        }
        ("GET", "/search.json?q=handbook") => json(json!({ "results": [
            { "title": "Welcome", "url": "https://books.example.com/1/handbook/11/welcome", "snippet": "Hello from the handbook", "marked_snippet": "Hello from the <mark>handbook</mark>", "book": { "title": "Handbook" } },
            { "title": "Pancakes", "url": "https://books.example.com/2/recipes/20/pancakes", "snippet": "A handbook of batter", "book": { "title": "Recipes" } }
        ] })),
        ("GET", "/books/1/search.json?q=long") => json(json!({ "results": [
            { "title": "Long read", "url": "https://books.example.com/1/handbook/12/long-read", "snippet": "Paragraph 1", "book": { "title": "Handbook" } }
        ] })),
        ("GET", "/search.json?q=zebra") | ("GET", "/books/1/search.json?q=zebra") => json(json!({ "results": [] })),
        ("GET", "/search.json?q=broken") => reply(500, "boom"),
        _ => not_found(),
    })
}

fn with_second_instance(tui: &mut Tui) {
    let url = tui.server.url.clone();
    tui.app.instances = Loading::Ready(vec![
        Instance {
            alias: "books".to_string(),
            url: url.clone(),
            status: "signed in".to_string(),
        },
        Instance {
            alias: "archive".to_string(),
            url,
            status: "signed in".to_string(),
        },
    ]);
}

#[test]
fn opens_an_instance_then_a_book_then_a_page_and_goes_back_with_esc() {
    let mut tui = library();

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Recipes");
    assert!(screen.contains("books > Books"), "{screen}");
    assert!(screen.contains("How we work"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("4 words");
    assert!(screen.contains("books > Books > Handbook"), "{screen}");
    assert!(screen.contains("Basics"), "{screen}");

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Hello from the handbook.");
    assert!(!screen.contains("**"), "{screen}");

    tui.press(KeyCode::Esc);
    tui.wait_for_request_count(4);
    tui.wait_for("4 words");
    tui.press(KeyCode::Esc);
    tui.wait_for_request_count(5);
    tui.wait_for("Recipes");

    assert_eq!(
        tui.paths(),
        [
            "GET /books.json",
            "GET /books/1/manifest.json",
            "GET /books/1/pages/11.json",
            "GET /books/1/manifest.json",
            "GET /books.json"
        ],
        "going back reloads what was changed meanwhile"
    );
    assert!(
        tui.server
            .requests()
            .iter()
            .all(|it| it.header("Authorization") == Some("Bearer wb_test_token"))
    );
}

#[test]
fn switches_instance_from_the_overlay_on_books_and_on_a_book_but_not_on_a_page() {
    let mut tui = library();
    with_second_instance(&mut tui);

    let screen = tui.screen();
    assert!(screen.contains("› books"), "{screen}");
    assert!(screen.contains("archive"), "{screen}");
    assert!(screen.contains("enter open"), "{screen}");

    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");

    tui.press(KeyCode::Tab);
    let screen = tui.screen();
    assert!(screen.contains("› books"), "{screen}");
    assert!(screen.contains("esc close"), "{screen}");
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("archive > Books");
    assert!(!screen.contains("Writebook ─"), "{screen}");
    tui.wait_for("Recipes");

    tui.press(KeyCode::Enter);
    tui.wait_for("4 words");
    tui.ctrl('i');
    let screen = tui.screen();
    assert!(screen.contains("› books"), "{screen}");
    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(screen.contains("archive > Books > Handbook"), "{screen}");
    assert!(!screen.contains("› books"), "{screen}");

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.wait_for("Hello from the handbook.");
    tui.ctrl('i');
    let screen = tui.screen();
    assert!(
        screen.contains("enter go to page"),
        "the index opens, not the instances:\n{screen}"
    );
    assert!(!screen.contains("› books"), "{screen}");
}

#[test]
fn shows_the_books_as_a_gallery_or_a_list_and_moves_the_selection_between_them() {
    let mut tui = library();
    tui.press(KeyCode::Enter);

    let screen = tui.wait_for("Recipes");
    assert!(screen.contains("Handbook"), "{screen}");
    assert!(screen.contains("How we work"), "{screen}");

    let screen = tui.screen();
    for key in [
        "←↑↓→ select",
        "enter open",
        "v list view",
        "/ search",
        "n new book",
        "s settings",
        "esc instances",
    ] {
        assert!(screen.contains(key), "{key}\n{screen}");
    }

    tui.press(KeyCode::Char('v'));
    let screen = tui.screen();
    assert!(screen.contains("v gallery view"), "{screen}");
    assert!(screen.contains("Handbook"), "{screen}");
    assert!(screen.contains("Recipes"), "{screen}");

    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Pancakes");
    assert!(screen.contains("books > Books > Recipes"), "{screen}");

    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    let screen = tui.screen();
    assert!(screen.contains("v gallery view"), "the view stays as chosen:\n{screen}");

    tui.press(KeyCode::Char('v'));
    tui.press(KeyCode::Up);
    tui.press(KeyCode::Enter);
    tui.wait_for("Office map");

    assert_eq!(
        tui.paths(),
        [
            "GET /books.json",
            "GET /books/2/manifest.json",
            "GET /books.json",
            "GET /books/1/manifest.json"
        ]
    );
}

#[test]
fn offers_editing_keys_on_an_editable_book_and_only_reading_keys_on_a_read_only_one() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");

    tui.press(KeyCode::Enter);
    tui.wait_for("Office map");
    let screen = tui.screen();
    for key in [
        "a add",
        "d delete",
        "m move",
        "e edit book",
        "p publishing",
        "v list view",
        "/ search",
        "esc books",
    ] {
        assert!(screen.contains(key), "{key}\n{screen}");
    }

    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.wait_for("2 words");
    let screen = tui.screen();
    for key in ["enter open", "v list view", "/ search", "esc books"] {
        assert!(screen.contains(key), "{key}\n{screen}");
    }
    for key in ["a add", "d delete", "m move", "e edit book", "p publishing"] {
        assert!(!screen.contains(key), "{key}\n{screen}");
    }

    tui.press(KeyCode::Char('d'));
    tui.press(KeyCode::Char('a'));
    tui.press(KeyCode::Char('e'));
    let screen = tui.screen();
    assert!(screen.contains("books > Books > Recipes"), "{screen}");
    assert!(!screen.contains("Delete"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Mix and fry.");
    assert!(!screen.contains("d delete"), "{screen}");
    assert!(!screen.contains("e edit"), "{screen}");
    assert_eq!(tui.paths().last().unwrap(), "GET /books/2/pages/20.json");
}

#[test]
fn lists_a_books_sections_pages_and_pictures_in_either_view_and_opens_each_kind() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);

    let screen = tui.wait_for("Office map");
    for title in ["Basics", "Welcome", "Long read", "Office map", "4 words", "120 words"] {
        assert!(screen.contains(title), "{title}\n{screen}");
    }

    tui.press(KeyCode::Char('v'));
    let screen = tui.screen();
    for title in ["Basics", "Welcome", "Long read", "Office map", "4 words", "120 words"] {
        assert!(screen.contains(title), "{title}\n{screen}");
    }
    assert!(screen.contains("v gallery view"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Start here.");
    assert!(screen.contains("1 of 4"), "{screen}");

    tui.press(KeyCode::Esc);
    tui.wait_for("Office map");
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("4 of 4");
    assert!(screen.contains("books > Books > Handbook > Office map"), "{screen}");
    assert!(screen.contains("Loading the picture…"), "{screen}");
}

#[test]
fn scrolls_a_long_page_and_turns_to_the_next_and_previous_page() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    tui.wait_for("Office map");

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Paragraph 1");
    assert!(screen.contains("3 of 4"), "{screen}");
    assert!(!screen.contains("Paragraph 40"), "{screen}");

    tui.press(KeyCode::PageDown);
    let screen = tui.screen();
    assert!(!screen.contains("Paragraph 1\n"), "{screen}");
    assert!(screen.contains("Paragraph 12"), "{screen}");

    tui.press(KeyCode::Char('j'));
    tui.press(KeyCode::Char('j'));
    tui.press(KeyCode::Char('k'));
    for _ in 0..10 {
        tui.press(KeyCode::PageDown);
    }
    let screen = tui.screen();
    assert!(screen.contains("Paragraph 60"), "{screen}");

    tui.press(KeyCode::Home);
    let screen = tui.screen();
    assert!(screen.contains("Paragraph 1\n"), "{screen}");

    tui.press(KeyCode::Right);
    let screen = tui.wait_for("4 of 4");
    assert!(screen.contains("Office map"), "{screen}");

    tui.press(KeyCode::Left);
    let screen = tui.wait_for("Paragraph 1");
    assert!(screen.contains("3 of 4"), "{screen}");
    tui.press(KeyCode::Left);
    tui.wait_for("Hello from the handbook.");
    tui.press(KeyCode::Left);
    tui.wait_for("Start here.");

    tui.press(KeyCode::Left);
    let screen = tui.wait_for("Office map");
    assert!(screen.contains("books > Books > Handbook"), "{screen}");
    assert!(!screen.contains("1 of 4"), "{screen}");
}

#[test]
fn jumps_to_a_page_from_the_index_overlay() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    tui.wait_for("Office map");
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.wait_for("Hello from the handbook.");

    tui.press(KeyCode::Tab);
    let screen = tui.screen();
    for title in ["Handbook", "Basics", "› Welcome", "Long read", "Office map", "◂"] {
        assert!(screen.contains(title), "{title}\n{screen}");
    }
    assert!(screen.contains("go to page"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(!screen.contains("Long read"), "{screen}");

    tui.ctrl('i');
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("4 of 4");
    assert!(screen.contains("books > Books > Handbook > Office map"), "{screen}");
    assert!(!screen.contains("go to page"), "{screen}");
    tui.wait_for_request("GET", "/books/1/pictures/13.json");
}

#[test]
fn searches_every_book_and_opens_a_result_in_its_book() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");

    tui.press(KeyCode::Char('/'));
    let screen = tui.screen();
    assert!(screen.contains("Search all books"), "{screen}");
    assert!(screen.contains("Type what to look for, then press enter."), "{screen}");

    tui.press(KeyCode::Enter);
    assert_eq!(tui.paths(), ["GET /books.json"], "nothing typed, nothing searched");

    tui.type_text("handbook");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Hello from the handbook");
    assert!(screen.contains("Handbook · Welcome:"), "{screen}");
    assert!(screen.contains("Recipes · Pancakes: A handbook of batter"), "{screen}");
    assert!(screen.contains("enter open"), "{screen}");
    assert_eq!(tui.paths().last().unwrap(), "GET /search.json?q=handbook");

    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    tui.wait_for("Pancakes");
    let screen = tui.wait_for("2 words");
    assert!(screen.contains("books > Books > Recipes"), "{screen}");
    assert!(!screen.contains("Search all books"), "{screen}");

    tui.press(KeyCode::Enter);
    tui.wait_for("Mix and fry.");
}

#[test]
fn searches_inside_a_book_and_says_when_nothing_matches_or_the_search_fails() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    tui.wait_for("Office map");

    tui.press(KeyCode::Char('/'));
    tui.type_text("zebra");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Nothing matches.");
    assert!(screen.contains("Search “Handbook”"), "{screen}");
    assert_eq!(tui.paths().last().unwrap(), "GET /books/1/search.json?q=zebra");

    for _ in 0..5 {
        tui.press(KeyCode::Backspace);
    }
    tui.type_text("long");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Long read: Paragraph 1");
    assert!(!screen.contains("Handbook ·"), "{screen}");

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Long read");
    assert!(!screen.contains("Search"), "{screen}");
    tui.press(KeyCode::Enter);
    tui.wait_for("Paragraph 1");

    tui.press(KeyCode::Esc);
    tui.wait_for("Office map");
    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Char('/'));
    tui.type_text("broken");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("answered 500 for /search.json");
    assert!(screen.contains("Search all books"), "{screen}");

    tui.press(KeyCode::Enter);
    tui.wait_for_request_count(tui.paths().len() + 1);
    tui.wait_for("answered 500 for /search.json");
    let searches = tui.paths().iter().filter(|it| it.ends_with("q=broken")).count();
    assert_eq!(searches, 2, "enter searches again after a failure");

    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(!screen.contains("Search all books"), "{screen}");
}

#[test]
fn shows_a_notice_instead_of_crashing_when_the_server_fails_or_answers_with_something_unreadable() {
    let mut tui = Tui::new(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(
            json!([{ "id": 1, "title": "Handbook", "editable": true }, { "id": 2, "title": "Recipes", "editable": true }]),
        ),
        ("GET", "/books/1/manifest.json") => reply(500, "boom"),
        ("GET", "/books/2/manifest.json") => json(json!({
            "book": { "title": "Recipes", "slug": "recipes", "editable": true },
            "leaves": [{ "id": 20, "type": "Page", "title": "Pancakes", "word_count": 2 }]
        })),
        ("GET", "/books/2/pages/20.json") => reply(200, "this is not json"),
        _ => not_found(),
    });

    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("500");
    assert!(screen.contains("books > Books > …"), "{screen}");
    assert!(!screen.contains("Pancakes"), "{screen}");

    tui.press(KeyCode::Esc);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    tui.wait_for("2 words");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Loading…");
    assert!(screen.contains("books > Books > Recipes > Pancakes"), "{screen}");
    assert!(!screen.contains("this is not json"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("2 words");
    assert!(screen.contains("books > Books > Recipes"), "{screen}");
}

#[test]
fn shows_an_instance_whose_server_fails_and_goes_back_to_the_instances() {
    let mut tui = Tui::new(|_| reply(500, "boom"));

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("500");
    assert!(screen.contains("books > Books"), "{screen}");

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("enter open");
    assert!(screen.contains("› books"), "{screen}");
    assert_eq!(tui.paths(), ["GET /books.json"]);
}

#[test]
fn drops_a_slow_reply_that_arrives_after_the_user_left_its_screen() {
    let mut tui = Tui::new(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(
            json!([{ "id": 1, "title": "Handbook", "editable": true }, { "id": 2, "title": "Recipes", "editable": true }]),
        ),
        ("GET", "/books/1/manifest.json") => {
            std::thread::sleep(Duration::from_millis(500));
            json(json!({
                "book": { "title": "Handbook", "slug": "handbook", "editable": true },
                "leaves": [{ "id": 10, "type": "Section", "title": "Late section" }]
            }))
        }
        ("GET", "/books/2/manifest.json") => json(json!({
            "book": { "title": "Recipes", "slug": "recipes", "editable": true },
            "leaves": [{ "id": 20, "type": "Page", "title": "Pancakes", "word_count": 2 }]
        })),
        _ => not_found(),
    });

    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    tui.press(KeyCode::Esc);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    tui.wait_for_request_count(4);
    tui.settle();

    let screen = tui.screen();
    assert!(screen.contains("books > Books > Recipes"), "{screen}");
    assert!(screen.contains("2 words"), "{screen}");
    assert!(!screen.contains("Late section"), "{screen}");
    let mut paths = tui.paths();
    paths.sort();
    assert_eq!(
        paths,
        [
            "GET /books.json",
            "GET /books.json",
            "GET /books/1/manifest.json",
            "GET /books/2/manifest.json"
        ]
    );
}

#[test]
fn toggles_help_and_asks_before_quitting_from_deep_in_a_book() {
    let mut tui = library();
    tui.press(KeyCode::Enter);
    tui.wait_for("Recipes");
    tui.press(KeyCode::Enter);
    tui.wait_for("Office map");
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Hello from the handbook.");
    for key in [
        "←→ turn pages",
        "↑↓ scroll",
        "e edit",
        "d delete",
        "esc book",
        "ctrl+i index",
        "ctrl+c quit",
        "? toggle help",
    ] {
        assert!(screen.contains(key), "{key}\n{screen}");
    }

    tui.press(KeyCode::Char('?'));
    let screen = tui.screen();
    assert!(screen.contains("? toggle help"), "{screen}");
    assert!(!screen.contains("turn pages"), "{screen}");

    tui.press(KeyCode::Char('?'));
    let screen = tui.screen();
    assert!(screen.contains("←→ turn pages"), "{screen}");

    tui.ctrl('c');
    let screen = tui.screen();
    assert!(screen.contains("Quit writebook tui?"), "{screen}");
    assert!(!tui.app.done);

    tui.press(KeyCode::Esc);
    let screen = tui.screen();
    assert!(!screen.contains("Quit writebook tui?"), "{screen}");
    assert!(screen.contains("books > Books > Handbook > Welcome"), "{screen}");
    assert!(!tui.app.done);

    tui.ctrl('c');
    tui.ctrl('c');
    assert!(tui.app.done);
}

fn read_only_library() -> Tui {
    Tui::read_only(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/books.json") => json(json!([{ "id": 1, "title": "Handbook", "editable": true }])),
        ("GET", "/books/1/manifest.json") => json(json!({
            "book": { "title": "Handbook", "slug": "handbook", "published": false, "editable": true },
            "leaves": [{ "id": 11, "type": "Page", "title": "Welcome", "word_count": 4 }]
        })),
        _ => not_found(),
    })
}

#[test]
fn a_read_only_token_offers_no_way_to_change_a_book_it_could_otherwise_edit() {
    let mut tui = read_only_library();

    tui.press(KeyCode::Enter);
    tui.wait_for("Handbook");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("4 words");
    for key in ["a add", "d delete", "m move", "e edit book", "p publishing"] {
        assert!(!screen.contains(key), "{key}\n{screen}");
    }

    tui.press(KeyCode::Char('d'));
    tui.press(KeyCode::Char('e'));
    let screen = tui.screen();
    assert!(!screen.contains("Delete"), "{screen}");
    assert_eq!(tui.paths(), ["GET /books.json", "GET /books/1/manifest.json"]);
}

#[test]
fn a_read_only_token_offers_no_new_book() {
    let mut tui = read_only_library();

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("Handbook");
    assert!(!screen.contains("n new book"), "{screen}");

    tui.press(KeyCode::Char('n'));
    let screen = tui.screen();
    assert!(!screen.contains("New book"), "{screen}");
    assert_eq!(tui.paths(), ["GET /books.json"]);
}
