//! Signing in without a terminal or a browser: `writebook instance add` prints
//! the link, and running it again with --code finishes with the approval code.

mod support;

use support::{Server, reply, stderr, stdout, writebook};

const TOKENS: &str = r#"{"access_token":"wb_at_new","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_new"}"#;

fn fake_writebook() -> Server {
    Server::start(|request| match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/oauth/device_authorizations") => support::device_authorization(request),
        ("POST", "/oauth/tokens") if request.body.contains("approval_code=WXYZ-2345") => reply(200, TOKENS),
        ("POST", "/oauth/tokens") => reply(400, r#"{"error":"invalid_grant","error_description":"no match"}"#),
        _ => reply(404, ""),
    })
}

#[test]
fn signs_in_in_two_steps_with_the_approval_code() {
    let server = fake_writebook();
    let config = support::tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];

    let add = ["instance", "add", server.url.as_str(), "--alias", "work"];

    let started = writebook(&add, &env);
    assert!(started.status.success(), "{}", stderr(&started));
    assert!(stdout(&started).contains(&format!("{}/oauth/device_verification?user_code=ABCD-EFGH", server.url)), "{}", stdout(&started));
    assert!(stdout(&started).contains("--code"), "{}", stdout(&started));
    assert!(stdout(&writebook(&["instance", "list"], &env)).contains("No instances"), "added only once signed in");

    let wrong = writebook(&[&add[..], &["--code", "AAAA-AAAA"]].concat(), &env);
    assert!(!wrong.status.success());
    assert!(stderr(&wrong).contains("doesn't match"), "{}", stderr(&wrong));

    let finished = writebook(&[&add[..], &["--code", "WXYZ-2345"]].concat(), &env);
    assert!(finished.status.success(), "{}", stderr(&finished));
    assert!(stdout(&writebook(&["instance", "list"], &env)).contains("Signed in as default"));

    let token_request = server.requests().into_iter().rev().find(|it| it.path == "/oauth/tokens").unwrap();
    assert!(token_request.body.contains("device_code=wb_dc_secret"), "{}", token_request.body);

    let credentials = std::fs::read_to_string(config.join("writebook/credentials.json")).unwrap();
    assert!(credentials.contains("wb_at_new"), "{credentials}");

    let again = writebook(&[&add[..], &["--code", "WXYZ-2345"]].concat(), &env);
    assert!(!again.status.success(), "the pending sign-in is used up");
    assert!(stderr(&again).contains("Start one with `writebook instance add work`"), "{}", stderr(&again));
}

#[test]
fn a_code_without_a_started_sign_in_explains_what_to_run() {
    let server = fake_writebook();

    let output = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains(&format!("writebook instance add {}", server.url)), "{}", stderr(&output));
    assert!(server.requests().is_empty());
}
