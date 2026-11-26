//! `writebook mcp`: books as MCP tools, on stdio for every instance you
//! added, and over HTTP for one instance with each caller's token.

#![cfg(test)]

#[cfg(test)]
mod support;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use support::{Server, reply, tempdir};

#[test]
fn reads_and_changes_pages_on_stdio() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, Some("wb_at_secret"));

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_instances",
            "list_books",
            "book_contents",
            "read_book",
            "read_page",
            "search",
            "create_page",
            "update_page",
            "create_section"
        ]
    );

    let instances = mcp.call("list_instances", json!({}));
    assert_eq!(
        instances,
        json!([{ "alias": "handbook", "url": server.url, "default": true, "can_change_books": true }])
    );

    let page = mcp.call("read_page", json!({ "book_id": 4, "page_id": 12 }));
    assert_eq!(
        page,
        json!({ "id": 12, "type": "page", "title": "On call", "markdown": "Call someone.", "fingerprint": "g" })
    );

    let stale = mcp.call_failing(
        "update_page",
        json!({ "book_id": 4, "page_id": 12, "fingerprint": "old", "markdown": "Page." }),
    );
    assert!(
        stale.contains("changed since it was read, so nothing was changed"),
        "{stale}"
    );

    let updated = mcp.call(
        "update_page",
        json!({ "book_id": 4, "page_id": 12, "fingerprint": "g", "markdown": "Page the person on call." }),
    );
    assert_eq!(updated, json!({ "id": 12, "fingerprint": "h" }));

    let patch = server.requests().into_iter().rfind(|it| it.method == "PATCH").unwrap();
    let sent: Value = serde_json::from_str(&patch.body).unwrap();
    assert_eq!(
        sent,
        json!({ "base_fingerprint": "g", "page": { "body": "Page the person on call." } })
    );
    assert_eq!(patch.header("Authorization"), Some("Bearer wb_at_secret"));
}

#[test]
fn a_read_only_token_gets_only_the_tools_that_read() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, Some("wb_api_secret"));

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "list_instances",
            "list_books",
            "book_contents",
            "read_book",
            "read_page",
            "search"
        ]
    );

    let refused = mcp.call_failing("create_page", json!({ "book_id": 4, "title": "New", "markdown": "" }));
    assert!(refused.contains("read-only"), "{refused}");
    assert!(server.requests().iter().all(|it| it.method == "GET"));
}

#[test]
fn reaches_the_instance_a_tool_names() {
    let server = fake_writebook();
    let other = fake_writebook();
    let mut mcp = Mcp::start_with(&server, Some("wb_at_secret"), &[&other]);

    let instances = mcp.call("list_instances", json!({}));
    assert_eq!(instances[0]["alias"], json!("handbook"));
    assert_eq!(instances[0]["can_change_books"], json!(true));
    assert_eq!(instances[1]["can_change_books"], json!(false));

    mcp.call("search", json!({ "query": "call" }));
    assert!(server.requests().iter().any(|it| it.path.starts_with("/search.json")));

    let elsewhere = mcp.call_failing("search", json!({ "query": "call", "instance": "other" }));
    assert!(
        elsewhere.contains(&format!("so it is not sent to {}", other.url)),
        "{elsewhere}"
    );
    assert!(other.requests().is_empty());

    let unknown = mcp.call_failing("list_books", json!({ "instance": "nope" }));
    assert!(unknown.contains("there is no instance 'nope'"), "{unknown}");
}

#[test]
fn says_so_when_not_signed_in_rather_than_reading_only_published_books() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, None);

    let refused = mcp.call_failing("list_books", json!({}));
    assert!(
        refused.contains(&format!("you're not signed in to {}", server.url)),
        "{refused}"
    );
    assert!(server.requests().is_empty());

    let instances = mcp.call("list_instances", json!({}));
    assert_eq!(instances[0]["can_change_books"], json!(false));
    let tools = mcp.request("tools/list", json!({}));
    assert!(!tools.to_string().contains("create_page"), "{tools}");
}

