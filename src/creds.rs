//! OAuth credentials per site and profile, obtained through the RFC 8628
//! device flow and refreshed transparently. Kept in the system keyring (see
//! storage.rs) so several checkouts share one login and no token ever lives
//! inside a checkout. Which profile a site is signed in as comes from the
//! inventory (see inventory.rs).
//!
//! A token given in WRITEBOOK_TOKEN wins over all of that: it's
//! used as-is, never refreshed, and nothing is stored. It belongs to one site,
//! named in WRITEBOOK_TOKEN_HOST, and is never sent anywhere
//! else, nor over plain http except to this machine. And the browser sign-in
//! only starts when someone is at a terminal to approve it; unattended runs
//! get an error instead of a wait.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api;
use crate::fsutil;
use crate::inventory;

mod sign_in;
mod storage;
pub use sign_in::sign_in;

const CLIENT_ID: &str = "writebook-cli";
const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const EXPIRY_SLACK: u64 = 30;

static GIVEN_TOKEN: OnceLock<GivenToken> = OnceLock::new();

struct GivenToken {
    token: String,
    host: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    #[serde(default)]
    pub access: Access,
}

/// What a sign-in may do. Writebook calls a read-only one scope "read".
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    #[default]
    Full,
    ReadOnly,
}

/// A token to send, and what the sign-in it came from may do.
pub struct Token {
    pub value: String,
    pub access: Access,
}

impl Token {
    /// WRITEBOOK_TOKEN can only be told apart by its prefix: API tokens only
    /// read, and the rest are what a sign-in handed out.
    pub fn given(value: String) -> Token {
        let access = if value.starts_with("wb_api_") { Access::ReadOnly } else { Access::Full };
        Token { value, access }
    }
}

impl From<Credentials> for Token {
    fn from(credentials: Credentials) -> Token {
        Token { value: credentials.access_token, access: credentials.access }
    }
}

impl Access {
    pub fn from_flag(read_only: bool) -> Access {
        if read_only {
            Access::ReadOnly
        } else {
            Access::Full
        }
    }
}

/// Use this token, for the given site, instead of the stored sign-in.
pub fn use_token(token: &str, host: Option<&str>) {
    let token = token.trim();
    let host = host.map(normalize_host).filter(|it| !it.is_empty());

    if !token.is_empty() {
        let _ = GIVEN_TOKEN.set(GivenToken { token: token.to_string(), host });
    }
}

/// What the given token means for this host: Ok(None) when there is no given
/// token, Ok(Some) when it may go there, and an error when it may not.
fn given_token(host: &str) -> Result<Option<String>> {
    let Some(given) = GIVEN_TOKEN.get() else {
        return Ok(None);
    };

    let Some(token_host) = &given.host else {
        bail!("say which site the token is for, so it is never sent anywhere else: WRITEBOOK_TOKEN_HOST={}", normalize_host(host));
    };

    if *token_host != normalize_host(host) {
        bail!("the token is for {token_host}, so it is not sent to {host}");
    }

    if token_host.starts_with("http://") && !is_local(token_host) {
        bail!("the token is only sent over https (or plain http to this machine), not to {token_host}");
    }

    Ok(Some(given.token.clone()))
}

/// Sign-ins only travel over https, or plain http to this machine.
pub fn ensure_private_connection(host: &str) -> Result<()> {
    if host.starts_with("http://") && !is_local(host) {
        bail!("{host} isn't https, so signing in to it would send your sign-in in the clear. Use its https:// address");
    }

    Ok(())
}

pub fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('/').to_lowercase()
}

fn is_local(host: &str) -> bool {
    let name = host.trim_start_matches("http://").split(':').next().unwrap_or_default();
    matches!(name, "localhost" | "127.0.0.1") || host.trim_start_matches("http://").starts_with("[::1]")
}

/// A valid access token for the host, whatever it takes: the given token,
/// cached when fresh, refreshed when expired, and a fresh device-flow sign-in
/// when all else fails.
pub fn access_token(host: &str) -> Result<Token> {
    if let Some(token) = given_token(host)? {
        return Ok(Token::given(token));
    }

    ensure_private_connection(host)?;
    let profile = inventory::profile_for(host)?;

    match storage::load(host, &profile)? {
        Some(credentials) if !expired(&credentials) => Ok(credentials.into()),
        Some(credentials) => match refresh(host, &profile, &credentials) {
            Ok(refreshed) => Ok(refreshed.into()),
            Err(_) => Ok(login(host, &profile, credentials.access)?.into()),
        },
        None => Ok(login(host, &profile, Access::Full)?.into()),
    }
}

