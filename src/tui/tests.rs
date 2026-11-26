use super::*;
use crate::leaf_kind::LeafKind;

const HOST: &str = "https://books.example.com";

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_c() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
}

#[test]
fn q_doesnt_quit_and_ctrl_c_asks_first() {
    let mut app = App::new(Images::none());

    app.key(press(KeyCode::Char('q')));
    assert!(!app.done);
    assert!(app.overlay.is_none());

    app.key(ctrl_c());
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm(Confirm {
            question: Question::Quit(_),
            ..
        }))
    ));
    assert!(!app.done);

    app.key(press(KeyCode::Esc));
    assert!(app.overlay.is_none());
    assert!(!app.done);

    app.key(ctrl_c());
    app.key(press(KeyCode::Char('y')));
    assert!(!app.done, "y is no answer; the choice is");

    app.key(press(KeyCode::Right));
    app.key(press(KeyCode::Enter));
    assert!(!app.done, "No was chosen");
    assert!(app.overlay.is_none());

    app.key(ctrl_c());
    app.key(press(KeyCode::Enter));
    assert!(app.done, "Yes is chosen to start with");
}

#[test]
fn a_second_ctrl_c_quits_and_staying_brings_back_the_open_overlay() {
    let mut app = App::new(Images::none());
    app.overlay = Some(Overlay::Confirm(Confirm::new(Question::Discard)));

    app.key(ctrl_c());
    app.key(press(KeyCode::Esc));
    assert!(matches!(
        app.overlay,
        Some(Overlay::Confirm(Confirm {
            question: Question::Discard,
            ..
        }))
    ));

    app.key(ctrl_c());
    app.key(ctrl_c());
    assert!(app.done);
}

fn form(id: Option<u64>, title: &str) -> BookForm {
    BookForm {
        id,
        title: title.to_string(),
        subtitle: String::new(),
        author: String::new(),
        theme: "blue".to_string(),
        everyone: false,
        cover: None,
        new_cover: None,
        remove_cover: false,
        members: Vec::new(),
    }
}

fn leaf(id: u64) -> data::Leaf {
    data::Leaf {
        id,
        kind: LeafKind::Page,
        title: format!("Page {id}"),
        word_count: None,
        preview: None,
        image: None,
    }
}

fn book() -> data::Book {
    data::Book {
        id: 1,
        title: "Handbook".to_string(),
        subtitle: None,
        author: None,
        theme: None,
        cover: None,
        slug: "handbook".to_string(),
        published: false,
        editable: true,
        leaves: vec![leaf(1), leaf(2)],
    }
}

fn books() -> Books {
    let mut books = Books::new(HOST.to_string(), "books.example.com".to_string());
    let card = data::BookCard {
        id: 1,
        title: "Handbook".to_string(),
        subtitle: None,
        author: None,
        theme: None,
        cover: None,
        editable: true,
    };
    books.books = Loading::Ready(vec![card.clone(), card]);
    books
}

fn book_page() -> BookPage {
    let mut page = BookPage::new(books(), 1);
    page.book = Loading::Ready(book());
    page
}

fn reading_page(leaf_id: u64) -> LeafPage {
    let book = book();
    let index = book.leaves.iter().position(|it| it.id == leaf_id).unwrap();
    let mut page = LeafPage::new(HOST.to_string(), 1, &book, index);
    page.loaded(Ok(data::LeafContent {
        title: "Page".to_string(),
        body: "# Title\n\nSome text, a list:\n\n- one\n- > two\n\n![a](/a.png)\n\n| a | b |\n|---|---|\n| 1 | 2 |"
            .to_string(),
        image: None,
        fingerprint: "f1".to_string(),
        record_gid: Some("gid://writebook/ActionText::RichText/1".to_string()),
    }));
    page
}

fn editing_page(leaf_id: u64) -> Screen {
    let mut page = reading_page(leaf_id);
    page.editor = Some(editor::Editor::new("Page", ""));
    Screen::Leaf(Box::new(page), Box::new(book_page()))
}

fn edited_body(app: &App) -> String {
    match &app.screen {
        Screen::Leaf(page, _) => page.editor.as_ref().unwrap().body(),
        _ => panic!("not on a page"),
    }
}

fn leaf_page(app: &mut App) -> &mut LeafPage {
    match &mut app.screen {
        Screen::Leaf(page, _) => page,
        _ => panic!("not on a page"),
    }
}

fn publication(published: bool) -> Publication {
    Publication {
        published,
        slug: "handbook".to_string(),
        url: format!("{HOST}/1/handbook"),
    }
}

