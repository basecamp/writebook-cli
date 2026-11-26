#![cfg(test)]

#[cfg(test)]
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use support::{Server, bytes, reply, stderr, stdout, tempdir, writebook_in};

const PICTURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0xff, 0xfe, 0x00];

#[test]
fn a_page_that_isnt_text_stops_status_and_push_with_its_path() {
    let server = fake_writebook(Arc::new(AtomicBool::new(false)));
    let scratch = tempdir();
    let checkout = clone(&server, &scratch);
    std::fs::write(checkout.join("on-call.md"), [b'C', b'a', b'l', b'l', 0xff, b'\n']).unwrap();

    let status = writebook_in(&checkout, &["status"], &token_for(&server));
    assert!(!status.status.success());
    assert!(
        stderr(&status).contains(&format!("could not read {}", checkout.join("on-call.md").display())),
        "{}",
        stderr(&status)
    );

    let pushed = writebook_in(&checkout, &["push"], &token_for(&server));
    assert!(!pushed.status.success());
    assert!(
        stderr(&pushed).contains(&format!("could not read {}", checkout.join("on-call.md").display())),
        "{}",
        stderr(&pushed)
    );
    assert!(server.requests().iter().all(|it| it.method == "GET"));
}

#[test]
fn a_picture_that_isnt_text_is_compared_by_its_bytes() {
    let server = fake_writebook(Arc::new(AtomicBool::new(false)));
    let scratch = tempdir();
    let checkout = clone(&server, &scratch);
    assert_eq!(std::fs::read(checkout.join("assets/diagram.png")).unwrap(), PICTURE);

    let status = writebook_in(&checkout, &["status"], &token_for(&server));

    assert!(status.status.success(), "{}", stderr(&status));
    assert!(!stdout(&status).contains("diagram.png"), "{}", stdout(&status));
}

#[cfg(unix)]
#[test]
fn a_cover_that_cant_be_removed_is_a_warning_and_stays() {
    use std::os::unix::fs::PermissionsExt;

    let cover_removed = Arc::new(AtomicBool::new(false));
    let server = fake_writebook(cover_removed.clone());
    let scratch = tempdir();
    let checkout = clone(&server, &scratch);
    let assets = checkout.join("assets");
    let cover = assets.join("cover-cover.png");
    assert!(cover.is_file());

    cover_removed.store(true, Ordering::SeqCst);
    std::fs::set_permissions(&assets, std::fs::Permissions::from_mode(0o555)).unwrap();
    let pulled = writebook_in(&checkout, &["pull"], &token_for(&server));
    std::fs::set_permissions(&assets, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(pulled.status.success(), "{}", stderr(&pulled));
    assert!(
        stderr(&pulled).contains(&format!(
            "warning: could not remove {}: Permission denied",
            cover.display()
        )),
        "{}",
        stderr(&pulled)
    );
    assert!(cover.is_file());
    let index = std::fs::read_to_string(checkout.join("index.md")).unwrap();
    assert!(index.contains("\ncover: \n"), "{index}");
}

fn fake_writebook(cover_removed: Arc<AtomicBool>) -> Server {
    Server::start(move |request| match request.path.as_str() {
        "/books/4/manifest.json" => {
            let (fingerprint, cover) = if cover_removed.load(Ordering::SeqCst) {
                ("f2", "null".to_string())
            } else {
                (
                    "f",
                    r#"{"checksum":"c","filename":"cover.png","url":"/u/cover.png"}"#.to_string(),
                )
            };

            reply(
                200,
                &format!(
                    r#"{{"book":{{"id":4,"title":"Handbook","subtitle":null,"author":"Ops","theme":"blue","slug":"handbook","fingerprint":"{fingerprint}","cover":{cover}}},
                        "leaves":[{{"id":12,"title":"On call","type":"Page","fingerprint":"g"}},
                                  {{"id":13,"title":"Diagram","type":"Picture","fingerprint":"h"}}]}}"#
                ),
            )
        }
        "/books/4/pages/12.json" => reply(
            200,
            r#"{"id":12,"title":"On call","fingerprint":"g","body":"Call someone.\n"}"#,
        ),
        "/books/4/pictures/13.json" => reply(
            200,
            r#"{"id":13,"title":"Diagram","fingerprint":"h","caption":"Diagram","image":{"filename":"diagram.png","url":"/u/diagram.png","checksum":"d"}}"#,
        ),
        "/u/diagram.png" | "/u/cover.png" => bytes(200, PICTURE),
        _ => reply(404, ""),
    })
}

fn clone(server: &Server, scratch: &std::path::Path) -> std::path::PathBuf {
    let checkout = scratch.join("handbook");
    let cloned = writebook_in(
        scratch,
        &[
            "clone",
            &format!("{}/4/handbook", server.url),
            checkout.to_str().unwrap(),
        ],
        &token_for(server),
    );
    assert!(cloned.status.success(), "{}", stderr(&cloned));
    checkout
}

fn token_for(server: &Server) -> [(&str, &str); 2] {
    [("WRITEBOOK_TOKEN", "secret"), ("WRITEBOOK_TOKEN_HOST", &server.url)]
}
