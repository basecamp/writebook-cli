//! A tiny HTTP server for tests: canned answers by method and path, and a
//! record of every request it saw. Each connection gets one answer and is
//! closed, which is all the CLI needs.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

pub fn reply(status: u16, body: &str) -> Reply {
    Reply { status, headers: Vec::new(), body: body.to_string() }
}

/// Writebook's answer to starting a sign-in, with the link on the host asked.
pub fn device_authorization(request: &Request) -> Reply {
    let link = format!("http://{}/oauth/device_verification", request.header("Host").unwrap());

    reply(
        200,
        &format!(
            r#"{{"device_code":"wb_dc_secret","user_code":"ABCD-EFGH","verification_uri":"{link}","verification_uri_complete":"{link}?user_code=ABCD-EFGH","expires_in":600,"interval":5}}"#
        ),
    )
}

pub fn redirect(location: &str) -> Reply {
    Reply { status: 302, headers: vec![("Location", location.to_string())], body: String::new() }
}

pub struct Server {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    pub fn start(route: impl Fn(&Request) -> Reply + Send + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_request(&mut stream) else { continue };
                let answer = route(&request);
                seen.lock().unwrap().push(request);

                let mut head =
                    format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", answer.status, answer.body.len());
                for (name, value) in &answer.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");

                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(answer.body.as_bytes());
            }
        });

        Server { url, requests }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;

    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;

    Some(Request { method, path, headers, body: String::from_utf8_lossy(&body).to_string() })
}

/// Run the CLI with no terminal attached, an empty config dir, and only the
/// environment given (no WRITEBOOK_TOKEN leaking in from the developer's shell).
pub fn writebook(args: &[&str], env: &[(&str, &str)]) -> Output {
    writebook_in(&std::env::current_dir().unwrap(), args, env)
}

/// Like writebook, run in the given directory.
pub fn writebook_in(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let config = tempdir();
    let mut command = Command::new(env!("CARGO_BIN_EXE_writebook"));

    command
        .current_dir(dir)
        .args(args)
        .env_remove("WRITEBOOK_TOKEN")
        .env_remove("WRITEBOOK_TOKEN_HOST")
        .env_remove("WRITEBOOK_PROFILE")
        .env_remove("WRITEBOOK_NO_SIGN_IN")
        .env_remove("CLAUDECODE")
        .env("XDG_CONFIG_HOME", &config)
        .env("WRITEBOOK_NO_KEYRING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in env {
        command.env(key, value);
    }

    command.output().unwrap()
}

pub fn tempdir() -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("writebook-test-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
