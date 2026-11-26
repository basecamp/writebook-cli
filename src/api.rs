//! Thin HTTP layer over the Writebook JSON API: bearer auth with transparent
//! token renewal on 401, JSON verbs, binary downloads, and multipart uploads.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::creds;

const USER_AGENT: &str = concat!("writebook-cli/", env!("CARGO_PKG_VERSION"));
const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

pub fn agent() -> ureq::Agent {
    agent_config()
        .redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never)
        .build()
        .new_agent()
}

fn agent_without_redirects() -> ureq::Agent {
    agent_config().max_redirects(0).build().new_agent()
}

fn agent_config() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        .user_agent(USER_AGENT)
}

fn origin_of(url: &str) -> String {
    let after_scheme = match url.find("://") {
        Some(index) => index + 3,
        None => 0,
    };
    let end = match url[after_scheme..].find(['/', '?', '#']) {
        Some(index) => index + after_scheme,
        None => url.len(),
    };

    url[..end].to_lowercase()
}

fn redirect_target(from: &str, location: &str) -> Result<String> {
    let target = url::Url::parse(from)?
        .join(location)
        .with_context(|| format!("{from} redirected to {location:?}"))?;

    if matches!(target.scheme(), "https" | "http") {
        Ok(target.into())
    } else {
        anyhow::bail!("{from} redirected to {location:?}, which isn't a web address")
    }
}

/// A non-2xx API response. Push inspects the status to tell a stale-write
/// conflict (409) from everything else.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub body: Value,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = self.body["error"].as_str().unwrap_or("");

        if detail.is_empty() {
            write!(f, "the server answered {}", self.status)
        } else {
            write!(f, "the server answered {}: {}", self.status, detail)
        }
    }
}

impl std::error::Error for ApiError {}

/// The site turned the token away.
#[derive(Debug)]
pub struct TokenRejected {
    pub host: String,
}

impl std::fmt::Display for TokenRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} did not accept the token — sign in again, or check WRITEBOOK_TOKEN",
            self.host
        )
    }
}

impl std::error::Error for TokenRejected {}

type Response = ureq::http::Response<ureq::Body>;

enum Request<'a> {
    Get,
    Delete,
    Post(&'a Value),
    Patch(&'a Value),
}

#[derive(Clone, Copy)]
enum MultipartMethod {
    Post,
    Patch,
}

pub struct Part<'a> {
    pub name: &'a str,
    pub filename: Option<&'a str>,
    pub data: PartData<'a>,
}

pub enum PartData<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

/// The token goes only to its own site, including across a download's redirects: it
/// is sent only while they keep the same scheme, host, and port.
pub struct Client {
    host: String,
    token: creds::Token,
    renewable: bool,
}

impl Client {
    pub fn connect(host: &str) -> Result<Client> {
        let token = creds::access_token(host)?;
        Ok(Client {
            host: host.to_string(),
            token,
            renewable: true,
        })
    }

    /// A client for a token someone else handed over, which is used as it is
    /// and not swapped for the stored sign-in.
    pub fn with_token(host: &str, token: creds::Token) -> Client {
        Client {
            host: host.to_string(),
            token,
            renewable: false,
        }
    }

    pub fn get(&mut self, path: &str) -> Result<Value> {
        self.request(path, Request::Get)
    }

    pub fn post(&mut self, path: &str, body: &Value) -> Result<Value> {
        self.request(path, Request::Post(body))
    }

    pub fn patch(&mut self, path: &str, body: &Value) -> Result<Value> {
        self.request(path, Request::Patch(body))
    }

    pub fn delete(&mut self, path: &str) -> Result<Value> {
        self.request(path, Request::Delete)
    }

    pub fn download(&mut self, path: &str) -> Result<Vec<u8>> {
        self.download_at_most(path, DOWNLOAD_LIMIT)
    }

    /// Download a file, failing when it's bigger than `limit` bytes.
    pub fn download_at_most(&mut self, path: &str, limit: u64) -> Result<Vec<u8>> {
        let mut response = self.send_renewing(|client| client.send_download(path))?;
        let status = response.status().as_u16();

        if (200..300).contains(&status) {
            response
                .body_mut()
                .with_config()
                .limit(limit)
                .read_to_vec()
                .with_context(|| format!("could not download {path}"))
        } else {
            Err(ApiError {
                status,
                body: Value::Null,
            }
            .into())
        }
    }

    pub fn post_multipart(&mut self, path: &str, parts: &[Part]) -> Result<Value> {
        self.multipart(path, MultipartMethod::Post, parts)
    }

