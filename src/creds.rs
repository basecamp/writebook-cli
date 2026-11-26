//! OAuth credentials per site and profile, obtained through the RFC 8628
//! device flow and refreshed transparently. Kept in the system keyring (see
//! storage.rs) so several checkouts share one login and no token ever lives
//! inside a checkout. Which profile a site is signed in as comes from the
//! inventory (see inventory.rs).
//!
//! A token given in WRITEBOOK_TOKEN wins over all of that: it's
//! used as-is, not refreshed, and nothing is stored. It belongs to one site,
//! named in WRITEBOOK_TOKEN_HOST, and is sent nowhere
//! else, nor over plain http except to this machine. And the browser sign-in
//! only starts when someone is at a terminal to approve it; unattended runs
//! get an error instead of a wait.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api;
use crate::inventory;

mod sign_in;
mod storage;
pub use sign_in::{forget_unfinished_sign_ins, sign_in};

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

impl Access {
    pub fn from_flag(read_only: bool) -> Access {
        if read_only { Access::ReadOnly } else { Access::Full }
    }
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
        let access = if value.starts_with("wb_api_") {
            Access::ReadOnly
        } else {
            Access::Full
        };
        Token { value, access }
    }
}

impl From<Credentials> for Token {
    fn from(credentials: Credentials) -> Token {
        Token {
            value: credentials.access_token,
            access: credentials.access,
        }
    }
}

/// Use this token, for the given site, instead of the stored sign-in. The
/// token is sent only to that site, and only over https (or plain http to this
/// machine); a request for any other site is refused.
pub fn use_token(token: &str, host: Option<&str>) {
    let token = token.trim();
    let host = host.map(normalize_host).filter(|it| !it.is_empty());

    if !token.is_empty() {
        let given = GivenToken {
            token: token.to_string(),
            host,
        };

        match GIVEN_TOKEN.set(given) {
            Ok(()) => {}
            Err(_already_set) => {}
        }
    }
}

fn given_token(host: &str) -> Result<Option<Token>> {
    let Some(given) = GIVEN_TOKEN.get() else {
        return Ok(None);
    };

    let token_host = match &given.host {
        Some(token_host) => token_host,
        None => bail!(
            "say which site the token is for, so it is never sent anywhere else: WRITEBOOK_TOKEN_HOST={}",
            normalize_host(host)
        ),
    };

    if *token_host != normalize_host(host) {
        bail!("the token is for {token_host}, so it is not sent to {host}");
    }

    if token_host.starts_with("http://") && !is_local(token_host) {
        bail!("the token is only sent over https (or plain http to this machine), not to {token_host}");
    }

    Ok(Some(Token::given(given.token.clone())))
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
        return Ok(token);
    }

    ensure_private_connection(host)?;
    let profile = inventory::profile_for(host)?;

    match storage::load(host, &profile)? {
        Some(credentials) if !expired(&credentials) => Ok(credentials.into()),
        Some(credentials) => refresh_or_sign_in_again(host, &profile, &credentials),
        None => Ok(sign_in_at_terminal(host, &profile, Access::Full)?.into()),
    }
}

/// Force-refresh after a 401: the cached token was rejected even though it
/// looked fresh, so rotate the refresh token or fall back to signing in again.
/// A renewal keeps the sign-in's access: one that comes back with more is
/// revoked.
pub fn renewed_access_token(host: &str) -> Result<Token> {
    if given_token(host)?.is_some() {
        bail!("{host} did not accept the token from WRITEBOOK_TOKEN");
    }

    ensure_private_connection(host)?;
    let profile = inventory::profile_for(host)?;

    match storage::load(host, &profile)? {
        Some(credentials) => refresh_or_sign_in_again(host, &profile, &credentials),
        None => Ok(sign_in_at_terminal(host, &profile, Access::Full)?.into()),
    }
}

fn refresh_or_sign_in_again(host: &str, profile: &str, credentials: &Credentials) -> Result<Token> {
    match refresh(host, profile, credentials) {
        Ok(refreshed) => Ok(refreshed.into()),
        Err(_) => Ok(sign_in_at_terminal(host, profile, credentials.access)?.into()),
    }
}

/// A token for read-only requests, if there is one without asking anybody:
/// the given token, or the stored sign-in (refreshed if need be). None means
/// read anonymously, which is also what happens when renewing fails.
pub fn existing_access_token(host: &str) -> Result<Option<String>> {
    match signed_in_token(host) {
        Ok(token) => Ok(token.map(|it| it.value)),
        Err(error) if error.is::<RenewalFailed>() => {
            eprintln!("note: {error:#}; reading without it");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The token for this site that needs nobody to approve anything: the given
/// token, or the stored sign-in, renewed if it expired. None when there is no
/// sign-in. A renewal that fails is an error that says why, so a caller that
/// can't print notes, like the TUI or MCP, can tell a dead sign-in from none.
pub fn signed_in_token(host: &str) -> Result<Option<Token>> {
    if let Some(token) = given_token(host)? {
        return Ok(Some(token));
    }

    let profile = inventory::profile_for(host)?;

    match storage::load(host, &profile)? {
        None => Ok(None),
        Some(credentials) => {
            ensure_private_connection(host)?;

            if expired(&credentials) {
                match refresh(host, &profile, &credentials) {
                    Ok(refreshed) => Ok(Some(refreshed.into())),
                    Err(cause) => Err(RenewalFailed {
                        host: host.to_string(),
                        cause,
                    }
                    .into()),
                }
            } else {
                Ok(Some(credentials.into()))
            }
        }
    }
}

#[derive(Debug)]
pub struct RenewalFailed {
    host: String,
    cause: anyhow::Error,
}

impl std::fmt::Display for RenewalFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not renew your sign-in to {} ({:#})", self.host, self.cause)
    }
}

impl std::error::Error for RenewalFailed {}

/// What the stored sign-in to this site as this profile may do, if there is
/// one. It may have expired; it's renewed when used.
pub fn stored_access(host: &str, profile: &str) -> Result<Option<Access>> {
    Ok(storage::load(host, profile)?.map(|it| it.access))
}

/// Whether the CLI has a token for this site that may change books. No
/// sign-in can't, and neither can a WRITEBOOK_TOKEN meant for another site,
/// which is not sent here.
pub fn can_write(host: &str) -> Result<bool> {
    match given_token(host) {
        Ok(Some(token)) => Ok(token.access == Access::Full),
        Ok(None) => Ok(stored_access(host, &inventory::profile_for(host)?)? == Some(Access::Full)),
        Err(_) => Ok(false),
    }
}

/// Fail before changing anything when the sign-in can only read.
pub fn ensure_can_write(host: &str) -> Result<()> {
    match given_token(host)? {
        Some(token) if token.access == Access::ReadOnly => {
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
                eprintln!(
                    "warning: could not tell {host} to revoke the sign-in ({error:#}); it was removed here anyway"
                );
            }
        }

        sign_in::forget_pending(host, profile)?;
        storage::delete(host, profile)
    })
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

fn sign_in_at_terminal(host: &str, profile: &str, access: Access) -> Result<Credentials> {
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

fn someone_can_approve() -> bool {
    let unattended = ["CI", "WRITEBOOK_NO_SIGN_IN", "CLAUDECODE", "CODEX_SANDBOX"]
        .iter()
        .any(|name| std::env::var_os(name).is_some());

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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970")
        .as_secs()
}
