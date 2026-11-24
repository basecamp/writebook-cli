//! How sign-ins are kept and given up: only over https, in a folder only you
//! can read, renewed by one command at a time, and revoked on sign-out.

mod support;

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use support::{Server, reply, stderr, stdout, tempdir, writebook};

#[test]
fn signs_in_only_over_https_except_to_this_machine() {
    let output = writebook(&["instance", "add", "http://books.example.com"], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("isn't https"), "{}", stderr(&output));
}

#[test]
fn signing_out_revokes_the_sign_in_and_drops_unfinished_ones() {
    let server = fake_writebook(Arc::new(Mutex::new(Vec::new())));
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    sign_in(&server, &env);

    let started = writebook(&["profile", "add", "editor"], &env);
    assert!(started.status.success(), "{}", stderr(&started));
    assert!(pending_sign_ins(&config).contains("wb_dc_secret"));

    let removed = writebook(&["instance", "remove", &server.url], &env);
    assert!(removed.status.success(), "{}", stderr(&removed));

    let revocation = server.requests().into_iter().find(|it| it.path == "/oauth/revocations").expect("a revocation");
    assert!(revocation.body.contains("token=wb_rt_1"), "{}", revocation.body);
    assert!(revocation.body.contains("client_id=writebook-cli"), "{}", revocation.body);
    assert!(!pending_sign_ins(&config).contains("wb_dc_secret"));
    assert!(!std::fs::read_to_string(config.join("writebook/credentials.json")).unwrap().contains("wb_rt_1"));
}

#[test]
fn one_command_at_a_time_renews_a_sign_in() {
    let refreshes = Arc::new(Mutex::new(Vec::new()));
    let server = fake_writebook(refreshes.clone());
    let config = tempdir();
    let env = [("XDG_CONFIG_HOME", config.to_str().unwrap())];
    sign_in(&server, &env);
    expire_sign_ins(&config);

    let commands: Vec<_> = (0..2)
        .map(|_| {
            let config = config.clone();
            let url = server.url.clone();
            std::thread::spawn(move || writebook(&["book", "list", "--instance", &url], &[("XDG_CONFIG_HOME", config.to_str().unwrap())]))
        })
        .collect();

    for command in commands {
        let output = command.join().unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(!stderr(&output).contains("could not renew"), "{}", stderr(&output));
        assert!(stdout(&output).contains("Ops Handbook"), "{}", stdout(&output));
    }

    assert_eq!(*refreshes.lock().unwrap(), ["wb_rt_1"]);
}

#[cfg(unix)]
#[test]
fn keeps_sign_ins_in_a_folder_only_you_can_read() {
    use std::os::unix::fs::PermissionsExt;

    let server = fake_writebook(Arc::new(Mutex::new(Vec::new())));
    let config = tempdir();
    let folder = config.join("writebook");
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();

    sign_in(&server, &[("XDG_CONFIG_HOME", config.to_str().unwrap())]);

    assert_eq!(std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::metadata(folder.join("credentials.json")).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn never_writes_through_a_file_planted_where_a_temporary_file_goes() {
    let server = fake_writebook(Arc::new(Mutex::new(Vec::new())));
    let config = tempdir();
    let folder = config.join("writebook");
    let target = tempdir().join("target");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&target, "untouched").unwrap();

    for name in ["instances.tmp", "credentials.tmp", "pending-sign-ins.tmp"] {
        std::os::unix::fs::symlink(&target, folder.join(name)).unwrap();
    }

    sign_in(&server, &[("XDG_CONFIG_HOME", config.to_str().unwrap())]);

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
    assert!(!std::fs::symlink_metadata(folder.join("instances.toml")).unwrap().file_type().is_symlink());
}

/// Each sign-in gets wb_at_1 and wb_rt_1, and each renewal the next number. A
/// refresh token works once, as Writebook's do; the refresh tokens used are
/// recorded. Renewals take a moment, so two at once overlap.
fn fake_writebook(refreshes: Arc<Mutex<Vec<String>>>) -> Server {
    let issued = Arc::new(AtomicUsize::new(1));

    Server::start(move |request| match (request.method.as_str(), request.path.as_str()) {
        ("POST", "/oauth/device_authorizations") => support::device_authorization(request),
        ("POST", "/oauth/tokens") if request.body.contains("grant_type=refresh_token") => {
            let token = request.body.split('&').find_map(|it| it.strip_prefix("refresh_token=")).unwrap().to_string();
            let mut used = refreshes.lock().unwrap();

            if used.contains(&token) {
                reply(400, r#"{"error":"invalid_grant"}"#)
            } else {
                used.push(token);
                drop(used);
                std::thread::sleep(Duration::from_millis(300));
                let number = issued.fetch_add(1, Ordering::SeqCst) + 1;
                reply(200, &format!(r#"{{"access_token":"wb_at_{number}","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_{number}"}}"#))
            }
        }
        ("POST", "/oauth/tokens") => reply(200, r#"{"access_token":"wb_at_1","token_type":"Bearer","expires_in":3600,"refresh_token":"wb_rt_1"}"#),
        ("POST", "/oauth/revocations") => reply(200, ""),
        ("GET", "/books.json") if request.header("Authorization").is_some_and(|it| it != "Bearer wb_at_1") => reply(
            200,
            r#"[{"id":4,"title":"Ops Handbook","subtitle":null,"author":null,"slug":"ops-handbook","published":true,"url":"http://books.test/4/ops-handbook"}]"#,
        ),
        _ => reply(404, ""),
    })
}

fn sign_in(server: &Server, env: &[(&str, &str)]) {
    let started = writebook(&["instance", "add", &server.url], env);
    assert!(started.status.success(), "{}", stderr(&started));

    let finished = writebook(&["instance", "add", &server.url, "--code", "WXYZ-2345"], env);
    assert!(finished.status.success(), "{}", stderr(&finished));
}

fn expire_sign_ins(config: &Path) {
    let path = config.join("writebook/credentials.json");
    let mut credentials: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    for sign_in in credentials["sign_ins"].as_object_mut().unwrap().values_mut() {
        sign_in["expires_at"] = 0.into();
    }

    std::fs::write(&path, credentials.to_string()).unwrap();
}

fn pending_sign_ins(config: &Path) -> String {
    std::fs::read_to_string(config.join("writebook/pending-sign-ins.json")).unwrap_or_default()
}