#[test]
fn a_form_goes_only_into_the_editor_it_was_loaded_for() {
    let mut app = App::new(Images::none());
    let page = BookFormPage::new(HOST.to_string(), "books.example.com".to_string(), None);
    let id = page.id;
    app.screen = Screen::BookForm(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));

    app.receive(Message::To(
        next_id(),
        Reply::BookFormLoaded(Ok(form(Some(9), "Someone else's"))),
    ));
    assert!(matches!(&app.screen, Screen::BookForm(page, _) if matches!(page.form, Loading::Pending)));

    app.receive(Message::To(id, Reply::BookFormLoaded(Ok(form(None, "")))));
    assert!(matches!(&app.screen, Screen::BookForm(page, _) if page.form.ready().is_some_and(|it| it.id.is_none())));
}

#[test]
fn a_save_or_delete_finishing_leaves_another_editor_alone() {
    let mut app = App::new(Images::none());
    let mut page = BookFormPage::new(HOST.to_string(), "books.example.com".to_string(), Some(1));
    page.form = Loading::Ready(form(Some(1), "Handbook"));
    let id = page.id;
    app.screen = Screen::BookForm(Box::new(page), Box::new(Screen::Book(Box::new(book_page()))));

    app.receive(Message::To(next_id(), Reply::BookSaved(Ok(7))));
    app.receive(Message::To(next_id(), Reply::BookDeleted(Ok(()))));
    assert!(matches!(&app.screen, Screen::BookForm(page, _) if page.id == id));

    match &mut app.screen {
        Screen::BookForm(page, _) => page.saving = true,
        _ => panic!("the editor closed"),
    }
    app.receive(Message::To(id, Reply::BookSaved(Err(anyhow::anyhow!("offline")))));
    assert!(matches!(&app.screen, Screen::BookForm(page, _) if !page.saving));
    assert_eq!(app.notice.as_deref(), Some("offline"));
}

#[test]
fn a_page_loads_only_into_the_page_it_was_loaded_for() {
    let mut app = App::new(Images::none());
    app.screen = Screen::Leaf(Box::new(reading_page(1)), Box::new(book_page()));
    let id = leaf_page(&mut app).id;
    let content = |body: &str| data::LeafContent {
        title: "Page".to_string(),
        body: body.to_string(),
        image: None,
        fingerprint: "f2".to_string(),
        record_gid: None,
    };

    app.receive(Message::To(next_id(), Reply::LeafLoaded(Ok(content("Elsewhere")))));
    assert_eq!(leaf_page(&mut app).content.ready().unwrap().fingerprint, "f1");

    app.receive(Message::To(id, Reply::LeafLoaded(Ok(content("Here")))));
    assert_eq!(leaf_page(&mut app).content.ready().unwrap().body, "Here");
}

#[test]
fn an_upload_goes_only_into_the_page_it_was_for() {
    let mut app = App::new(Images::none());
    app.screen = editing_page(2);
    let id = leaf_page(&mut app).id;
    let _request = leaf_page(&mut app).upload(PathBuf::from("b.png")).unwrap();

    app.receive(Message::To(
        next_id(),
        Reply::Uploaded(Ok("![a.png](/a.png)".to_string())),
    ));
    assert_eq!(edited_body(&app), "");
    assert!(app.notice.is_some());

    app.receive(Message::To(id, Reply::Uploaded(Ok("![b.png](/b.png)".to_string()))));
    assert!(edited_body(&app).contains("![b.png](/b.png)"));
}

#[test]
fn a_page_waits_for_its_uploads_before_saving() {
    let mut app = App::new(Images::none());
    app.screen = editing_page(1);
    let id = leaf_page(&mut app).id;
    let _request = leaf_page(&mut app).upload(PathBuf::from("a.png")).unwrap();

    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(!leaf_page(&mut app).saving);
    assert_eq!(app.notice.as_deref(), Some("Wait for the upload to finish, then save."));

    app.receive(Message::To(id, Reply::Uploaded(Ok("![a.png](/a.png)".to_string()))));
    assert!(edited_body(&app).contains("![a.png](/a.png)"));
}

#[test]
fn pasting_while_a_page_saves_changes_nothing() {
    let mut app = App::new(Images::none());
    app.screen = editing_page(1);
    leaf_page(&mut app).saving = true;

    app.paste("too late");
    assert_eq!(edited_body(&app), "");
}

#[test]
fn books_load_while_settings_are_open_over_them() {
    let mut app = App::new(Images::none());
    let mut books = Books::new(HOST.to_string(), "books.example.com".to_string());
    let id = books.id;
    books.books = Loading::Pending;
    app.screen = Screen::Settings(Box::new(SettingsPage::new(HOST.to_string())), books);

    app.receive(Message::To(
        id,
        Reply::BooksLoaded(Ok(data::Library {
            books: Vec::new(),
            can_write: true,
        })),
    ));
    assert!(matches!(&app.screen, Screen::Settings(_, books) if matches!(books.books, Loading::Ready(_))));
}