    pub fn patch_multipart(&mut self, path: &str, parts: &[Part]) -> Result<Value> {
        self.multipart(path, MultipartMethod::Patch, parts)
    }

    fn request(&mut self, path: &str, request: Request) -> Result<Value> {
        response_json(self.send_renewing(|client| client.send(path, &request))?)
    }

    fn multipart(&mut self, path: &str, method: MultipartMethod, parts: &[Part]) -> Result<Value> {
        let boundary = multipart_boundary();
        let body = multipart_body(&boundary, parts);
        let content_type = format!("multipart/form-data; boundary={boundary}");

        response_json(self.send_renewing(|client| client.send_multipart(path, method, &content_type, &body))?)
    }

    fn send_renewing(&mut self, send: impl Fn(&Client) -> Result<Response>) -> Result<Response> {
        let response = send(self)?;

        if response.status().as_u16() == 401 {
            self.renew_token()?;
            send(self)
        } else {
            Ok(response)
        }
    }

    fn send(&self, path: &str, request: &Request) -> Result<Response> {
        if !matches!(request, Request::Get) {
            self.ensure_can_write()?;
        }

        let url = url_on(&self.host, path)?;
        let authorization = self.authorization();

        let response = match request {
            Request::Get => agent()
                .get(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .call(),
            Request::Delete => agent()
                .delete(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .call(),
            Request::Post(json) => agent()
                .post(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .send_json(json),
            Request::Patch(json) => agent()
                .patch(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .send_json(json),
        };

        response.with_context(|| format!("could not reach {}", self.host))
    }

    fn send_download(&self, path: &str) -> Result<Response> {
        let agent = agent_without_redirects();
        let mut url = url_on(&self.host, path)?;

        for _ in 0..MAX_REDIRECTS {
            let mut request = agent.get(&url);

            if origin_of(&url) == self.host {
                request = request.header("Authorization", &self.authorization());
            }

            let response = request
                .call()
                .with_context(|| format!("could not reach {}", self.host))?;

            match response.headers().get("location").and_then(|it| it.to_str().ok()) {
                Some(location) if response.status().is_redirection() => url = redirect_target(&url, location)?,
                _ => return Ok(response),
            }
        }

        anyhow::bail!("{} redirected {path} too many times", self.host)
    }

    fn send_multipart(&self, path: &str, method: MultipartMethod, content_type: &str, body: &[u8]) -> Result<Response> {
        self.ensure_can_write()?;
        let url = url_on(&self.host, path)?;
        let authorization = self.authorization();

        let response = match method {
            MultipartMethod::Post => agent()
                .post(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .header("Content-Type", content_type)
                .send(body),
            MultipartMethod::Patch => agent()
                .patch(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .header("Content-Type", content_type)
                .send(body),
        };

        response.with_context(|| format!("could not reach {}", self.host))
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.token.value)
    }

    fn ensure_can_write(&self) -> Result<()> {
        if self.token.access == creds::Access::ReadOnly {
            bail!(
                "the sign-in to {} is read-only, which can't change books, so nothing more was changed",
                self.host
            );
        }

        Ok(())
    }

    fn renew_token(&mut self) -> Result<()> {
        if self.renewable {
            self.token = creds::renewed_access_token(&self.host)?;
            Ok(())
        } else {
            Err(TokenRejected {
                host: self.host.clone(),
            }
            .into())
        }
    }
}

/// A path on the host as a full URL. Paths come from the server too (image
/// URLs in a manifest), and the token goes wherever they point, so only a
/// path on this host is taken, not a full URL or a `//other.host` one.
pub fn url_on(host: &str, path: &str) -> Result<String> {
    if path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') {
        Ok(format!("{host}{path}"))
    } else {
        anyhow::bail!("refusing to request {path:?}: only paths on {host} are requested")
    }
}

fn response_json(mut response: Response) -> Result<Value> {
    let status = response.status().as_u16();
    let body = match response.body_mut().read_to_string() {
        Ok(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };

    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(ApiError { status, body }.into())
    }
}

fn multipart_body(boundary: &str, parts: &[Part]) -> Vec<u8> {
    let mut body = Vec::new();

    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());

        match (&part.data, part.filename) {
            (PartData::Bytes(bytes), filename) => {
                let filename = filename.unwrap_or("file");
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
                         Content-Type: application/octet-stream\r\n\r\n",
                        part.name, filename
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(bytes);
            }
            (PartData::Text(text), _) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", part.name).as_bytes(),
                );
                body.extend_from_slice(text.as_bytes());
            }
        }

        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn multipart_boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before 1970")
        .as_nanos();
    format!("writebook-{nanos}-{}", std::process::id())
}
