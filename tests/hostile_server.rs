//! A Writebook that has been taken over, or isn't what it claims to be, can't
//! use the CLI to write outside a checkout, get the token sent somewhere
//! else, open links elsewhere, or take over the terminal.

#![cfg(test)]

#[cfg(test)]
mod support;

use support::{Server, cut_short, redirect, reply, stderr, stdout, tempdir, writebook, writebook_in};

#[test]
fn clone_stays_inside_the_checkout() {
    let server = Server::start(|request| match request.path.as_str() {
        "/books/4/manifest.json" => reply(
            200,
            r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"../escaped","fingerprint":"f","cover":null},
                "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#,
        ),
        "/books/4/pages/12.json" => reply(
            200,
            r#"{"id":12,"title":"On call","fingerprint":"g","body":"![x](/u/../../planted.txt)"}"#,
        ),
        _ => reply(200, "planted"),
    });
    let outside = tempdir();
    let work = outside.join("work");
    std::fs::create_dir(&work).unwrap();

    let output = writebook_in(
        &work,
        &["clone", &format!("{}/4/handbook", server.url)],
        &token_for(&server),
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("would lead outside the checkout"),
        "{}",
        stderr(&output)
    );
    assert!(work.join("escaped").is_dir(), "the slug names a directory right here");
    assert!(!outside.join("escaped").exists());
    assert!(!outside.join("planted.txt").exists());
    assert!(!work.join("planted.txt").exists());
}

