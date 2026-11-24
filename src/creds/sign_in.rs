//! Signing in through the device flow, on machines with or without a browser.
//!
//! Writebook gives the person a link to approve. Nothing has to open on this
//! machine: they can approve on any device. Afterwards Writebook shows an
//! approval code, and typing it here finishes the sign-in. At a terminal we
//! also keep polling, so waiting works too. Without a terminal (an agent, a
//! script), `writebook instance add` splits the flow in two steps: it prints
//! the link and remembers the pending sign-in, and running it again with
//! `--code <code>` finishes it.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use super::{
    CLIENT_ID, Credentials, DEVICE_CODE_GRANT_TYPE, config_dir, credentials_from, error_description, now,
    post_token_request, request_device_authorization, someone_can_approve, storage, string_field,
};
use crate::{fsutil, inventory};

#[derive(Serialize, Deserialize, Clone)]
struct Pending {
    device_code: String,
    user_code: String,
    link: String,
    expires_at: u64,
    interval: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct PendingFile {
    #[serde(default)]
    sign_ins: BTreeMap<String, Pending>,
}

/// What came back from trying an approval code.
enum Attempt {
    SignedIn(Credentials),
    WrongCode,
    NotApprovedYet,
}

/// Sign in to a site as a profile: all at once at a terminal, otherwise in two
/// steps, finishing with the approval code. False when only the first step
/// is done.
pub fn sign_in(host: &str, profile: &str, code: Option<&str>) -> Result<bool> {
    match code {
        Some(code) => finish_pending(host, profile, code).map(|_| true),
        None if someone_can_approve() => at_terminal(host, profile).map(|_| true),
        None => start_pending(host, profile).map(|_| false),
    }
}

/// The whole flow in one go, for a person at a terminal: print the link, then
/// take the approval code they type, or notice the approval by polling.
pub(super) fn at_terminal(host: &str, profile: &str) -> Result<Credentials> {
    let pending = start(host)?;

    println!("Sign in to {host} to connect this device.");
    println!();
    println!("  Open this link on any device: {}", pending.link);
    println!("  Check that it shows this code: {}", pending.user_code);
    println!();
    println!("After you approve, Writebook shows an approval code. Type it here and press Enter,");
    println!("or just wait here until the approval is noticed.");

    if has_display() {
        open_in_browser(&pending.link);
    }

    let (sender, receiver) = mpsc::channel();

    {
        let sender = sender.clone();
        let host = host.to_string();
        let pending = pending.clone();
        std::thread::spawn(move || {
            let _ = sender.send(poll_until_approved(&host, &pending));
        });
    }

    {
        let host = host.to_string();
        let pending = pending.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let code = line.trim();

                if code.is_empty() {
                    continue;
                }

                match try_code(&host, &pending, code) {
                    Ok(Attempt::SignedIn(credentials)) => {
                        let _ = sender.send(Ok(credentials));
                        return;
                    }
                    Ok(Attempt::WrongCode) => eprintln!("That code doesn't match. Check it and type it again:"),
                    Ok(Attempt::NotApprovedYet) => {
                        eprintln!("Writebook hasn't seen the approval yet. Approve at the link first, then type the code:")
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        return;
                    }
                }
            }
        });
    }

    let credentials = receiver.recv().context("sign-in stopped unexpectedly")??;
    storage::save(host, profile, &credentials)?;
    println!("Signed in.");
    Ok(credentials)
}

/// Step one without a terminal: print the link and remember the sign-in.
fn start_pending(host: &str, profile: &str) -> Result<()> {
    let pending = start(host)?;

    let mut file = load_pending()?;
    file.sign_ins.insert(pending_key(host, profile), pending.clone());
    fsutil::write_json_privately(&pending_path(), &file)?;

    let minutes = pending.expires_at.saturating_sub(now()).div_ceil(60);

    println!("To sign in to {host}:");
    println!();
    println!("  1. Open {} on any device, check that it shows the code {}, and approve it.", pending.link, pending.user_code);
    println!("  2. Writebook then shows an approval code. Finish by running the same command again with:");
    println!();
    println!("     --code <approval code>");
    println!();
    println!("The link works for {minutes} minutes.");
    Ok(())
}