#[test]
fn publishing_goes_only_to_the_overlay_it_was_for_even_under_the_quit_question() {
    let mut app = App::new(Images::none());
    let page = book_page();
    let book_page_id = page.id;
    app.screen = Screen::Book(Box::new(page));
    let earlier = next_id();
    app.overlay = Some(Overlay::Publishing(Publishing::new(HOST, book_page_id, &book())));

    app.receive(Message::To(earlier, Reply::Published(next_id(), Ok(publication(true)))));
    match &mut app.overlay {
        Some(Overlay::Publishing(publishing)) => assert!(
            matches!(publishing.key(press(KeyCode::Char('c'))), publishing::Action::Nothing),
            "still private"
        ),
        _ => panic!("the overlay closed"),
    }

    let id = match &mut app.overlay {
        Some(Overlay::Publishing(publishing)) => {
            assert!(matches!(
                publishing.key(press(KeyCode::Down)),
                publishing::Action::Request(_)
            ));
            assert!(
                matches!(publishing.key(press(KeyCode::Down)), publishing::Action::Nothing),
                "saving"
            );
            publishing.id
        }
        _ => panic!("the overlay closed"),
    };

    app.key(ctrl_c());
    app.receive(Message::To(id, Reply::Published(book_page_id, Ok(publication(true)))));
    app.key(press(KeyCode::Esc));

    match &mut app.overlay {
        Some(Overlay::Publishing(publishing)) => assert!(matches!(
            publishing.key(press(KeyCode::Char('c'))),
            publishing::Action::Copy(_)
        )),
        _ => panic!("the overlay is gone"),
    }
    assert!(matches!(&app.screen, Screen::Book(page) if page.book.ready().unwrap().published));
}

#[test]
fn publishing_blocks_editing_the_address_while_saving() {
    let mut publishing = Publishing::new(HOST, next_id(), &book());

    assert!(matches!(
        publishing.key(press(KeyCode::Down)),
        publishing::Action::Request(_)
    ));
    publishing.key(press(KeyCode::Char('e')));
    assert!(!publishing.typing());
}

#[test]
fn search_results_arrive_under_the_quit_question() {
    let mut app = App::new(Images::none());
    let search = Search::new(None, "every book".to_string());
    let id = search.id;
    app.overlay = Some(Overlay::Search(Box::new(search)));
    app.paste("rabbit");
    app.key(press(KeyCode::Enter));

    app.key(ctrl_c());
    app.receive(Message::To(id, Reply::Found("rabbit".to_string(), Ok(Vec::new()))));
    app.key(press(KeyCode::Esc));

    assert!(matches!(&app.overlay, Some(Overlay::Search(search)) if matches!(search.results, Some(Loading::Ready(_)))));
}

#[test]
fn only_the_last_invite_reset_shows_its_link() {
    let mut app = App::new(Images::none());
    let mut page = SettingsPage::new(HOST.to_string());
    page.people = Loading::Ready(data::People {
        join_url: Some(format!("{HOST}/join/old")),
        can_administer: true,
        people: Vec::new(),
    });
    let id = page.id;
    let _first = page.reset_invite();
    let _second = page.reset_invite();
    let profile = ProfilePage::new(HOST.to_string());
    app.screen = Screen::Profile(Box::new(profile), Box::new(Screen::Settings(Box::new(page), books())));

    app.receive(Message::To(
        next_id(),
        Reply::InviteReset(2, Ok(format!("{HOST}/join/elsewhere"))),
    ));
    app.receive(Message::To(
        id,
        Reply::InviteReset(2, Ok(format!("{HOST}/join/second"))),
    ));
    app.receive(Message::To(id, Reply::InviteReset(1, Ok(format!("{HOST}/join/first")))));

    match &app.screen {
        Screen::Profile(_, back) => match back.as_ref() {
            Screen::Settings(page, _) => assert_eq!(
                page.people.ready().unwrap().join_url,
                Some(format!("{HOST}/join/second"))
            ),
            _ => panic!("no settings"),
        },
        _ => panic!("the profile closed"),
    }
}

#[test]
fn a_profile_save_finishing_leaves_a_reopened_profile_alone() {
    let mut app = App::new(Images::none());
    let mut page = ProfilePage::new(HOST.to_string());
    page.loaded(Ok(data::Profile {
        id: 1,
        name: "Stanko".to_string(),
        email: "stanko@example.com".to_string(),
    }));
    app.screen = Screen::Profile(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));
    app.key(press(KeyCode::Down));
    app.key(press(KeyCode::Down));
    app.paste("secret");

    app.receive(Message::To(next_id(), Reply::ProfileSaved(Ok(()))));
    assert!(matches!(&app.screen, Screen::Profile(page, _) if page.password() == "secret"));
    assert!(app.notice.is_none());
}

