//! Signing in without a terminal or a browser: `writebook login` prints the
//! link, and `writebook login --code` finishes with the approval code.

mod support;

use support::{Server, reply, stderr, stdout, writebook};

const AUTHORIZATION: &str = r#"{"device_code":"wb_dc_secret","user_code":"ABCD-EFGH","verification_uri":"http://x/oauth/device_verification","verification_uri_complete":"http://x/oauth/device_verification?user_code=ABCD-EFGH","expires_in":600,"interval":5}"#;
const TOKENS: &str = r#"{"access_token":"wb_at_new","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_new"}"#;

fn fake_writebook() -> Server {
    Server::start(|request| match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/oauth/device_authorizations") => reply(200, AUTHORIZATION),
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

    let started = writebook(&["login", &server.url], &env);
    assert!(started.status.success(), "{}", stderr(&started));
    assert!(stdout(&started).contains("http://x/oauth/device_verification?user_code=ABCD-EFGH"), "{}", stdout(&started));
    assert!(stdout(&started).contains("--code"), "{}", stdout(&started));

    let wrong = writebook(&["login", &server.url, "--code", "AAAA-AAAA"], &env);
    assert!(!wrong.status.success());
    assert!(stderr(&wrong).contains("doesn't match"), "{}", stderr(&wrong));

    let finished = writebook(&["login", &server.url, "--code", "WXYZ-2345"], &env);
    assert!(finished.status.success(), "{}", stderr(&finished));

    let token_request = server.requests().into_iter().rev().find(|it| it.path == "/oauth/tokens").unwrap();
    assert!(token_request.body.contains("device_code=wb_dc_secret"), "{}", token_request.body);

    let credentials = std::fs::read_to_string(config.join("writebook/credentials.json")).unwrap();
    assert!(credentials.contains("wb_at_new"), "{credentials}");

    let again = writebook(&["login", &server.url, "--code", "WXYZ-2345"], &env);
    assert!(!again.status.success(), "the pending sign-in is used up");
    assert!(stderr(&again).contains("writebook login"), "{}", stderr(&again));
}

#[test]
fn a_code_without_a_started_sign_in_explains_what_to_run() {
    let server = fake_writebook();

    let output = writebook(&["login", &server.url, "--code", "WXYZ-2345"], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains(&format!("writebook login {}", server.url)), "{}", stderr(&output));
    assert!(server.requests().is_empty());
}
