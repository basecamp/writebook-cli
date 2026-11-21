//! The read-only commands and token sign-in, end to end against a fake
//! Writebook.

mod support;

use std::time::{Duration, Instant};
use support::{Reply, Server, redirect, reply, stderr, stdout, writebook};

const PAGE_MD: &str = "---\ntitle: \"On &amp; off call\"\nurl: \"http://books.test/4/handbook/12/on-call\"\n---\n\n# On call\n\nPage the person on call.\n";

#[test]
fn reads_a_published_page_from_the_markdown_export() {
    let server = Server::start(|request| match request.path.as_str() {
        "/4/handbook/12/on-call.md" => reply(200, PAGE_MD),
        _ => reply(404, ""),
    });

    let output = writebook(&["read", &format!("{}/4/handbook/12/on-call#steps", server.url)], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "---\ntitle: \"On & off call\"\nurl: \"http://books.test/4/handbook/12/on-call\"\n---\n\n# On call\n\nPage the person on call.\n"
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].header("Authorization"), None);
}

#[test]
fn reads_json() {
    let server = Server::start(|_| reply(200, PAGE_MD));

    let output = writebook(&["read", "--json", &format!("{}/4/handbook/12/on-call", server.url)], &[]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();

    assert_eq!(json["title"], "On & off call");
    assert_eq!(json["url"], "http://books.test/4/handbook/12/on-call");
    assert_eq!(json["markdown"], "# On call\n\nPage the person on call.\n");
    assert_eq!(json["source"], "export");
}

#[test]
fn reads_through_the_api_with_a_token() {
    let server = Server::start(|request| {
        if request.header("Authorization") != Some("Bearer secret") {
            return reply(401, "");
        }

        match request.path.as_str() {
            "/books/4/manifest.json" => reply(
                200,
                r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
                    "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#,
            ),
            "/books/4/pages/12.json" => {
                reply(200, r##"{"id":12,"title":"On call","type":"Page","fingerprint":"g","body":"# Private steps\n"}"##)
            }
            _ => reply(404, ""),
        }
    });

    let output =
        writebook(&["read", "--json", &format!("{}/4/handbook/12/on-call", server.url)], &[("WRITEBOOK_TOKEN", "secret")]);
    assert!(output.status.success(), "{}", stderr(&output));

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["markdown"], "# Private steps\n");
    assert_eq!(json["url"], format!("{}/4/handbook/12/on-call", server.url));
    assert_eq!(json["source"], "api");
}

#[test]
fn the_token_flag_wins_over_the_environment() {
    let server = Server::start(|request| match request.header("Authorization") {
        Some("Bearer from-flag") => reply(404, ""),
        _ => reply(401, ""),
    });

    let output =
        writebook(&["--token", "from-flag", "read", &format!("{}/4/handbook", server.url)], &[("WRITEBOOK_TOKEN", "from-env")]);

    // 404 from the API means "no API here", so it falls back to the export,
    // which 404s too: an error, but one about the page, not the token.
    assert!(!output.status.success());
    assert!(stderr(&output).contains("was not found"), "{}", stderr(&output));
    assert!(server.requests().iter().all(|it| it.header("Authorization") == Some("Bearer from-flag")));
}

#[test]
fn falls_back_to_the_export_when_the_site_has_no_api() {
    let server = Server::start(|request| match request.path.as_str() {
        "/4/handbook/12/on-call.md" => reply(200, PAGE_MD),
        _ => reply(404, "<html>not found</html>"),
    });

    let output = writebook(&["read", &format!("{}/4/handbook/12/on-call", server.url)], &[("WRITEBOOK_TOKEN", "secret")]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Page the person on call."));
}

#[test]
fn a_rejected_token_is_an_error() {
    let server = Server::start(|_| reply(401, r#"{"error":"invalid_token"}"#));

    let output = writebook(&["read", &format!("{}/4/handbook/12/on-call", server.url)], &[("WRITEBOOK_TOKEN", "nope")]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("did not accept the token"), "{}", stderr(&output));
}

#[test]
fn explains_private_pages() {
    let server = Server::start(|_| redirect("/session/new"));

    let output = writebook(&["read", &format!("{}/4/handbook/12/on-call", server.url)], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("WRITEBOOK_TOKEN"), "{}", stderr(&output));
}

#[test]
fn lists_books() {
    let server = Server::start(|request| match request.path.as_str() {
        "/" => reply(
            200,
            r#"<div class="library">
                 <figure class="library__book theme--blue">
                   <turbo-frame id="bookmark_book_4" src="/books/4/bookmark"></turbo-frame>
                   <h2><strong>Ops Handbook</strong><span class="overflow-line-clamp">37signals</span></h2>
                 </figure>
               </div>"#,
        ),
        _ => reply(404, ""),
    });

    let output = writebook(&["books", &server.url], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), format!("- [Ops Handbook]({}/4/ops-handbook) by 37signals\n", server.url));
}

#[test]
fn searches_a_book_like_a_browser_when_there_is_no_token() {
    let server = Server::start(|request| match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/4/book") => Reply {
            status: 200,
            headers: vec![("Set-Cookie", "_writebook_session=abc; path=/; HttpOnly".to_string())],
            body: r#"<meta name="csrf-token" content="form+token=" />"#.to_string(),
        },
        ("POST", "/books/4/search") => {
            if request.header("Cookie") != Some("_writebook_session=abc")
                || !request.body.contains("authenticity_token=form%2Btoken%3D")
            {
                return reply(422, "");
            }

            assert!(request.body.contains("search=roll+back"), "{}", request.body);

            reply(
                200,
                r#"<a class="search__result hide_from_edit_mode txt-ink" href="/4/handbook/12/deploys?search=roll+back">
                     <strong>Deploys:</strong> how to <mark>roll</mark> <mark>back</mark></a>"#,
            )
        }
        _ => reply(404, ""),
    });

    let output = writebook(&["search", "--json", &format!("{}/4/handbook", server.url), "roll back"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json[0]["title"], "Deploys");
    assert_eq!(json[0]["url"], format!("{}/4/handbook/12/deploys", server.url));
    assert_eq!(json[0]["snippet"], "how to roll back");
}

#[test]
fn unattended_sign_in_fails_fast_instead_of_waiting_for_a_browser() {
    let server = Server::start(|_| {
        reply(200, r#"{"device_code":"d","user_code":"U","verification_uri_complete":"x","expires_in":600,"interval":5}"#)
    });
    let target = support::tempdir().join("checkout");

    let started = Instant::now();
    let output = writebook(&["clone", &format!("{}/4/handbook", server.url), target.to_str().unwrap()], &[]);

    assert!(!output.status.success());
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(stderr(&output).contains("--token"), "{}", stderr(&output));
    assert!(server.requests().is_empty(), "it must not even start a sign-in");
}

#[test]
fn clone_uses_the_given_token() {
    let server = Server::start(|request| {
        if request.header("Authorization") != Some("Bearer secret") {
            return reply(401, "");
        }

        match request.path.as_str() {
            "/books/4/manifest.json" => reply(
                200,
                r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},"leaves":[]}"#,
            ),
            _ => reply(404, ""),
        }
    });
    let target = support::tempdir().join("checkout");

    let output =
        writebook(&["clone", &format!("{}/4/handbook", server.url), target.to_str().unwrap()], &[("WRITEBOOK_TOKEN", "secret")]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(target.join("index.md").exists());
}
