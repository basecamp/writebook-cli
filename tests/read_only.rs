//! Read-only sign-ins: asked for with --read-only, kept that way, and
//! stopped before they try to change a book.

mod support;

use support::{Server, reply, stderr, stdout, tempdir, writebook, writebook_in};

#[test]
fn signs_in_read_only_and_stops_before_changing_a_book() {
    let server = fake_writebook(r#","scope":"read""#);
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];

    let started = writebook(&["instance", "add", &server.url, "--read-only"], &env);
    assert!(started.status.success(), "{}", stderr(&started));
    let finished = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], &env);
    assert!(finished.status.success(), "{}", stderr(&finished));

    let authorization = server.requests().into_iter().find(|it| it.path == "/oauth/device_authorizations").unwrap();
    assert!(authorization.body.contains("scope=read&") || authorization.body.ends_with("scope=read"), "{}", authorization.body);

    let listed = writebook(&["instance", "list"], &env);
    assert!(stdout(&listed).contains("Signed in as default, read-only"), "{}", stdout(&listed));

    let checkout = checkout_of(&server);
    let pushed = writebook_in(&checkout, &["push"], &env);
    assert!(!pushed.status.success());
    assert!(stderr(&pushed).contains("read-only, which can't change books"), "{}", stderr(&pushed));
    assert!(server.requests().iter().all(|it| !it.path.starts_with("/books")));
}

#[test]
fn signs_in_with_full_access_unless_asked() {
    let server = fake_writebook(r#","scope":"read write""#);
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];

    writebook(&["instance", "add", &server.url], &env);
    let finished = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], &env);
    assert!(finished.status.success(), "{}", stderr(&finished));

    let authorization = server.requests().into_iter().find(|it| it.path == "/oauth/device_authorizations").unwrap();
    assert!(authorization.body.contains("scope=read+write") || authorization.body.contains("scope=read%20write"), "{}", authorization.body);

    let listed = writebook(&["instance", "list"], &env);
    assert!(stdout(&listed).contains("Signed in as default"), "{}", stdout(&listed));
    assert!(!stdout(&listed).contains("read-only"), "{}", stdout(&listed));
}

#[test]
fn turns_down_full_access_when_read_only_was_asked() {
    let server = fake_writebook("");
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];

    writebook(&["instance", "add", &server.url, "--read-only"], &env);
    let finished = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], &env);

    assert!(!finished.status.success());
    assert!(stderr(&finished).contains("can't make read-only sign-ins yet"), "{}", stderr(&finished));

    let revocation = server.requests().into_iter().find(|it| it.path == "/oauth/revocations").expect("a revocation");
    assert!(revocation.body.contains("token=wb_rt_1"), "{}", revocation.body);
    assert!(!std::fs::read_to_string(config.join("writebook/credentials.json")).unwrap_or_default().contains("wb_rt_1"));
}

