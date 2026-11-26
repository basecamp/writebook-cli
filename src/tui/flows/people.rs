use std::sync::{Arc, Mutex};

use crossterm::event::KeyCode;
use serde_json::json;

use super::{Tui, body, json, not_found, reply};

struct Workplace {
    people: Vec<(u64, &'static str, String)>,
    invite: &'static str,
    can_administer: bool,
    failing: bool,
    profile_email: String,
    wrong_password: bool,
}

fn workplace(can_administer: bool) -> (Tui, Arc<Mutex<Workplace>>) {
    let state = Arc::new(Mutex::new(Workplace {
        people: vec![
            (1, "Me", "administrator".to_string()),
            (2, "Kevin", "member".to_string()),
            (3, "Jason", "member".to_string()),
        ],
        invite: "abc",
        can_administer,
        failing: false,
        profile_email: "me@example.com".to_string(),
        wrong_password: false,
    }));
    let shared = Arc::clone(&state);

    let tui = Tui::new(move |request| {
        let mut state = shared.lock().unwrap();
        let host = request.header("Host").unwrap().to_string();
        let write = matches!(request.method.as_str(), "PATCH" | "DELETE" | "POST");

        if write && state.failing && request.path != "/users/1/profile.json" {
            reply(403, r#"{"error":"Only administrators can do that"}"#)
        } else {
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/books.json") => json(json!([{ "id": 1, "title": "Handbook", "editable": true }])),
                ("GET", "/users.json") => {
                    let users: Vec<_> = state
                        .people
                        .iter()
                        .map(|(id, name, role)| json!({ "id": id, "name": name, "role": role, "current": *id == 1 }))
                        .collect();
                    json(
                        json!({ "users": users, "join_url": format!("http://{host}/join/{}", state.invite), "can_administer": state.can_administer }),
                    )
                }
                ("PATCH", path) if path.starts_with("/users/") && path.ends_with("/profile.json") => {
                    if state.wrong_password {
                        reply(422, r#"{"errors":["Current password is incorrect"]}"#)
                    } else {
                        state.profile_email = body(request)["user"]["email_address"].as_str().unwrap().to_string();
                        json(json!({}))
                    }
                }
                ("PATCH", path) if path.starts_with("/users/") => {
                    let id: u64 = path
                        .trim_start_matches("/users/")
                        .trim_end_matches(".json")
                        .parse()
                        .unwrap();
                    let role = body(request)["user"]["role"].as_str().unwrap().to_string();
                    state.people.iter_mut().find(|it| it.0 == id).unwrap().2 = role;
                    json(json!({}))
                }
                ("DELETE", path) if path.starts_with("/users/") => {
                    let id: u64 = path
                        .trim_start_matches("/users/")
                        .trim_end_matches(".json")
                        .parse()
                        .unwrap();
                    state.people.retain(|it| it.0 != id);
                    json(json!({}))
                }
                ("POST", "/account/join_code.json") => {
                    state.invite = "xyz";
                    json(json!({ "join_url": format!("http://{host}/join/xyz") }))
                }
                ("GET", "/users/1/profile/edit.json") => {
                    json(json!({ "name": "Me", "email_address": state.profile_email }))
                }
                _ => not_found(),
            }
        }
    });

    (tui, state)
}

fn settings(can_administer: bool) -> (Tui, Arc<Mutex<Workplace>>) {
    let (mut tui, state) = workplace(can_administer);
    tui.press(KeyCode::Enter);
    tui.wait_for("Handbook");
    tui.press(KeyCode::Char('s'));
    tui.wait_for("Kevin");
    (tui, state)
}

fn row(screen: &str, name: &str) -> String {
    screen
        .lines()
        .find(|it| it.contains(name))
        .unwrap_or_else(|| panic!("no row for {name}\n{screen}"))
        .trim()
        .to_string()
}

#[test]
fn opens_settings_from_the_books_with_people_roles_and_the_invite_link_and_esc_goes_back() {
    let (mut tui, _) = settings(true);

    let screen = tui.screen();
    assert!(screen.contains("books > Settings"), "{screen}");
    assert!(screen.contains("Share to invite more people"), "{screen}");
    assert!(screen.contains(&format!("{}/join/abc", tui.server.url)), "{screen}");
    assert!(row(&screen, "Me (you)").ends_with("administrator"), "{screen}");
    assert!(row(&screen, "Kevin").ends_with("‹ member ›"), "{screen}");
    assert!(row(&screen, "Jason").ends_with("‹ member ›"), "{screen}");
    for binding in ["your profile", "copy invite", "QR code", "new invite link", "esc books"] {
        assert!(screen.contains(binding), "{screen}");
    }

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for_gone("Share to invite more people");
    assert!(screen.contains("books > Books"), "{screen}");
    assert!(screen.contains("Handbook"), "{screen}");
    assert_eq!(tui.paths(), ["GET /books.json", "GET /users.json"]);
}

#[test]
fn an_administrator_makes_someone_an_administrator_and_back() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Down);
    tui.press(KeyCode::Right);
    let screen = tui.wait_for("‹ administrator ›");
    assert!(row(&screen, "Kevin").ends_with("‹ administrator ›"), "{screen}");

