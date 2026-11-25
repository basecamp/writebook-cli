//! The inventory of instances and their profiles: adding an instance signs in
//! to it, instances are picked by alias, URL or default, and each profile is
//! its own sign-in.

mod support;

use std::process::Output;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::{Server, reply, stderr, stdout, writebook};

/// Each sign-in gets its own access token: wb_at_1, wb_at_2, and so on.
fn fake_writebook() -> Server {
    let sign_ins = Arc::new(AtomicUsize::new(0));

    Server::start(move |request| match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/oauth/device_authorizations") => support::device_authorization(request),
        ("POST", "/oauth/tokens") => {
            let number = sign_ins.fetch_add(1, Ordering::SeqCst) + 1;
            reply(200, &format!(r#"{{"access_token":"wb_at_{number}","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_{number}"}}"#))
        }
        ("GET", "/books.json") => reply(
            200,
            r#"[{"id":4,"title":"Ops Handbook","subtitle":null,"author":null,"slug":"ops-handbook","published":true,
                 "url":"http://books.test/4/ops-handbook"}]"#,
        ),
        _ => reply(404, ""),
    })
}

/// Both steps of a sign-in without a terminal: get the link, then finish with the code.
fn signed_in(args: &[&str], env: &[(&str, &str)]) -> Output {
    let started = writebook(args, env);
    assert!(started.status.success(), "{}", stderr(&started));

    let finished = writebook(&[args, &["--code", "WXYZ-2345"]].concat(), env);
    assert!(finished.status.success(), "{}", stderr(&finished));
    finished
}

#[test]
fn adds_instances_and_picks_them_by_alias_url_or_default() {
    let server = fake_writebook();
    let other = fake_writebook();
    let config = support::tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    let host = server.url.trim_start_matches("http://");

    let added = signed_in(&["instance", "add", &server.url], &env);
    assert!(stdout(&added).contains(&format!("Added {host} ({}), signed in as default", server.url)), "{}", stdout(&added));

    let renamed = writebook(&["instance", "add", &format!("{}/", server.url), "--alias", "work"], &env);
    assert!(!renamed.status.success());
    assert!(stderr(&renamed).contains(&format!("already in the inventory as '{host}'")), "{}", stderr(&renamed));

    signed_in(&["instance", "add", &other.url, "--alias", "other"], &env);

    let listed = writebook(&["instance", "list"], &env);
    let alias_width = format!("{host} (default)").len();
    assert_eq!(
        stdout(&listed),
        format!(
            "{:alias_width$}  {:url_width$}  Status\n{:alias_width$}  {}  Signed in as default\n{:alias_width$}  {}  Signed in as default\n",
            "Alias",
            "URL",
            format!("{host} (default)"),
            server.url,
            "other",
            other.url,
            url_width = server.url.len()
        )
    );

    let by_default = writebook(&["book", "list", "--markdown"], &env);
    assert!(by_default.status.success(), "{}", stderr(&by_default));
    assert_eq!(stdout(&by_default), "- [Ops Handbook](http://books.test/4/ops-handbook)\n");
    assert_eq!(server.requests().last().unwrap().header("Authorization"), Some("Bearer wb_at_1"));

    writebook(&["book", "list", "--instance", "other"], &env);
    assert_eq!(other.requests().last().unwrap().path, "/books.json");

    let set_default = writebook(&["instance", "set-default", "other"], &env);
    assert!(set_default.status.success(), "{}", stderr(&set_default));
    assert!(stdout(&writebook(&["instance", "list"], &env)).contains("other (default)"));

    let removed = writebook(&["instance", "remove", &server.url], &env);
    assert!(removed.status.success(), "{}", stderr(&removed));
    let json = serde_json::from_str::<serde_json::Value>(&stdout(&writebook(&["instance", "list", "--json"], &env))).unwrap()["data"].clone();
    assert_eq!(json, serde_json::json!([{ "alias": "other", "url": other.url, "default": true, "profile": "default", "signed_in": true, "read_only": false }]));

    let credentials = std::fs::read_to_string(config.join("writebook/credentials.json")).unwrap();
    assert!(!credentials.contains(&server.url), "removing an instance signs out of it: {credentials}");

    let unknown = writebook(&["book", "list", "--instance", "nope"], &env);
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("there is no instance 'nope'"), "{}", stderr(&unknown));
}

#[test]
fn without_any_instance_explains_how_to_add_one() {
    let output = writebook(&["book", "list"], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("writebook instance add <url>"), "{}", stderr(&output));
}

#[test]
fn signs_in_as_each_profile() {
    let server = fake_writebook();
    let config = support::tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];

    signed_in(&["instance", "add", &server.url, "--alias", "work", "--profile", "me"], &env);
    let editor = signed_in(&["profile", "add", "editor"], &env);
    assert!(stdout(&editor).contains(&format!("Signed in to work ({}) as editor", server.url)), "{}", stdout(&editor));

    assert_eq!(stdout(&writebook(&["profile", "list"], &env)), "* me\n  editor\n");

    writebook(&["book", "list"], &env);
    assert_eq!(server.requests().last().unwrap().header("Authorization"), Some("Bearer wb_at_1"));

    writebook(&["book", "list", "--profile", "editor"], &env);
    assert_eq!(server.requests().last().unwrap().header("Authorization"), Some("Bearer wb_at_2"));

    writebook(&["profile", "set-default", "editor", "--instance", "work"], &env);
    writebook(&["book", "list"], &env);
    assert_eq!(server.requests().last().unwrap().header("Authorization"), Some("Bearer wb_at_2"));

    let unknown = writebook(&["book", "list", "--profile", "boss"], &env);
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("writebook profile add boss --instance work"), "{}", stderr(&unknown));

    let remove_default = writebook(&["profile", "remove", "editor"], &env);
    assert!(!remove_default.status.success());
    assert!(stderr(&remove_default).contains("is the default profile"), "{}", stderr(&remove_default));

    writebook(&["profile", "set-default", "me"], &env);
    let removed = writebook(&["profile", "remove", "editor"], &env);
    assert!(removed.status.success(), "{}", stderr(&removed));

    let credentials = std::fs::read_to_string(config.join("writebook/credentials.json")).unwrap();
    assert!(credentials.contains("wb_at_1") && !credentials.contains("wb_at_2"), "{credentials}");
}