#[test]
fn search_returns_at_most_50_matches() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, Some("wb_at_secret"));

    let hits = mcp.call("search", json!({ "query": "page" }));

    assert_eq!(hits.as_array().unwrap().len(), 50);
}

#[test]
fn a_change_writebook_answers_without_an_id_is_an_error() {
    let server = fake_writebook();
    let mut mcp = Mcp::start(&server, Some("wb_at_secret"));

    let failed = mcp.call_failing("create_page", json!({ "book_id": 4, "title": "New", "markdown": "" }));

    assert!(failed.contains("had no id for the page"), "{failed}");
}

#[test]
fn needs_an_instance_on_stdio() {
    let output = support::writebook(&["mcp"], &[]);

    assert!(!output.status.success());
    assert!(
        support::stderr(&output).contains("there are no instances to serve"),
        "{}",
        support::stderr(&output)
    );
}

#[test]
fn serves_each_http_caller_with_their_own_token() {
    let server = fake_writebook();
    let config = tempdir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_writebook"))
        .args(["mcp", "--instance", &server.url, "--serve", "127.0.0.1:0"])
        .env_remove("WRITEBOOK_TOKEN")
        .env("XDG_CONFIG_HOME", &config)
        .env("WRITEBOOK_NO_KEYRING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut logs = BufReader::new(child.stderr.take().unwrap());
    let mut serving = String::new();
    logs.read_line(&mut serving).unwrap();
    let serving: Value = serde_json::from_str(&serving).unwrap();
    assert_eq!(
        (&serving["event"], &serving["instance"], &serving["transport"]),
        (&json!("serving"), &json!(server.url), &json!("http"))
    );
    let endpoint = serving["address"].as_str().unwrap().to_string();

    let anonymous = post(
        &endpoint,
        None,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    assert_eq!(anonymous.0, 401);

    post(&endpoint, Some("wb_at_alice"), &initialize());
    let (_, tools) = post(
        &endpoint,
        Some("wb_at_alice"),
        &json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" }),
    );
    assert!(
        !tools.contains("list_instances") && !tools.contains("\"instance\""),
        "{tools}"
    );
    let (status, answer) = post(
        &endpoint,
        Some("wb_at_alice"),
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "list_books", "arguments": {} } }),
    );
    assert_eq!(status, 200, "{answer}");
    assert!(answer.contains("Handbook"), "{answer}");

    let books = server
        .requests()
        .into_iter()
        .find(|it| it.path == "/books.json")
        .unwrap();
    assert_eq!(books.header("Authorization"), Some("Bearer wb_at_alice"));

    let (_, revoked) = post(
        &endpoint,
        Some("wb_at_revoked"),
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_books", "arguments": {} } }),
    );
    assert!(revoked.contains("did not accept your Writebook token"), "{revoked}");

    child.kill().unwrap();
    child.wait().unwrap();

    let mut rest = String::new();
    std::io::Read::read_to_string(&mut logs, &mut rest).unwrap();
    let events: Vec<Value> = rest.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let calls: Vec<(&str, &str, Option<&str>)> = events
        .iter()
        .filter(|it| it["event"] == "tool_call")
        .map(|it| {
            (
                it["tool"].as_str().unwrap(),
                it["outcome"].as_str().unwrap(),
                it["reason"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("list_books", "ok", None),
            ("list_books", "error", Some("token_rejected"))
        ]
    );
    assert!(
        events
            .iter()
            .any(|it| it["event"] == "http_request" && it["status"] == 401 && it["method"] == "POST"),
        "{rest}"
    );

    for word in ["wb_at_alice", "wb_at_revoked", "127.0.0.1", "Handbook"] {
        assert!(!rest.contains(word), "the logs mention a token, an address or a title");
    }
}

fn fake_writebook() -> Server {
    Server::start(|request| match (request.method.as_str(), request.path.as_str()) {
        _ if request.header("Authorization") == Some("Bearer wb_at_revoked") => reply(401, ""),
        ("GET", path) if path.starts_with("/search.json?q=page") => {
            let hits: Vec<Value> =
                (1..=60).map(|n| json!({ "title": format!("Page {n}"), "url": "http://books.test/4/handbook/12/page", "book": { "title": "Handbook" } })).collect();
            reply(200, &json!({ "results": hits }).to_string())
        }
        ("GET", path) if path.starts_with("/search.json") => reply(200, r#"{"results":[]}"#),
        ("POST", "/books/4/pages.json") => reply(201, "{}"),
        ("GET", "/books.json") => reply(
            200,
            r#"[{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","slug":"handbook","url":"http://books.test/4/handbook"}]"#,
        ),
        ("GET", "/books/4/manifest.json") => reply(
            200,
            r#"{"book":{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"f","cover":null},
                "leaves":[{"id":12,"title":"On call","type":"Page","fingerprint":"g"}]}"#,
        ),
        ("GET", "/books/4/pages/12.json") => reply(
            200,
            r#"{"id":12,"title":"On call","fingerprint":"g","body":"Call someone."}"#,
        ),
        ("PATCH", "/books/4/pages/12.json")
            if serde_json::from_str::<Value>(&request.body).unwrap()["base_fingerprint"] == "g" =>
        {
            reply(200, r#"{"id":12,"fingerprint":"h"}"#)
        }
        ("PATCH", "/books/4/pages/12.json") => reply(409, r#"{"error":"stale"}"#),
        _ => reply(404, ""),
    })
}

struct Mcp {
    child: Child,
    output: BufReader<ChildStdout>,
    next_id: u64,
    _config: support::TempDir,
}

impl Mcp {
    fn start(server: &Server, token: Option<&str>) -> Mcp {
        Mcp::start_with(server, token, &[])
    }

    fn start_with(server: &Server, token: Option<&str>, others: &[&Server]) -> Mcp {
        let config = tempdir();
        let mut inventory = format!("default = \"handbook\"\n{}", instance("handbook", &server.url));
        for (n, other) in others.iter().enumerate() {
            let alias = if n == 0 {
                "other".to_string()
            } else {
                format!("other{}", n + 1)
            };
            inventory += &instance(&alias, &other.url);
        }
        std::fs::create_dir_all(config.join("writebook")).unwrap();
        std::fs::write(config.join("writebook/instances.toml"), inventory).unwrap();

        let mut command = Command::new(env!("CARGO_BIN_EXE_writebook"));
        command
            .args(["mcp"])
            .env_remove("WRITEBOOK_TOKEN")
            .env("XDG_CONFIG_HOME", &config);

        if let Some(token) = token {
            command
                .env("WRITEBOOK_TOKEN", token)
                .env("WRITEBOOK_TOKEN_HOST", &server.url);
        }

        let mut child = command
            .env("WRITEBOOK_NO_KEYRING", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut mcp = Mcp {
            child,
            output,
            next_id: 1,
            _config: config,
        };

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

        let mut message = Value::Null;
        while message["id"] != json!(id) {
            let mut line = String::new();
            assert_ne!(self.output.read_line(&mut line).unwrap(), 0, "writebook mcp stopped");
            message = serde_json::from_str(&line).unwrap();
        }
        message["result"].clone()
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        if let Err(error) = self.child.kill() {
            eprintln!("could not kill writebook mcp: {error}");
        }
        if let Err(error) = self.child.wait() {
            eprintln!("could not wait for writebook mcp: {error}");
        }
    }
}

fn instance(alias: &str, url: &str) -> String {
    format!("[instances.{alias}]\nurl = \"{url}\"\ndefault_profile = \"default\"\nprofiles = [\"default\"]\n")
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
    (
        response.status().as_u16(),
        response.body_mut().read_to_string().unwrap(),
    )
}