    tui.press(KeyCode::Left);
    let screen = tui.wait_for_gone("‹ administrator ›");
    assert!(row(&screen, "Kevin").ends_with("‹ member ›"), "{screen}");

    let requests = tui.server.requests();
    assert_eq!(
        tui.paths(),
        [
            "GET /books.json",
            "GET /users.json",
            "PATCH /users/2.json",
            "GET /users.json",
            "PATCH /users/2.json",
            "GET /users.json"
        ]
    );
    assert_eq!(body(&requests[2]), json!({ "user": { "role": "administrator" } }));
    assert_eq!(body(&requests[4]), json!({ "user": { "role": "member" } }));
}

#[test]
fn removes_someone_only_after_answering_yes_to_the_question() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Down);
    tui.press(KeyCode::Char('d'));
    let screen = tui.wait_for("Remove Kevin?");
    assert!(screen.contains("They won't be able to sign in any more."), "{screen}");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Remove Kevin?");
    assert!(screen.contains("Kevin"), "{screen}");

    tui.press(KeyCode::Char('d'));
    tui.wait_for("Remove Kevin?");
    tui.press(KeyCode::Esc);
    let screen = tui.wait_for_gone("Remove Kevin?");
    assert!(screen.contains("Kevin"), "{screen}");

    tui.press(KeyCode::Char('d'));
    tui.wait_for("Remove Kevin?");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Kevin");
    assert!(screen.contains("Jason"), "{screen}");

    assert_eq!(
        tui.paths(),
        [
            "GET /books.json",
            "GET /users.json",
            "DELETE /users/2.json",
            "GET /users.json"
        ]
    );
}

#[test]
fn resets_the_invite_link_only_after_answering_yes_and_shows_the_new_one() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Char('r'));
    let screen = tui.wait_for("Make a new invite link?");
    assert!(screen.contains("The current link will stop working."), "{screen}");
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Make a new invite link?");
    assert!(screen.contains("/join/abc"), "{screen}");

    tui.press(KeyCode::Char('r'));
    tui.wait_for("Make a new invite link?");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("The old invite link no longer works.");
    assert!(screen.contains(&format!("{}/join/xyz", tui.server.url)), "{screen}");
    assert!(!screen.contains("/join/abc"), "{screen}");

    assert_eq!(
        tui.paths(),
        ["GET /books.json", "GET /users.json", "POST /account/join_code.json"]
    );
    assert_eq!(body(&tui.server.requests()[2]), json!({}));
}

#[test]
fn copies_the_invite_link_and_says_so() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Char('c'));
    let screen = tui.wait_for("Copied");
    assert!(
        screen.contains(&format!("Copied {}/join/abc", tui.server.url)),
        "{screen}"
    );
}

