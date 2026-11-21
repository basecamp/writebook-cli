//! Per-host OAuth credentials, obtained through the RFC 8628 device flow and
//! refreshed transparently. Stored in ~/.config/writebook/credentials.json so
//! several checkouts share one login and no token ever lives inside a checkout.
//!
//! A token given with --token or WRITEBOOK_TOKEN wins over all of that: it's
//! used as-is, never refreshed, and nothing is stored. And the browser sign-in
//! only starts when someone is at a terminal to approve it; unattended runs
//! get an error instead of a wait.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api;
use crate::fsutil;

mod sign_in;
pub use sign_in::login_command;

const CLIENT_ID: &str = "writebook-cli";
const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const EXPIRY_SLACK: u64 = 30;

static GIVEN_TOKEN: OnceLock<String> = OnceLock::new();

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default)]
    hosts: BTreeMap<String, Credentials>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
}

/// Use this token for every request instead of the stored sign-in.
pub fn use_token(token: &str) {
    let token = token.trim();

    if !token.is_empty() {
        let _ = GIVEN_TOKEN.set(token.to_string());
    }
}

fn given_token() -> Option<String> {
    GIVEN_TOKEN.get().cloned()
}

/// A valid access token for the host, whatever it takes: the given token,
/// cached when fresh, refreshed when expired, and a fresh device-flow sign-in
/// when all else fails.
pub fn access_token(host: &str) -> Result<String> {
    if let Some(token) = given_token() {
        return Ok(token);
    }

    let file = load()?;

    match file.hosts.get(host) {
        Some(credentials) if !expired(credentials) => Ok(credentials.access_token.clone()),
        Some(credentials) => match refresh(host, &credentials.refresh_token) {
            Ok(refreshed) => Ok(refreshed.access_token),
            Err(_) => Ok(login(host)?.access_token),
        },
        None => Ok(login(host)?.access_token),
    }
}

/// Force-refresh after a 401: the cached token was rejected even though it
/// looked fresh, so rotate the refresh token or fall back to signing in again.
pub fn renewed_access_token(host: &str) -> Result<String> {
    if given_token().is_some() {
        bail!("{host} did not accept the token from --token or WRITEBOOK_TOKEN");
    }

    let file = load()?;

    match file.hosts.get(host) {
        Some(credentials) => match refresh(host, &credentials.refresh_token) {
            Ok(refreshed) => Ok(refreshed.access_token),
            Err(_) => Ok(login(host)?.access_token),
        },
        None => Ok(login(host)?.access_token),
    }
}

/// A token for read-only requests, if there is one without asking anybody:
/// the given token, or the stored sign-in (refreshed if need be). None means
/// read anonymously.
pub fn existing_access_token(host: &str) -> Option<String> {
    if let Some(token) = given_token() {
        return Some(token);
    }

    let credentials = load().ok()?.hosts.get(host).cloned()?;

    if expired(&credentials) {
        refresh(host, &credentials.refresh_token).ok().map(|it| it.access_token)
    } else {
        Some(credentials.access_token)
    }
}

fn login(host: &str) -> Result<Credentials> {
    if !someone_can_approve() {
        bail!(
            "not signed in to {host}, and nobody is at a terminal to approve a browser sign-in. \
             Pass --token or set WRITEBOOK_TOKEN, run this once in a terminal to sign in, \
             or sign in in two steps with `writebook login {host}`"
        );
    }

    sign_in::at_terminal(host)
}

fn refresh(host: &str, refresh_token: &str) -> Result<Credentials> {
    let (status, body) = post_token_request(
        host,
        &[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )?;

    if status != 200 {
        bail!("token refresh failed: {}", error_description(&body));
    }

    let credentials = credentials_from(&body)?;
    store(host, &credentials)?;
    Ok(credentials)
}

fn request_device_authorization(host: &str) -> Result<Value> {
    let mut response = api::agent()
        .post(&format!("{host}/oauth/device_authorizations"))
        .send_form([("client_id", CLIENT_ID)])
        .with_context(|| format!("could not reach {host}"))?;

    if response.status() != 200 {
        bail!("{host} does not support device sign-in — is it running a Writebook with the CLI API?");
    }

    Ok(response.body_mut().read_json()?)
}

fn post_token_request(host: &str, form: &[(&str, &str)]) -> Result<(u16, Value)> {
    let mut response = api::agent()
        .post(&format!("{host}/oauth/tokens"))
        .send_form(form.iter().copied())
        .with_context(|| format!("could not reach {host}"))?;

    let status = response.status().as_u16();
    let body = response.body_mut().read_json().unwrap_or(Value::Null);
    Ok((status, body))
}

fn credentials_from(body: &Value) -> Result<Credentials> {
    Ok(Credentials {
        access_token: string_field(body, "access_token")?,
        refresh_token: string_field(body, "refresh_token")?,
        expires_at: now() + body["expires_in"].as_u64().unwrap_or(3600),
    })
}

fn expired(credentials: &Credentials) -> bool {
    credentials.expires_at <= now() + EXPIRY_SLACK
}

fn store(host: &str, credentials: &Credentials) -> Result<()> {
    let mut file = load()?;
    file.hosts.insert(host.to_string(), credentials.clone());
    fsutil::write_json_privately(&credentials_path(), &file)
}

fn load() -> Result<CredentialsFile> {
    let path = credentials_path();

    if path.exists() {
        let contents = fsutil::read_to_string(&path)?;
        serde_json::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
    } else {
        Ok(CredentialsFile::default())
    }
}

fn credentials_path() -> PathBuf {
    config_dir().join("credentials.json")
}

fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|it| !it.is_empty()) {
        PathBuf::from(dir).join("writebook")
    } else {
        dirs::home_dir().expect("could not determine the home directory").join(".config/writebook")
    }
}

/// The device flow needs a person to read the code and approve it. Without a
/// terminal on both ends (agents, cron, CI) it would only wait and time out.
fn someone_can_approve() -> bool {
    std::env::var_os("CI").is_none() && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn string_field(body: &Value, field: &str) -> Result<String> {
    Ok(body[field]
        .as_str()
        .with_context(|| format!("server response is missing {field}"))?
        .to_string())
}

fn error_description(body: &Value) -> String {
    body["error_description"]
        .as_str()
        .or(body["error"].as_str())
        .unwrap_or("unexpected server response")
        .to_string()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("clock before 1970").as_secs()
}