/// Force-refresh after a 401: the cached token was rejected even though it
/// looked fresh, so rotate the refresh token or fall back to signing in again.
pub fn renewed_access_token(host: &str) -> Result<Token> {
    if given_token(host)?.is_some() {
        bail!("{host} did not accept the token from WRITEBOOK_TOKEN");
    }

    ensure_private_connection(host)?;
    let profile = inventory::profile_for(host)?;

    match storage::load(host, &profile)? {
        Some(credentials) => match refresh(host, &profile, &credentials) {
            Ok(refreshed) => Ok(refreshed.into()),
            Err(_) => Ok(login(host, &profile, credentials.access)?.into()),
        },
        None => Ok(login(host, &profile, Access::Full)?.into()),
    }
}

/// A token for read-only requests, if there is one without asking anybody:
/// the given token, or the stored sign-in (refreshed if need be). None means
/// read anonymously.
pub fn existing_access_token(host: &str) -> Result<Option<String>> {
    if let Some(token) = given_token(host)? {
        return Ok(Some(token));
    }

    let profile = inventory::profile_for(host)?;

    let Some(credentials) = storage::load(host, &profile)? else {
        return Ok(None);
    };

    ensure_private_connection(host)?;

    if expired(&credentials) {
        match refresh(host, &profile, &credentials) {
            Ok(refreshed) => Ok(Some(refreshed.access_token)),
            Err(error) => {
                eprintln!("note: could not renew your sign-in to {host} ({error:#}); reading without it");
                Ok(None)
            }
        }
    } else {
        Ok(Some(credentials.access_token))
    }
}

/// What the stored sign-in to this site as this profile may do, if there is
/// one. It may have expired; it's renewed when used.
pub fn stored_access(host: &str, profile: &str) -> Result<Option<Access>> {
    Ok(storage::load(host, profile)?.map(|it| it.access))
}

/// Whether the token the CLI would send to this site may change books.
pub fn can_write(host: &str) -> Result<bool> {
    match given_token(host)? {
        Some(token) => Ok(Token::given(token).access == Access::Full),
        None => Ok(stored_access(host, &inventory::profile_for(host)?)? != Some(Access::ReadOnly)),
    }
}

/// Fail before changing anything when the sign-in can only read.
pub fn ensure_can_write(host: &str) -> Result<()> {
    match given_token(host)? {
        Some(token) if token.starts_with("wb_api_") => {
            bail!("WRITEBOOK_TOKEN holds a read-only API token, which can't change books. Sign in to {host} to do that")
        }
        Some(_) => Ok(()),
        None => {
            let profile = inventory::profile_for(host)?;

            if stored_access(host, &profile)? == Some(Access::ReadOnly) {
                bail!(
                    "you're signed in to {host} as {profile} read-only, which can't change books. \
                     Sign in as a profile that can, like `writebook profile add editor`, and make it the default"
                );
            }

            Ok(())
        }
    }
}

/// Sign out of a site as this profile, and tell the site to stop honoring
/// the sign-in, so a copy of it left anywhere is no use.
pub fn forget(host: &str, profile: &str) -> Result<()> {
    storage::locked(|| {
        if let Some(credentials) = storage::load(host, profile)? {
            if let Err(error) = revoke(host, &credentials) {
                eprintln!("warning: could not tell {host} to revoke the sign-in ({error:#}); it was removed here anyway");
            }
        }

        sign_in::forget_pending(host, profile)?;
        storage::delete(host, profile)
    })
}

/// Drop the sign-ins to a site that were started but never finished, for
/// any profile.
pub fn forget_unfinished_sign_ins(host: &str) -> Result<()> {
    sign_in::forget_all_pending(host)
}

fn revoke(host: &str, credentials: &Credentials) -> Result<()> {
    ensure_private_connection(host)?;

    let response = api::agent()
        .post(&format!("{host}/oauth/revocations"))
        .send_form([("client_id", CLIENT_ID), ("token", credentials.refresh_token.as_str())])
        .with_context(|| format!("could not reach {host}"))?;

    if response.status() == 200 {
        Ok(())
    } else {
        bail!("{host} answered {}", response.status())
    }
}