#[test]
fn a_member_can_copy_the_invite_link_but_not_change_people_or_reset_it() {
    let (mut tui, _) = settings(false);

    let screen = tui.screen();
    assert!(!screen.contains("‹"), "{screen}");
    assert!(!screen.contains("remove"), "{screen}");
    assert!(!screen.contains("new invite link"), "{screen}");
    assert!(screen.contains("copy invite"), "{screen}");

    tui.press(KeyCode::Down);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Char('d'));
    tui.press(KeyCode::Char('r'));
    let screen = tui.screen();
    assert!(!screen.contains("Remove Kevin?"), "{screen}");
    assert!(!screen.contains("Make a new invite link?"), "{screen}");

    tui.press(KeyCode::Char('c'));
    tui.wait_for("Copied");
    assert_eq!(tui.paths(), ["GET /books.json", "GET /users.json"]);
}

#[test]
fn nobody_changes_their_own_role_or_removes_themselves() {
    let (mut tui, state) = settings(true);

    let screen = tui.screen();
    assert!(screen.contains("› Me (you)"), "{screen}");
    assert!(screen.contains("enter your profile"), "{screen}");
    assert!(!screen.contains("remove"), "{screen}");

    tui.press(KeyCode::Right);
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Char('d'));
    let screen = tui.screen();
    assert!(!screen.contains("Remove Me?"), "{screen}");
    assert!(row(&screen, "Me (you)").ends_with("administrator"), "{screen}");

    tui.press(KeyCode::Down);
    let screen = tui.screen();
    assert!(screen.contains("remove"), "{screen}");
    assert!(!screen.contains("your profile"), "{screen}");

    assert_eq!(tui.paths(), ["GET /books.json", "GET /users.json"]);
    assert_eq!(state.lock().unwrap().people.len(), 3);
}

#[test]
fn a_server_error_on_a_change_shows_a_notice_and_keeps_the_list_as_it_was() {
    let (mut tui, state) = settings(true);
    state.lock().unwrap().failing = true;
    let refused = "the server answered 403: Only administrators can do that";

    tui.press(KeyCode::Down);
    tui.press(KeyCode::Right);
    let screen = tui.wait_for(refused);
    assert!(row(&screen, "Kevin").ends_with("‹ member ›"), "{screen}");

    tui.press(KeyCode::Char('d'));
    tui.wait_for("Remove Kevin?");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for(refused);
    assert!(screen.contains("Kevin"), "{screen}");

    tui.press(KeyCode::Char('r'));
    tui.wait_for("Make a new invite link?");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for(refused);
    assert!(screen.contains("/join/abc"), "{screen}");
    assert!(!screen.contains("The old invite link no longer works."), "{screen}");

    tui.wait_for_request_count(7);
    let state = state.lock().unwrap();
    assert_eq!(state.people.len(), 3);
    assert_eq!(state.people[1].2, "member");
    assert_eq!(state.invite, "abc");
}

#[test]
fn changes_only_the_name_without_asking_for_the_current_password() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Enter);
    let screen = tui.wait_for("books > Settings > Me");
    assert!(screen.contains("me@example.com"), "{screen}");
    assert!(screen.contains("esc settings"), "{screen}");

    tui.type_text(" Two");
    let screen = tui.screen();
    assert!(screen.contains("Me Two"), "{screen}");
    assert!(!screen.contains("Current password"), "{screen}");
    tui.ctrl('s');
    tui.wait_for("Saved.");

    let requests = tui.server.requests();
    let saved = requests.iter().find(|it| it.method == "PATCH").unwrap();
    assert_eq!(saved.path, "/users/1/profile.json");
    assert_eq!(
        body(saved),
        json!({ "user": { "name": "Me Two", "email_address": "me@example.com", "current_password": "" } })
    );

    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("Share to invite more people");
    assert!(screen.contains("books > Settings"), "{screen}");
    tui.press(KeyCode::Esc);
    let screen = tui.wait_for("Handbook");
    assert!(screen.contains("books > Books"), "{screen}");
}