/// Step two without a terminal: trade the approval code for a sign-in.
fn finish_pending(host: &str, profile: &str, code: &str) -> Result<()> {
    let mut file = load_pending()?;
    let key = pending_key(host, profile);
    let login = inventory::sign_in_command(host, profile);

    let Some(pending) = file.sign_ins.get(&key).cloned() else {
        bail!("no sign-in to {host} is waiting for a code. Start one with `{login}`");
    };

    if pending.expires_at <= now() {
        file.sign_ins.remove(&key);
        fsutil::write_json_privately(&pending_path(), &file)?;
        bail!("the sign-in link for {host} has expired. Start again with `{login}`");
    }

    match try_code(host, &pending, code.trim())? {
        Attempt::SignedIn(credentials) => {
            storage::save(host, profile, &credentials)?;
            file.sign_ins.remove(&key);
            fsutil::write_json_privately(&pending_path(), &file)?;
            println!("Signed in to {host}.");
            Ok(())
        }
        Attempt::WrongCode => bail!("that approval code doesn't match. Check the code Writebook showed and try again"),
        Attempt::NotApprovedYet => bail!("{host} hasn't seen the approval yet. Approve at {} first", pending.link),
    }
}

fn start(host: &str) -> Result<Pending> {
    let authorization = request_device_authorization(host)?;

    Ok(Pending {
        device_code: string_field(&authorization, "device_code")?,
        user_code: string_field(&authorization, "user_code")?,
        link: string_field(&authorization, "verification_uri_complete")?,
        expires_at: now() + authorization["expires_in"].as_u64().unwrap_or(600),
        interval: authorization["interval"].as_u64().unwrap_or(5),
    })
}

fn try_code(host: &str, pending: &Pending, code: &str) -> Result<Attempt> {
    let (status, body) = post_token_request(
        host,
        &[
            ("client_id", CLIENT_ID),
            ("grant_type", DEVICE_CODE_GRANT_TYPE),
            ("device_code", &pending.device_code),
            ("approval_code", code),
        ],
    )?;

    if status == 200 {
        return Ok(Attempt::SignedIn(credentials_from(&body)?));
    }

    match body["error"].as_str() {
        Some("invalid_grant") => Ok(Attempt::WrongCode),
        // A Writebook without approval codes ignores the code and answers as
        // it would a poll.
        Some("authorization_pending" | "slow_down") => Ok(Attempt::NotApprovedYet),
        _ => Err(sign_in_error(&body)),
    }
}

fn poll_until_approved(host: &str, pending: &Pending) -> Result<Credentials> {
    let mut interval = pending.interval;

    loop {
        std::thread::sleep(Duration::from_secs(interval));

        if now() > pending.expires_at {
            bail!("the sign-in link expired before it was approved — try again");
        }

        let (status, body) = post_token_request(
            host,
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", DEVICE_CODE_GRANT_TYPE),
                ("device_code", &pending.device_code),
            ],
        )?;

        if status == 200 {
            return credentials_from(&body);
        }

        match body["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some("invalid_grant") => {
                // Most likely a typed approval code just used this sign-in;
                // give that answer the chance to arrive first.
                std::thread::sleep(Duration::from_secs(2));
                return Err(sign_in_error(&body));
            }
            _ => return Err(sign_in_error(&body)),
        }
    }
}

fn sign_in_error(body: &Value) -> anyhow::Error {
    match body["error"].as_str() {
        Some("expired_token") => anyhow::anyhow!("the sign-in link expired before it was approved — try again"),
        Some("access_denied") => anyhow::anyhow!("the sign-in was denied"),
        _ => anyhow::anyhow!("sign-in failed: {}", error_description(body)),
    }
}

/// Whether a browser could open here at all. Over SSH or on a server there is
/// nothing to open, and the printed link is the way in.
fn has_display() -> bool {
    if cfg!(target_os = "macos") {
        return std::env::var_os("SSH_CONNECTION").is_none();
    }

    ["DISPLAY", "WAYLAND_DISPLAY"].iter().any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };

    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn pending_key(host: &str, profile: &str) -> String {
    format!("{host} {profile}")
}

fn load_pending() -> Result<PendingFile> {
    let path = pending_path();

    if path.exists() {
        let contents = fsutil::read_to_string(&path)?;
        serde_json::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
    } else {
        Ok(PendingFile::default())
    }
}

fn pending_path() -> PathBuf {
    config_dir().join("pending-sign-ins.json")
}