/// Sign in again with the same access the last sign-in had.
fn login(host: &str, profile: &str, access: Access) -> Result<Credentials> {
    if !someone_can_approve() {
        bail!(
            "not signed in to {host}, and nobody is at a terminal to approve a browser sign-in. \
             Set WRITEBOOK_TOKEN and WRITEBOOK_TOKEN_HOST, run this once in a terminal to sign in, \
             or sign in in two steps with `{}`",
            sign_in_command(host, profile, access)
        );
    }

    sign_in::at_terminal(host, profile, access)
}

fn sign_in_command(host: &str, profile: &str, access: Access) -> String {
    match access {
        Access::Full => inventory::sign_in_command(host, profile),
        Access::ReadOnly => format!("{} --read-only", inventory::sign_in_command(host, profile)),
    }
}

/// Trade the refresh token for new credentials. Each refresh token works
/// once, so two commands refreshing at the same time would sign each other
/// out: the lock lets one go first, and the other then finds its work done.
fn refresh(host: &str, profile: &str, stale: &Credentials) -> Result<Credentials> {
    storage::locked(|| {
        match storage::load(host, profile)? {
            Some(current) if current.refresh_token != stale.refresh_token && !expired(&current) => return Ok(current),
            Some(_) => {}
            None => bail!("signed out of {host} in the meantime"),
        }

        let (status, body) = post_token_request(
            host,
            &[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", &stale.refresh_token),
            ],
        )?;

        if status != 200 {
            bail!("token refresh failed: {}", error_description(&body));
        }

        let credentials = renewed(host, stale, credentials_from(&body)?, &body)?;
        storage::save(host, profile, &credentials)?;
        Ok(credentials)
    })
}

/// A renewal keeps the access the sign-in had. One that doesn't say keeps
/// it as it was; one that claims more is revoked rather than kept.
fn renewed(host: &str, stale: &Credentials, mut credentials: Credentials, body: &Value) -> Result<Credentials> {
    if body["scope"].is_null() {
        credentials.access = stale.access;
    } else if stale.access == Access::ReadOnly && credentials.access != Access::ReadOnly {
        match revoke(host, &credentials) {
            Ok(()) => bail!("{host} renewed a read-only sign-in with full access, so it was revoked. Sign in again"),
            Err(error) => bail!(
                "{host} renewed a read-only sign-in with full access, so it wasn't kept, \
                 but telling {host} to revoke it failed ({error:#}). Sign in again"
            ),
        }
    }

    Ok(credentials)
}

/// Writebooks from before read-only sign-ins leave the scope out, and give
/// full access.
fn access_from(body: &Value) -> Access {
    if body["scope"].as_str() == Some("read") {
        Access::ReadOnly
    } else {
        Access::Full
    }
}

fn request_device_authorization(host: &str, access: Access) -> Result<Value> {
    let scope = match access {
        Access::Full => "read write",
        Access::ReadOnly => "read",
    };

    let mut response = api::agent()
        .post(&format!("{host}/oauth/device_authorizations"))
        .send_form([("client_id", CLIENT_ID), ("scope", scope)])
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
        access: access_from(body),
    })
}

fn expired(credentials: &Credentials) -> bool {
    credentials.expires_at <= now() + EXPIRY_SLACK
}

/// Where sign-ins and the instance list are kept, created readable by you
/// alone if it isn't there yet.
pub fn config_dir() -> Result<PathBuf> {
    let dir = if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|it| !it.is_empty()) {
        PathBuf::from(dir).join("writebook")
    } else {
        dirs::home_dir().context("could not determine the home directory")?.join(".config/writebook")
    };

    fsutil::create_private_dir(&dir)?;
    Ok(dir)
}

/// The device flow needs a person to read the code and approve it. Without a
/// terminal on both ends (cron, CI), or under an agent (which may well have a
/// terminal), it would only wait and time out.
fn someone_can_approve() -> bool {
    let unattended = ["CI", "WRITEBOOK_NO_SIGN_IN", "CLAUDECODE", "CODEX_SANDBOX"].iter().any(|name| std::env::var_os(name).is_some());

    !unattended && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
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