#[test]
fn changing_the_email_or_password_needs_the_current_password_and_sends_it() {
    let (mut tui, state) = settings(true);

    tui.press(KeyCode::Enter);
    tui.wait_for("books > Settings > Me");
    tui.press(KeyCode::Tab);
    for _ in 0..3 {
        tui.press(KeyCode::Backspace);
    }
    tui.type_text("org");
    let screen = tui.screen();
    assert!(screen.contains("me@example.org"), "{screen}");
    assert!(screen.contains("Current password"), "{screen}");

    tui.ctrl('s');
    let screen = tui.wait_for("Fill in your current password to change your email or password.");
    assert!(!tui.paths().iter().any(|it| it.starts_with("PATCH")), "{screen}");

    tui.press(KeyCode::Tab);
    tui.type_text("newsecret");
    tui.press(KeyCode::Tab);
    tui.type_text("oldsecret");
    let screen = tui.screen();
    assert!(
        !screen.contains("newsecret") && !screen.contains("oldsecret"),
        "{screen}"
    );
    tui.ctrl('s');
    tui.wait_for("Saved.");

    let requests = tui.server.requests();
    let patches: Vec<_> = requests.iter().filter(|it| it.method == "PATCH").collect();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].path, "/users/1/profile.json");
    assert_eq!(
        body(patches[0]),
        json!({ "user": { "name": "Me", "email_address": "me@example.org", "current_password": "oldsecret", "password": "newsecret" } })
    );
    assert_eq!(state.lock().unwrap().profile_email, "me@example.org");
}

#[test]
fn a_wrong_current_password_shows_the_servers_reason_and_keeps_the_form() {
    let (mut tui, state) = settings(true);
    state.lock().unwrap().wrong_password = true;

    tui.press(KeyCode::Enter);
    tui.wait_for("books > Settings > Me");
    tui.press(KeyCode::Tab);
    tui.type_text("x");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.type_text("guess");
    tui.ctrl('s');

    let screen = tui.wait_for("Current password is incorrect");
    assert!(screen.contains("books > Settings > Me"), "{screen}");
    assert!(screen.contains("me@example.comx"), "{screen}");
    assert!(screen.contains("•••••"), "{screen}");

    state.lock().unwrap().wrong_password = false;
    tui.ctrl('s');
    tui.wait_for("Saved.");
}

#[test]
fn rows_line_up_with_wide_names_and_the_selection_stays_on_someone_when_the_list_shrinks() {
    let (mut tui, state) = workplace(true);
    {
        let mut state = state.lock().unwrap();
        state.people.push((4, "山田太郎", "member".to_string()));
        for id in 5..=40 {
            let name: &'static str = Box::leak(format!("Person {id}").into_boxed_str());
            state.people.push((id, name, "member".to_string()));
        }
    }
    tui.press(KeyCode::Enter);
    tui.wait_for("Handbook");
    tui.press(KeyCode::Char('s'));
    let screen = tui.wait_for("Kevin");
    assert_eq!(
        row(&screen, "山").chars().count(),
        row(&screen, "Kevin").chars().count(),
        "{screen}"
    );

    for _ in 0..39 {
        tui.press(KeyCode::Down);
    }
    let screen = tui.screen();
    assert!(row(&screen, "Person 40").starts_with("›"), "{screen}");

    tui.press(KeyCode::Char('d'));
    tui.wait_for("Remove Person 40?");
    tui.press(KeyCode::Left);
    tui.press(KeyCode::Enter);
    let screen = tui.wait_for_gone("Person 40");
    assert!(row(&screen, "Person 39").starts_with("›"), "{screen}");
}

#[test]
fn enter_moves_on_through_the_profile_and_saves_from_the_button() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Enter);
    tui.wait_for("books > Settings > Me");
    for _ in 0..3 {
        tui.press(KeyCode::Enter);
    }
    assert!(!tui.paths().iter().any(|it| it.starts_with("PATCH")));

    tui.press(KeyCode::Enter);
    tui.wait_for("Saved.");
    assert_eq!(tui.paths().iter().filter(|it| it.starts_with("PATCH")).count(), 1);
}

#[test]
fn tab_moves_through_the_profile_and_only_ctrl_i_opens_the_instances() {
    let (mut tui, _) = settings(true);

    tui.press(KeyCode::Enter);
    tui.wait_for("books > Settings > Me");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Char('!'));
    let screen = tui.screen();
    assert!(screen.contains("me@example.com!"), "{screen}");
    assert!(!screen.contains("esc close"), "{screen}");

    tui.ctrl('i');
    let screen = tui.screen();
    assert!(screen.contains("esc close"), "{screen}");
}