#[test]
fn a_read_only_api_token_stops_before_changing_a_book() {
    let server = fake_writebook("");
    let checkout = checkout_of(&server);

    let pushed = writebook_in(&checkout, &["push"], &[("WRITEBOOK_TOKEN", "wb_api_secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)]);

    assert!(!pushed.status.success());
    assert!(stderr(&pushed).contains("read-only API token"), "{}", stderr(&pushed));
    assert!(server.requests().is_empty());
}

#[test]
fn a_renewal_keeps_a_read_only_sign_in_read_only() {
    let server = fake_writebook_renewing(r#","scope":"read""#, "");
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    sign_in_read_only(&server, &env);
    expire_sign_ins(&config);

    let listed = writebook(&["book", "list", "--instance", &server.url], &env);
    assert!(listed.status.success(), "{}", stderr(&listed));

    let instances = writebook(&["instance", "list"], &env);
    assert!(stdout(&instances).contains("Signed in as default, read-only"), "{}", stdout(&instances));
}

#[test]
fn a_renewal_with_full_access_is_revoked_and_signing_in_again_stays_read_only() {
    let server = fake_writebook_renewing(r#","scope":"read""#, r#","scope":"read write""#);
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    sign_in_read_only(&server, &env);
    expire_sign_ins(&config);

    let cloned = writebook(&["clone", &format!("{}/4/handbook", server.url), tempdir().join("checkout").to_str().unwrap()], &env);

    let alias = server.url.trim_start_matches("http://");
    assert!(stderr(&cloned).contains(&format!("`writebook instance add {alias} --read-only`")), "{}", stderr(&cloned));
    assert!(server.requests().iter().any(|it| it.path == "/oauth/revocations" && it.body.contains("token=wb_rt_2")));
}

#[test]
fn a_renewal_narrowed_to_read_only_stops_a_push_before_it_changes_anything() {
    let server = fake_writebook_renewing(r#","scope":"read write""#, r#","scope":"read""#);
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    writebook(&["instance", "add", &server.url], &env);
    writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], &env);

    let checkout = tempdir().join("handbook");
    let cloned = writebook(&["clone", &format!("{}/4/handbook", server.url), checkout.to_str().unwrap()], &env);
    assert!(cloned.status.success(), "{}", stderr(&cloned));
    std::fs::write(checkout.join("on-call.md"), "Page the person on call.\n").unwrap();
    expire_sign_ins(&config);

    let pushed = writebook_in(&checkout, &["push"], &env);

    assert!(!pushed.status.success());
    assert!(stderr(&pushed).contains("is read-only, which can't change books"), "{}", stderr(&pushed));
    assert!(server.requests().iter().all(|it| it.method == "GET" || it.path.starts_with("/oauth")));
}

#[test]
fn says_so_when_a_full_access_renewal_could_not_be_revoked() {
    let server = Server::start(|request| match request.path.as_str() {
        "/oauth/device_authorizations" => support::device_authorization(request),
        "/oauth/tokens" if request.body.contains("grant_type=refresh_token") => reply(
            200,
            r#"{"access_token":"wb_at_2","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_2","scope":"read write"}"#,
        ),
        "/oauth/tokens" => {
            reply(200, r#"{"access_token":"wb_at_1","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_1","scope":"read"}"#)
        }
        _ => reply(500, ""),
    });
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    sign_in_read_only(&server, &env);
    expire_sign_ins(&config);

    let listed = writebook(&["book", "list", "--instance", &server.url], &env);

    assert!(stderr(&listed).contains("so it wasn't kept, but telling"), "{}", stderr(&listed));
    assert!(stderr(&listed).contains("to revoke it failed"), "{}", stderr(&listed));
}

#[test]
fn an_expired_read_only_sign_in_link_says_to_start_again_read_only() {
    let server = fake_writebook(r#","scope":"read""#);
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    writebook(&["instance", "add", &server.url, "--read-only"], &env);

    let path = config.join("writebook/pending-sign-ins.json");
    let mut pending: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for sign_in in pending["sign_ins"].as_object_mut().unwrap().values_mut() {
        sign_in["expires_at"] = 0.into();
    }
    std::fs::write(&path, pending.to_string()).unwrap();

    let finished = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], &env);

    assert!(stderr(&finished).contains(&format!("Start again with `writebook instance add {} --read-only`", server.url)), "{}", stderr(&finished));
}

/// The checkout has no index.md, so getting as far as reading it means the
/// token was let through.
#[test]
fn a_supplied_token_pushes_whatever_profile_is_picked() {
    let server = fake_writebook(r#","scope":"read""#);
    let config = tempdir();
    sign_in_read_only(&server, &[("XDG_CONFIG_HOME", config.to_str().unwrap())]);
    let checkout = checkout_of(&server);

    let pushed = writebook_in(
        &checkout,
        &["--profile", "nobody", "push"],
        &[("XDG_CONFIG_HOME", config.to_str().unwrap()), ("WRITEBOOK_TOKEN", "wb_at_secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)],
    );

    assert!(stderr(&pushed).contains("index.md"), "{}", stderr(&pushed));
}

/// Its token responses end with the given scope, or none, like a Writebook
/// from before read-only sign-ins.
fn fake_writebook(scope: &'static str) -> Server {
    fake_writebook_renewing(scope, scope)
}

/// Like fake_writebook, with its own scope for renewals, which hand out
/// wb_at_2 and wb_rt_2.
fn fake_writebook_renewing(scope: &'static str, renewed_scope: &'static str) -> Server {
    Server::start(move |request| match request.path.as_str() {
        "/oauth/device_authorizations" => support::device_authorization(request),
        "/oauth/tokens" if request.body.contains("grant_type=refresh_token") => reply(
            200,
            &format!(r#"{{"access_token":"wb_at_2","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_2"{renewed_scope}}}"#),
        ),
        "/oauth/tokens" => {
            reply(200, &format!(r#"{{"access_token":"wb_at_1","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_1"{scope}}}"#))
        }
        "/oauth/revocations" => reply(200, ""),
        "/books.json" => reply(200, "[]"),
        "/books/4/manifest.json" => reply(
            200,
            r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
                "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#,
        ),
        "/books/4/pages/12.json" => reply(200, r#"{"id":12,"title":"On call","fingerprint":"g","record_gid":"gid://writebook/Page/12","body":"Call someone."}"#),
        _ => reply(404, ""),
    })
}

fn sign_in_read_only(server: &Server, env: &[(&str, &str)]) {
    writebook(&["instance", "add", &server.url, "--read-only"], env);
    let finished = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], env);
    assert!(finished.status.success(), "{}", stderr(&finished));
}

fn expire_sign_ins(config: &std::path::Path) {
    let path = config.join("writebook/credentials.json");
    let mut credentials: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    for sign_in in credentials["sign_ins"].as_object_mut().unwrap().values_mut() {
        sign_in["expires_at"] = 0.into();
    }

    std::fs::write(&path, credentials.to_string()).unwrap();
}

fn checkout_of(server: &Server) -> std::path::PathBuf {
    let checkout = tempdir();
    std::fs::create_dir_all(checkout.join(".writebook")).unwrap();
    std::fs::write(checkout.join(".writebook/config.json"), format!(r#"{{"remotes":{{"origin":{{"url":"{}","book_id":4}}}}}}"#, server.url)).unwrap();
    checkout
}