#[cfg(unix)]
#[test]
fn clone_never_follows_a_symlink_named_like_the_book() {
    let server = Server::start(|request| match request.path.as_str() {
        "/books/4/manifest.json" => reply(200, &book_with_a_picture()),
        _ => reply(404, ""),
    });
    let outside = tempdir();
    let work = tempdir();
    std::os::unix::fs::symlink(&outside, work.join("handbook")).unwrap();

    let output = writebook_in(
        &work,
        &["clone", &format!("{}/4/handbook", server.url)],
        &token_for(&server),
    );

    assert!(!output.status.success());
    assert!(stderr(&output).contains("is a symlink"), "{}", stderr(&output));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn never_sends_the_token_to_an_image_url_on_another_server() {
    let elsewhere = Server::start(|_| reply(200, "PNG"));
    let image_url = format!("{}/a.png", elsewhere.url);
    let server = Server::start(move |request| match request.path.as_str() {
        "/books/4/manifest.json" => reply(200, &book_with_a_picture()),
        "/books/4/pictures/13.json" => reply(200, &picture_at(&image_url)),
        _ => reply(404, ""),
    });

    let output = writebook(
        &[
            "clone",
            &format!("{}/4/handbook", server.url),
            tempdir().join("checkout").to_str().unwrap(),
        ],
        &token_for(&server),
    );

    assert!(!output.status.success());
    assert!(stderr(&output).contains("only paths on"), "{}", stderr(&output));
    assert!(elsewhere.requests().is_empty());
}

#[test]
fn keeps_the_token_only_while_redirects_stay_on_the_site() {
    let elsewhere = Server::start(|request| match request.path.as_str() {
        "/a.png" => redirect("final.png"),
        _ => reply(200, "PNG"),
    });
    let elsewhere_url = elsewhere.url.clone();
    let server = Server::start(move |request| match request.path.as_str() {
        "/books/4/manifest.json" => reply(200, &book_with_a_picture()),
        "/books/4/pictures/13.json" => reply(200, &picture_at("/rails/active_storage/blobs/redirect/a.png")),
        "/rails/active_storage/blobs/redirect/a.png" => redirect(&format!(
            "http://{}/rails/active_storage/disk/a.png",
            request.header("Host").unwrap()
        )),
        "/rails/active_storage/disk/a.png" if request.header("Authorization") == Some("Bearer secret") => {
            redirect(&format!("{elsewhere_url}/a.png"))
        }
        _ => reply(404, ""),
    });
    let scratch = tempdir();
    let checkout = scratch.join("checkout");

    let output = writebook(
        &[
            "clone",
            &format!("{}/4/handbook", server.url),
            checkout.to_str().unwrap(),
        ],
        &token_for(&server),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(std::fs::read_to_string(checkout.join("assets/a.png")).unwrap(), "PNG");

    let requests = elsewhere.requests();
    assert_eq!(
        requests.iter().map(|it| it.path.as_str()).collect::<Vec<_>>(),
        ["/a.png", "/final.png"]
    );
    assert!(requests.iter().all(|it| it.header("Authorization").is_none()));
    assert!(server.requests().iter().all(|it| it.path != "/final.png"));
}

#[test]
fn never_opens_or_prints_a_sign_in_link_to_somewhere_else() {
    let server = Server::start(|request| match request.path.as_str() {
        "/oauth/device_authorizations" => reply(
            200,
            r#"{"device_code":"wb_dc_secret","user_code":"ABCD-EFGH","verification_uri_complete":"https://phish.example.com/approve","expires_in":600,"interval":5}"#,
        ),
        _ => reply(404, ""),
    });

    let output = writebook(&["instance", "add", &server.url], &[]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("sent a sign-in link to somewhere else"),
        "{}",
        stderr(&output)
    );
    assert!(!stdout(&output).contains("phish.example.com"), "{}", stdout(&output));
}

#[test]
fn never_opens_or_prints_a_sign_in_link_with_terminal_escapes() {
    let server = Server::start(|request| match request.path.as_str() {
        "/oauth/device_authorizations" => reply(
            200,
            &format!(
                r#"{{"device_code":"wb_dc_secret","user_code":"ABCD-EFGH","verification_uri_complete":"http://{}/approve\u001b]52;c;cm0gLXJmIH4K\u0007","expires_in":600,"interval":5}}"#,
                request.header("Host").unwrap()
            ),
        ),
        _ => reply(404, ""),
    });

    let output = writebook(&["instance", "add", &server.url], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("control characters"), "{}", stderr(&output));
    assert!(!stdout(&output).contains('\x1b'), "{:?}", stdout(&output));
    assert!(!stderr(&output).contains('\x1b'), "{:?}", stderr(&output));
}

#[test]
fn prints_no_terminal_escape_sequences_from_the_server() {
    let server = Server::start(|_| {
        reply(
            200,
            "---\ntitle: \"On call\"\n---\n\n# On call\x1b]52;c;cm0gLXJmIH4K\x07\n\nPage \x1b[2Jthe person on call.\n",
        )
    });

    let output = writebook(&["read", &format!("{}/4/handbook/12/on-call", server.url)], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains('\x1b'), "{:?}", stdout(&output));
    assert!(!stdout(&output).contains('\x07'), "{:?}", stdout(&output));
    assert!(
        stdout(&output).contains("Page [2Jthe person on call."),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_successful_answer_that_isnt_json_is_an_error() {
    let server = Server::start(|request| match request.path.as_str() {
        "/books/4/manifest.json" => reply(200, &book_with_a_page()),
        "/books/4/pages/12.json" => reply(200, r##"{"id":12,"title":"On call","fingerprint":"g","body":"# St"##),
        _ => reply(404, ""),
    });
    let work = tempdir();

    let output = writebook_in(
        &work,
        &["clone", &format!("{}/4/handbook", server.url)],
        &token_for(&server),
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("the answer to /books/4/pages/12.json isn't valid JSON"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("secret"));
    assert!(!work.join("handbook/index.md").exists());
}

#[test]
fn a_successful_answer_cut_short_is_an_error() {
    let server = Server::start(|request| match request.path.as_str() {
        "/books/4/manifest.json" => reply(200, &book_with_a_page()),
        "/books/4/pages/12.json" => cut_short(
            200,
            r##"{"id":12,"title":"On call","fingerprint":"g","body":"# Steps"}"##,
        ),
        _ => reply(404, ""),
    });
    let work = tempdir();

    let output = writebook_in(
        &work,
        &["clone", &format!("{}/4/handbook", server.url)],
        &token_for(&server),
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("could not read the answer to /books/4/pages/12.json"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("secret"));
    assert!(!work.join("handbook/index.md").exists());
}

fn token_for(server: &Server) -> [(&str, &str); 2] {
    [("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)]
}

fn book_with_a_page() -> String {
    r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
        "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#
        .to_string()
}

fn book_with_a_picture() -> String {
    r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
        "leaves":[{"id":13,"title":"Diagram","type":"Picture","fingerprint":"g"}]}"#
        .to_string()
}

fn picture_at(url: &str) -> String {
    format!(
        r#"{{"id":13,"title":"Diagram","fingerprint":"g","caption":"Diagram","image":{{"filename":"a.png","url":"{url}","checksum":"c"}}}}"#
    )
}