#[test]
fn tab_moves_through_the_profile_and_only_ctrl_i_opens_the_instances() {
    let mut app = App::new(Images::none());
    let mut page = ProfilePage::new(HOST.to_string());
    page.loaded(Ok(data::Profile {
        id: 1,
        name: "Stanko".to_string(),
        email: "stanko@example.com".to_string(),
    }));
    app.screen = Screen::Profile(Box::new(page), Box::new(Screen::Instances(InstancePicker::new(0))));

    app.key(press(KeyCode::Tab));
    app.key(press(KeyCode::Char('!')));
    assert!(app.overlay.is_none());
    assert!(matches!(&app.screen, Screen::Profile(page, _) if page.email() == "stanko@example.com!"));

    app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL));
    assert!(matches!(app.overlay, Some(Overlay::Instances(_))));
}

#[test]
fn pastes_into_the_field_being_typed_in() {
    let mut app = App::new(Images::none());
    app.overlay = Some(Overlay::Search(Box::new(Search::new(None, "every book".to_string()))));

    app.paste("white\nrabbit?");
    match &mut app.overlay {
        Some(Overlay::Search(search)) => assert!(
            matches!(search.key(press(KeyCode::Enter)), search::Action::Search(query) if query == "white rabbit?")
        ),
        _ => panic!("the search closed"),
    }

    app.screen = editing_page(1);
    app.overlay = None;
    app.paste("- one\n- two");
    assert_eq!(edited_body(&app), "- one\n- two");
}

#[test]
fn draws_every_screen_and_overlay_in_a_terminal_too_small_for_them() {
    let mut settings = SettingsPage::new(HOST.to_string());
    settings.people = Loading::Ready(data::People {
        join_url: Some(format!("{HOST}/join/abc")),
        can_administer: true,
        people: (1..30)
            .map(|id| data::Person {
                id,
                name: format!("Person {id}"),
                role: "member".to_string(),
                current: id == 1,
            })
            .collect(),
    });
    let mut edit = BookFormPage::new(HOST.to_string(), "books.example.com".to_string(), Some(1));
    edit.form = Loading::Ready(form(Some(1), "Handbook"));
    let mut profile = ProfilePage::new(HOST.to_string());
    profile.loaded(Ok(data::Profile {
        id: 1,
        name: "Stanko".to_string(),
        email: "stanko@example.com".to_string(),
    }));

    let screens: Vec<fn() -> Screen> = vec![
        || Screen::Instances(InstancePicker::new(0)),
        || Screen::Books(books()),
        || Screen::Book(Box::new(book_page())),
        || Screen::Leaf(Box::new(reading_page(1)), Box::new(book_page())),
        || editing_page(1),
    ];
    let mut apps: Vec<App> = Vec::new();

    for screen in screens {
        for view in [View::Gallery, View::List] {
            let mut app = App::new(Images::none());
            app.screen = screen();
            app.books_view = view;
            app.book_view = view;
            apps.push(app);
        }
    }

    for screen in [
        Screen::BookForm(Box::new(edit), Box::new(Screen::Books(books()))),
        Screen::Settings(Box::new(settings), books()),
        Screen::Profile(Box::new(profile), Box::new(Screen::Books(books()))),
    ] {
        let mut app = App::new(Images::none());
        app.screen = screen;
        apps.push(app);
    }

    let overlays: Vec<fn(&App) -> Overlay> = vec![
        |_| Overlay::Instances(InstancePicker::new(0)),
        |_| Overlay::Add(Add::default()),
        |_| Overlay::Publishing(Publishing::new(HOST, 1, &book())),
        |_| Overlay::Search(Box::new(Search::new(None, "every book".to_string()))),
        |_| Overlay::Confirm(Confirm::new(Question::Quit(None))),
        |app| match &app.screen {
            Screen::Leaf(page, _) => Overlay::Index(Index::new(page)),
            _ => Overlay::Confirm(Confirm::new(Question::Discard)),
        },
        |_| {
            Overlay::Files(
                Box::new(FilePicker::new("Choose a cover", files::IMAGES, std::env::temp_dir())),
                Purpose::Cover,
            )
        },
    ];

    for app in &mut apps {
        for overlay in std::iter::once(None).chain(overlays.iter().map(Some)) {
            app.overlay = overlay.map(|it| it(app));

            for (width, height) in [(0, 0), (1, 1), (2, 1), (4, 3), (8, 5), (12, 4), (20, 8), (30, 12)] {
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
            }
        }
    }
}

#[test]
fn ctrl_i_and_tab_both_open_the_instances() {
    assert!(is_tab_or_ctrl_i(KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::CONTROL
    )));
    assert!(is_tab_or_ctrl_i(press(KeyCode::Tab)));
    assert!(!is_tab_or_ctrl_i(press(KeyCode::Char('i'))));
}
