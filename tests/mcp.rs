//! `writebook mcp`: a book's pages as MCP tools, on stdio with your sign-in,
//! and over HTTP with each caller's own token.

mod support;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use support::{Server, reply, tempdir};

#[test]
fn reads_and_changes_pages_on_stdio() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, "wb_at_secret");

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|it| it["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_books", "book_contents", "read_book", "read_page", "search", "create_page", "update_page", "create_section"]);

    let page = mcp.call("read_page", json!({ "book_id": 4, "page_id": 12 }));
    assert_eq!(page, json!({ "id": 12, "type": "page", "title": "On call", "markdown": "Call someone.", "fingerprint": "g" }));

    let stale = mcp.call_failing("update_page", json!({ "book_id": 4, "page_id": 12, "fingerprint": "old", "markdown": "Page." }));
    assert!(stale.contains("changed since it was read, so nothing was changed"), "{stale}");

    let updated = mcp.call("update_page", json!({ "book_id": 4, "page_id": 12, "fingerprint": "g", "markdown": "Page the person on call." }));
    assert_eq!(updated, json!({ "id": 12, "fingerprint": "h" }));

    let patch = server.requests().into_iter().rfind(|it| it.method == "PATCH").unwrap();
    let sent: Value = serde_json::from_str(&patch.body).unwrap();
    assert_eq!(sent, json!({ "base_fingerprint": "g", "page": { "body": "Page the person on call." } }));
    assert_eq!(patch.header("Authorization"), Some("Bearer wb_at_secret"));
}

#[test]
fn a_read_only_token_gets_only_the_tools_that_read() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, "wb_api_secret");

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|it| it["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_books", "book_contents", "read_book", "read_page", "search"]);

    let refused = mcp.call_failing("create_page", json!({ "book_id": 4, "title": "New", "markdown": "" }));
    assert!(refused.contains("read-only"), "{refused}");
    assert!(server.requests().iter().all(|it| it.method == "GET"));
}

#[test]
fn needs_a_sign_in_on_stdio() {
    let server = fake_writebook();

    let output = support::writebook(&["mcp", "--instance", &server.url], &[]);

    assert!(!output.status.success());
    assert!(support::stderr(&output).contains("there isn't one. Sign in with `writebook instance add"), "{}", support::stderr(&output));
}

#[test]
fn serves_each_http_caller_with_their_own_token() {
    let server = fake_writebook();
    let config = tempdir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_writebook"))
        .args(["mcp", "--instance", &server.url, "--server", "127.0.0.1:0"])
        .env_remove("WRITEBOOK_TOKEN")
        .env("XDG_CONFIG_HOME", &config)
        .env("WRITEBOOK_NO_KEYRING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut banner = String::new();
    BufReader::new(child.stderr.take().unwrap()).read_line(&mut banner).unwrap();
    let endpoint = banner.split_whitespace().find(|it| it.ends_with("/mcp.")).unwrap().trim_end_matches('.').to_string();

    let anonymous = post(&endpoint, None, &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    assert_eq!(anonymous.0, 401);

    post(&endpoint, Some("wb_at_alice"), &initialize());
    let (status, answer) = post(
        &endpoint,
        Some("wb_at_alice"),
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "list_books", "arguments": {} } }),
    );
    assert_eq!(status, 200, "{answer}");
    assert!(answer.contains("Handbook"), "{answer}");

    let books = server.requests().into_iter().find(|it| it.path == "/books.json").unwrap();
    assert_eq!(books.header("Authorization"), Some("Bearer wb_at_alice"));

    let (_, revoked) = post(
        &endpoint,
        Some("wb_at_revoked"),
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_books", "arguments": {} } }),
    );
    assert!(revoked.contains("did not accept your Writebook token"), "{revoked}");

    child.kill().unwrap();
    child.wait().unwrap();
}

fn fake_writebook() -> Server {
    Server::start(|request| match (request.method.as_str(), request.path.as_str()) {
        _ if request.header("Authorization") == Some("Bearer wb_at_revoked") => reply(401, ""),
        ("GET", "/books.json") => {
            reply(200, r#"[{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","slug":"handbook","url":"http://books.test/4/handbook"}]"#)
        }
        ("GET", "/books/4/manifest.json") => reply(
            200,
            r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
                "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#,
        ),
        ("GET", "/books/4/pages/12.json") => reply(200, r#"{"id":12,"title":"On call","fingerprint":"g","body":"Call someone."}"#),
        ("PATCH", "/books/4/pages/12.json") if serde_json::from_str::<Value>(&request.body).unwrap()["base_fingerprint"] == "g" => reply(200, r#"{"id":12,"fingerprint":"h"}"#),
        ("PATCH", "/books/4/pages/12.json") => reply(409, r#"{"error":"stale"}"#),
        _ => reply(404, ""),
    })
}

/// `writebook mcp` on stdio, signed in with the given WRITEBOOK_TOKEN.
struct Mcp {
    child: Child,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(server: &Server, token: &str) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_writebook"))
            .args(["mcp", "--instance", &server.url])
            .env("WRITEBOOK_TOKEN", token)
            .env("WRITEBOOK_TOKEN_HOST", &server.url)
            .env("XDG_CONFIG_HOME", tempdir())
            .env("WRITEBOOK_NO_KEYRING", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Mcp { child, output, next_id: 1 };

        let params = initialize()["params"].clone();
        mcp.request("initialize", params);
        mcp.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        mcp
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let result = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        assert_ne!(result["isError"], json!(true), "{result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn call_failing(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        assert_eq!(result["isError"], json!(true), "{result}");
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));

        loop {
            let mut line = String::new();
            assert_ne!(self.output.read_line(&mut line).unwrap(), 0, "writebook mcp stopped");
            let message: Value = serde_json::from_str(&line).unwrap();

            if message["id"] == json!(id) {
                return message["result"].clone();
            }
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } },
    })
}

fn post(endpoint: &str, token: Option<&str>, body: &Value) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut request = agent
        .post(endpoint)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-06-18");

    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }

    let mut response = request.send_json(body).unwrap();
    (response.status().as_u16(), response.body_mut().read_to_string().unwrap())
}
