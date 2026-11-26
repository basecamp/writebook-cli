//! The read-only commands and token sign-in, end to end against a fake
//! Writebook.

#![cfg(test)]

#[cfg(test)]
mod support;

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

    let output = writebook(
        &["read", "--json", &format!("{}/4/handbook/12/on-call", server.url)],
        &[],
    );
    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();

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
            "/books/4/pages/12.json" => reply(
                200,
                r##"{"id":12,"title":"On call","type":"Page","fingerprint":"g","body":"# Private steps\n"}"##,
            ),
            _ => reply(404, ""),
        }
    });

    let output = writebook(
        &["read", "--json", &format!("{}/4/handbook/12/on-call", server.url)],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );
    assert!(output.status.success(), "{}", stderr(&output));

    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();
    assert_eq!(json["markdown"], "# Private steps\n");
    assert_eq!(json["url"], format!("{}/4/handbook/12/on-call", server.url));
    assert_eq!(json["source"], "api");
}

#[test]
fn reads_a_whole_book_through_the_api_as_markdown_with_front_matter() {
    let server = Server::start(|request| {
        if request.header("Authorization") != Some("Bearer secret") {
            return reply(401, "");
        }

        match request.path.as_str() {
            "/books/4/manifest.json" => reply(
                200,
                r#"{"book":{"id":4,"title":"On \"call\"","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
                    "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#,
            ),
            "/books/4/pages/12.json" => reply(
                200,
                r##"{"id":12,"title":"On call","type":"Page","fingerprint":"g","body":"# Steps"}"##,
            ),
            _ => reply(404, ""),
        }
    });

    let output = writebook(
        &["read", &format!("{}/4/handbook", server.url)],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!(
            "---\ntitle: \"On \\\"call\\\"\"\nauthor: \"Ops\"\nurl: \"{}/4/handbook\"\n---\n\n# Steps\n",
            server.url
        )
    );
}

