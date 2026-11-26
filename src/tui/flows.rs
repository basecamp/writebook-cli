//! The TUI driven end to end: keys go in, the screen is drawn into a test
//! terminal, and every request goes to a fake Writebook. Each test runs in a
//! process of its own under nextest, so each signs in to its own server.

#[path = "../../tests/support/server.rs"]
mod server;

mod book_editing;
mod browsing;
mod page_editing;
mod people;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(60);

pub use server::{Reply, Request, Server, reply};

use super::data::Instance;
use super::images::Images;
use super::{App, Loading};

pub struct Tui {
    pub app: App,
    pub server: Server,
    terminal: Terminal<TestBackend>,
}

impl Tui {
    /// The TUI on its first screen, with one instance: the fake Writebook,
    /// signed in with a token that may change books.
    pub fn new(route: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Tui {
        Tui::signed_in_with("wb_test_token", route)
    }

    /// Like new, signed in with an API token that may only read.
    pub fn read_only(route: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Tui {
        Tui::signed_in_with("wb_api_test_token", route)
    }

    pub fn press(&mut self, code: KeyCode) {
        self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    pub fn ctrl(&mut self, character: char) {
        self.app
            .key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL));
    }

    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.press(KeyCode::Char(character));
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.app.paste(text);
    }

    pub fn wait_for(&mut self, text: &str) -> String {
        self.wait_until(text, |screen| screen.contains(text))
    }

    pub fn wait_for_gone(&mut self, text: &str) -> String {
        self.wait_until(text, |screen| !screen.contains(text))
    }

    pub fn wait_for_request(&mut self, method: &str, path: &str) -> Request {
        self.eventually(
            |tui| {
                tui.server
                    .requests()
                    .into_iter()
                    .find(|it| it.method == method && it.path == path)
            },
            |tui| format!("no {method} {path} reached the server; it saw {:?}", tui.paths()),
        )
    }

    pub fn wait_for_request_count(&mut self, count: usize) {
        self.eventually(
            |tui| {
                if tui.server.requests().len() < count {
                    None
                } else {
                    Some(())
                }
            },
            |tui| format!("the server saw only {:?}", tui.paths()),
        )
    }

    /// Take in replies until none arrives for a while: for checking that a
    /// late one changed nothing.
    pub fn settle(&mut self) {
        while let Ok(message) = self.app.receiver.recv_timeout(Duration::from_millis(200)) {
            self.app.receive(message);
        }
    }

    /// Every method and path the server saw, in order.
    pub fn paths(&self) -> Vec<String> {
        self.server
            .requests()
            .iter()
            .map(|it| format!("{} {}", it.method, it.path))
            .collect()
    }

    pub fn screen(&mut self) -> String {
        self.terminal.draw(|frame| self.app.draw(frame)).unwrap();

        let buffer = self.terminal.backend().buffer();
        let width = buffer.area.width as usize;
        let symbols: Vec<&str> = buffer.content.iter().map(|cell| cell.symbol()).collect();

        symbols
            .chunks(width)
            .map(|row| row.concat().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn signed_in_with(token: &str, route: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Tui {
        let server = Server::start(route);
        crate::creds::use_token(token, Some(&server.url));
        assert!(
            crate::creds::signed_in_token(&server.url).is_ok_and(|it| it.is_some()),
            "the token is set once a process, so run these with `bin/dev test run`, which gives each test a process of its own"
        );

        let mut app = App::new(Images::none());
        app.instances = Loading::Ready(vec![Instance {
            alias: "books".to_string(),
            url: server.url.clone(),
            status: "signed in".to_string(),
        }]);

        Tui {
            app,
            server,
            terminal: Terminal::new(TestBackend::new(100, 40)).unwrap(),
        }
    }

    fn wait_until(&mut self, what: &str, done: impl Fn(&str) -> bool) -> String {
        self.eventually(
            |tui| {
                let screen = tui.screen();
                if done(&screen) { Some(screen) } else { None }
            },
            |tui| format!("gave up waiting on {what:?}; the screen shows:\n{}", tui.screen()),
        )
    }

    fn eventually<T>(
        &mut self,
        mut check: impl FnMut(&mut Tui) -> Option<T>,
        failure: impl Fn(&mut Tui) -> String,
    ) -> T {
        let started = Instant::now();
        let mut found = check(self);

        while found.is_none() {
            assert!(started.elapsed() < PATIENCE, "{}", failure(self));
            if let Ok(message) = self.app.receiver.recv_timeout(Duration::from_millis(20)) {
                self.app.receive(message);
            }
            found = check(self);
        }

        found.expect("the loop ends once the check finds a value")
    }
}

/// A fake Writebook's answer with a JSON body.
pub fn json(body: serde_json::Value) -> Reply {
    reply(200, &body.to_string())
}

pub fn not_found() -> Reply {
    reply(404, r#"{"error":"not found"}"#)
}

/// The request's JSON body.
pub fn body(request: &Request) -> serde_json::Value {
    serde_json::from_str(&request.body).unwrap()
}
