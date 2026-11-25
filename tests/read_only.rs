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

/// Its token responses end with the given scope, or none, like a Writebook
/// from before read-only sign-ins.
fn fake_writebook(scope: &'static str) -> Server {
    Server::start(move |request| match request.path.as_str() {
        "/oauth/device_authorizations" => support::device_authorization(request),
        "/oauth/tokens" => {
            reply(200, &format!(r#"{{"access_token":"wb_at_1","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_1"{scope}}}"#))
        }
        "/oauth/revocations" => reply(200, ""),
        _ => reply(404, ""),
    })
}

fn checkout_of(server: &Server) -> std::path::PathBuf {
    let checkout = tempdir();
    std::fs::create_dir_all(checkout.join(".writebook")).unwrap();
    std::fs::write(checkout.join(".writebook/config.json"), format!(r#"{{"remotes":{{"origin":{{"url":"{}","book_id":4}}}}}}"#, server.url)).unwrap();
    checkout
}
