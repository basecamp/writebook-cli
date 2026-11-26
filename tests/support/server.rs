//! A tiny HTTP server for tests: canned answers by method and path, and a
//! record of every request it saw. Each connection gets one answer and is
//! closed, which is all the CLI needs.

#![allow(
    dead_code,
    reason = "each test binary includes this module and uses a different part of it"
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
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
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
    pub length: usize,
}

pub fn reply(status: u16, body: &str) -> Reply {
    bytes(status, body.as_bytes())
}

pub fn bytes(status: u16, body: &[u8]) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: body.to_vec(),
        length: body.len(),
    }
}

/// A reply whose Content-Length promises more than the body it sends before
/// closing the connection.
pub fn cut_short(status: u16, body: &str) -> Reply {
    Reply {
        length: body.len() + 100,
        ..reply(status, body)
    }
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
    Reply {
        headers: vec![("Location", location.to_string())],
        ..reply(302, "")
    }
}

pub struct Server {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    /// Each connection is answered on a thread of its own, so a slow answer
    /// holds up no other request.
    pub fn start(route: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let route = Arc::new(route);

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (route, seen) = (route.clone(), seen.clone());

                std::thread::spawn(move || answer(stream, &*route, &seen));
            }
        });

        Server { url, requests }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn answer(mut stream: std::net::TcpStream, route: &dyn Fn(&Request) -> Reply, seen: &Mutex<Vec<Request>>) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let answer = route(&request);
    seen.lock().unwrap().push(request);

    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        answer.status, answer.length
    );
    for (name, value) in &answer.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");

    for part in [head.as_bytes(), &answer.body] {
        if let Err(error) = stream.write_all(part) {
            eprintln!("client went away before the reply was sent: {error}");
        }
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

    Some(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}