#[test]
fn falls_back_to_the_export_when_the_site_has_no_api() {
    let server = Server::start(|request| match request.path.as_str() {
        "/4/handbook/12/on-call.md" => reply(200, PAGE_MD),
        _ => reply(404, "<html>not found</html>"),
    });

    let output = writebook(
        &["read", &format!("{}/4/handbook/12/on-call", server.url)],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Page the person on call."));
}

#[test]
fn a_rejected_token_is_an_error() {
    let server = Server::start(|_| reply(401, r#"{"error":"invalid_token"}"#));

    let output = writebook(
        &["read", &format!("{}/4/handbook/12/on-call", server.url)],
        &[("WRITEBOOK_TOKEN", "nope"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("did not accept the token"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn explains_private_pages() {
    let server = Server::start(|_| reply(404, ""));

    let output = writebook(&["read", &format!("{}/4/handbook/12/on-call", server.url)], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("WRITEBOOK_TOKEN"), "{}", stderr(&output));
}

#[test]
fn never_sends_the_token_to_another_site() {
    let server = Server::start(|_| reply(200, PAGE_MD));

    let output = writebook(
        &["read", &format!("{}/4/handbook/12/on-call", server.url)],
        &[
            ("WRITEBOOK_TOKEN", "secret"),
            ("WRITEBOOK_TOKEN_HOST", "https://books.example.com"),
        ],
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("the token is for https://books.example.com"),
        "{}",
        stderr(&output)
    );
    assert!(server.requests().is_empty());
}

#[test]
fn a_token_needs_to_say_which_site_it_is_for() {
    let server = Server::start(|_| reply(200, PAGE_MD));

    let output = writebook(
        &["read", &format!("{}/4/handbook/12/on-call", server.url)],
        &[("WRITEBOOK_TOKEN", "secret")],
    );

    assert!(!output.status.success());
    assert!(stderr(&output).contains("WRITEBOOK_TOKEN_HOST"), "{}", stderr(&output));
    assert!(server.requests().is_empty());
}

#[test]
fn never_sends_the_token_over_plain_http_to_another_machine() {
    let output = writebook(
        &["read", "http://books.example.com/4/handbook/12/on-call"],
        &[
            ("WRITEBOOK_TOKEN", "secret"),
            ("WRITEBOOK_TOKEN_HOST", "http://books.example.com"),
        ],
    );

    assert!(!output.status.success());
    assert!(stderr(&output).contains("only sent over https"), "{}", stderr(&output));
}

#[test]
fn agents_never_get_a_browser_sign_in_even_with_a_terminal() {
    let server = Server::start(|_| reply(500, ""));
    let scratch = support::tempdir();
    let target = scratch.join("checkout");

    let output = writebook(
        &["clone", &format!("{}/4/handbook", server.url), target.to_str().unwrap()],
        &[("CLAUDECODE", "1")],
    );

    assert!(!output.status.success());
    assert!(server.requests().is_empty());
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

    let table = writebook(&["book", "list", "--instance", &server.url], &[]);
    assert!(table.status.success(), "{}", stderr(&table));
    assert_eq!(
        stdout(&table),
        format!(
            "ID  Title         Author     URL\n4   Ops Handbook  37signals  {}/4/ops-handbook\n",
            server.url
        )
    );

    let markdown = writebook(&["book", "list", "--instance", &server.url, "--markdown"], &[]);
    assert_eq!(
        stdout(&markdown),
        format!("- [Ops Handbook]({}/4/ops-handbook) by 37signals\n", server.url)
    );

    let both = writebook(
        &["book", "list", "--instance", &server.url, "--markdown", "--json"],
        &[],
    );
    assert!(!both.status.success(), "--json and --markdown are one or the other");
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
                reply(422, "")
            } else {
                assert!(request.body.contains("search=roll+back"), "{}", request.body);

                reply(
                    200,
                    r#"<a class="search__result hide_from_edit_mode txt-ink" href="/4/handbook/12/deploys?search=roll+back">
                     <strong>Deploys:</strong> how to <mark>roll</mark> <mark>back</mark></a>"#,
                )
            }
        }
        _ => reply(404, ""),
    });

    let output = writebook(
        &[
            "search",
            "roll back",
            "--json",
            "--book",
            &format!("{}/4/handbook", server.url),
        ],
        &[],
    );
    assert!(output.status.success(), "{}", stderr(&output));

    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();
    assert_eq!(json[0]["title"], "Deploys");
    assert_eq!(json[0]["url"], format!("{}/4/handbook/12/deploys", server.url));
    assert_eq!(json[0]["snippet"], "how to roll back");
}

#[test]
fn unattended_sign_in_fails_fast_instead_of_waiting_for_a_browser() {
    let server = Server::start(|_| {
        reply(
            200,
            r#"{"device_code":"d","user_code":"U","verification_uri_complete":"x","expires_in":600,"interval":5}"#,
        )
    });
    let scratch = support::tempdir();
    let target = scratch.join("checkout");

    let output = writebook(
        &["clone", &format!("{}/4/handbook", server.url), target.to_str().unwrap()],
        &[],
    );

    assert!(!output.status.success());
    assert!(stderr(&output).contains("WRITEBOOK_TOKEN"), "{}", stderr(&output));
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
    let scratch = support::tempdir();
    let target = scratch.join("checkout");

    let output = writebook(
        &["clone", &format!("{}/4/handbook", server.url), target.to_str().unwrap()],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(target.join("index.md").exists());
}

#[test]
fn explains_a_site_with_no_published_books() {
    let server = Server::start(|_| redirect("/session/new"));

    let output = writebook(&["book", "list", "--instance", &server.url], &[]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("has no published books"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn lists_books_from_json_with_a_token() {
    let server = Server::start(|request| {
        if request.header("Authorization") != Some("Bearer secret") {
            return reply(401, "");
        }

        match request.path.as_str() {
            "/books.json" => reply(
                200,
                r#"[{"id":4,"title":"Ops Handbook","subtitle":null,"author":null,"slug":"ops-handbook","published":false,
                     "url":"http://books.test/4/ops-handbook"}]"#,
            ),
            _ => reply(404, ""),
        }
    });

    let output = writebook(
        &["book", "list", "--markdown", "--instance", &server.url],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "- [Ops Handbook](http://books.test/4/ops-handbook)\n");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn explains_a_site_with_no_published_books_from_json() {
    let server = Server::start(|request| match request.path.as_str() {
        "/books.json" => reply(200, "[]"),
        _ => reply(404, ""),
    });

    let output = writebook(&["book", "list", "--instance", &server.url], &[]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("has no published books"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn says_private_books_need_a_token_when_an_anonymous_search_finds_nothing() {
    let server = Server::start(|request| match request.path.split('?').next().unwrap() {
        "/search.json" => reply(200, r#"{"query":"failover","results":[]}"#),
        _ => reply(404, ""),
    });

    let output = writebook(&["search", "failover", "--instance", &server.url], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "No matches.\n");
    assert!(
        stderr(&output).contains("Private ones need a token"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn searches_every_book_in_one_json_request() {
    let server = Server::start(
        |request| match (request.method.as_str(), request.path.split('?').next().unwrap()) {
            ("GET", "/search.json") => {
                let query = request.path.split_once('?').map(|(_, it)| it).unwrap_or_default();
                if query != "q=roll+back" && query != "q=roll%20back" {
                    reply(400, "")
                } else {
                    reply(
                        200,
                        r#"{"query":"roll back","results":[{"id":12,"title":"Deploys","type":"Page","book":{"id":4,"title":"Handbook"},
                     "snippet":"how to roll\n back","url":"http://books.test/4/handbook/12/deploys"}]}"#,
                    )
                }
            }
            _ => reply(404, ""),
        },
    );

    let table = writebook(&["search", "roll back", "--instance", &server.url], &[]);
    assert!(table.status.success(), "{}", stderr(&table));
    assert_eq!(
        stdout(&table),
        "Book      Title    URL\nHandbook  Deploys  http://books.test/4/handbook/12/deploys\n"
    );
    assert_eq!(server.requests().len(), 1);

    let markdown = writebook(&["search", "roll back", "--markdown", "--instance", &server.url], &[]);
    assert_eq!(
        stdout(&markdown),
        "- [Deploys](http://books.test/4/handbook/12/deploys) (Handbook): how to roll back\n"
    );
}

#[test]
fn searches_a_book_through_json_with_a_read_only_token() {
    let server = Server::start(|request| {
        if request.header("Authorization") != Some("Bearer secret") {
            return reply(401, "");
        }

        match (request.method.as_str(), request.path.split('?').next().unwrap()) {
            ("POST", _) => reply(403, ""),
            ("GET", "/books/4/search.json") => reply(
                200,
                r#"{"query":"roll","results":[{"id":12,"title":"Deploys","type":"Page","book":{"id":4,"title":"Handbook"},
                     "snippet":"how to roll back","url":"http://books.test/4/handbook/12/deploys"}]}"#,
            ),
            _ => reply(404, ""),
        }
    });

    let output = writebook(
        &[
            "search",
            "roll",
            "--json",
            "--book",
            &format!("{}/4/handbook", server.url),
        ],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );
    assert!(output.status.success(), "{}", stderr(&output));

    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();
    assert_eq!(json[0]["title"], "Deploys");
    assert_eq!(json[0]["book"], "");
    assert_eq!(json[0]["url"], "http://books.test/4/handbook/12/deploys");
    assert_eq!(json[0]["snippet"], "how to roll back");
    assert!(server.requests().iter().all(|request| request.method == "GET"));
}

fn book_with_three_kinds_of_leaves() -> Server {
    Server::start(|request| {
        if request.header("Authorization") != Some("Bearer secret") {
            return reply(302, "");
        }

        match request.path.as_str() {
            "/books/4/manifest.json" => reply(
                200,
                r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
                    "leaves":[{"id":11,"title":"On call","type":"Section","fingerprint":"a","theme":"blue"},
                              {"id":12,"title":"Paging","type":"Page","fingerprint":"b"},
                              {"id":13,"title":"Rotation chart","type":"Picture","fingerprint":"c","image":null}]}"#,
            ),
            _ => reply(404, ""),
        }
    })
}

#[test]
fn lists_a_books_contents_by_url() {
    let server = book_with_three_kinds_of_leaves();

    let env = [
        ("WRITEBOOK_TOKEN", "secret"),
        ("WRITEBOOK_TOKEN_HOST", server.url.as_str()),
    ];
    let book = format!("{}/4/handbook", server.url);

    let table = writebook(&["book", "contents", &book], &env);
    assert!(table.status.success(), "{}", stderr(&table));
    assert_eq!(
        stdout(&table),
        format!(
            "ID  Type     Title           URL\n\
             11  Section  On call         {0}/4/handbook/11/on-call\n\
             12  Page     Paging          {0}/4/handbook/12/paging\n\
             13  Picture  Rotation chart  {0}/4/handbook/13/rotation-chart\n",
            server.url
        )
    );

    let markdown = writebook(&["book", "contents", &book, "--markdown"], &env);
    assert_eq!(
        stdout(&markdown),
        format!(
            "# [Handbook]({0}/4/handbook)\n\n\
             1. [On call]({0}/4/handbook/11/on-call) (section)\n\
             2. [Paging]({0}/4/handbook/12/paging)\n\
             3. [Rotation chart]({0}/4/handbook/13/rotation-chart) (picture)\n",
            server.url
        )
    );
}

#[test]
fn lists_a_books_contents_by_id_on_an_instance() {
    let server = book_with_three_kinds_of_leaves();

    let output = writebook(
        &["book", "contents", "4", "--instance", &server.url, "--json"],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let envelope = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["title"], "Handbook");
    assert_eq!(
        envelope["data"]["leaves"][0],
        serde_json::json!({ "id": 11, "type": "section", "title": "On call", "url": format!("{}/4/handbook/11/on-call", server.url) })
    );
    assert_eq!(
        envelope["breadcrumbs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|it| it["command"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "writebook read <url>".to_string(),
            format!("writebook search \"<query>\" --book {}/4/handbook", server.url),
            format!("writebook clone {}/4/handbook", server.url),
        ]
    );

    let filtered = writebook(
        &[
            "book",
            "contents",
            "4",
            "--instance",
            &server.url,
            "--jq",
            ".data.leaves[] | select(.type != \"page\") | .title",
        ],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );
    assert!(filtered.status.success(), "{}", stderr(&filtered));
    assert_eq!(stdout(&filtered), "On call\nRotation chart\n");

    let objects = writebook(
        &[
            "book",
            "contents",
            "4",
            "--instance",
            &server.url,
            "--jq",
            ".data.leaves[0] | {id, type}",
        ],
        &[("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );
    assert_eq!(stdout(&objects), "{\n  \"id\": 11,\n  \"type\": \"section\"\n}\n");
}

#[test]
fn refuses_an_invalid_jq_expression_before_asking_the_server() {
    let server = book_with_three_kinds_of_leaves();

    let output = writebook(
        &["book", "contents", "4", "--instance", &server.url, "--jq", ".data["],
        &[],
    );

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("is not a valid jq expression"),
        "{}",
        stderr(&output)
    );
    assert!(server.requests().is_empty());
}

#[test]
fn listing_a_books_contents_needs_a_sign_in() {
    let server = book_with_three_kinds_of_leaves();

    let output = writebook(&["book", "contents", &format!("{}/4/handbook", server.url)], &[]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains(&format!("Sign in with `writebook instance add {}`", server.url)),
        "{}",
        stderr(&output)
    );
    assert!(server.requests().is_empty());
}
