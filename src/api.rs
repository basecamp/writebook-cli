//! Thin HTTP layer over the Writebook JSON API: bearer auth with transparent
//! token renewal on 401, JSON verbs, binary downloads, and multipart uploads.

use anyhow::{Context, Result};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::creds;

const USER_AGENT: &str = concat!("writebook-cli/", env!("CARGO_PKG_VERSION"));
const DOWNLOAD_LIMIT: u64 = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        .redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never)
        .user_agent(USER_AGENT)
        .build()
        .new_agent()
}

fn agent_without_redirects() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(std::time::Duration::from_secs(10)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(60)))
        .user_agent(USER_AGENT)
        .build()
        .new_agent()
}

/// The scheme, host, and port of a URL, like https://books.example.com.
fn origin_of(url: &str) -> String {
    let after_scheme = url.find("://").map(|it| it + 3).unwrap_or_default();
    let end = url[after_scheme..].find(['/', '?', '#']).map(|it| it + after_scheme).unwrap_or(url.len());

    url[..end].to_lowercase()
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

pub struct Part<'a> {
    pub name: &'a str,
    pub filename: Option<&'a str>,
    pub data: PartData<'a>,
}

pub enum PartData<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

pub struct Client {
    host: String,
    token: String,
}

impl Client {
    pub fn connect(host: &str) -> Result<Client> {
        let token = creds::access_token(host)?;
        Ok(Client { host: host.to_string(), token })
    }

    pub fn get(&mut self, path: &str) -> Result<Value> {
        self.request("GET", path, None)
    }

    pub fn post(&mut self, path: &str, body: &Value) -> Result<Value> {
        self.request("POST", path, Some(body))
    }

    pub fn patch(&mut self, path: &str, body: &Value) -> Result<Value> {
        self.request("PATCH", path, Some(body))
    }

    pub fn delete(&mut self, path: &str) -> Result<Value> {
        self.request("DELETE", path, None)
    }

    pub fn download(&mut self, path: &str) -> Result<Vec<u8>> {
        let mut response = self.send_download(path)?;

        if response.status().as_u16() == 401 {
            self.renew_token()?;
            response = self.send_download(path)?;
        }

        let status = response.status().as_u16();

        if !(200..300).contains(&status) {
            return Err(ApiError { status, body: Value::Null }.into());
        }

        response
            .body_mut()
            .with_config()
            .limit(DOWNLOAD_LIMIT)
            .read_to_vec()
            .with_context(|| format!("could not download {path}"))
    }

    pub fn post_multipart(&mut self, path: &str, parts: &[Part]) -> Result<Value> {
        let boundary = boundary();
        let body = multipart_body(&boundary, parts);
        let content_type = format!("multipart/form-data; boundary={boundary}");

        let mut response = self.multipart_request(path, &content_type, &body)?;

        if response.status().as_u16() == 401 {
            self.renew_token()?;
            response = self.multipart_request(path, &content_type, &body)?;
        }

        judge(response)
    }

    pub fn patch_multipart(&mut self, path: &str, parts: &[Part]) -> Result<Value> {
        let boundary = boundary();
        let body = multipart_body(&boundary, parts);
        let content_type = format!("multipart/form-data; boundary={boundary}");

        let mut response = self.multipart_request_with("PATCH", path, &content_type, &body)?;

        if response.status().as_u16() == 401 {
            self.renew_token()?;
            response = self.multipart_request_with("PATCH", path, &content_type, &body)?;
        }

        judge(response)
    }

    fn request(&mut self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut response = self.send(method, path, body)?;

        if response.status().as_u16() == 401 {
            self.renew_token()?;
            response = self.send(method, path, body)?;
        }

        judge(response)
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<ureq::http::Response<ureq::Body>> {
        let url = self.url(path)?;
        let authorization = format!("Bearer {}", self.token);

        let response = match (method, body) {
            ("GET", None) => agent()
                .get(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .call(),
            ("DELETE", None) => agent()
                .delete(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .call(),
            ("POST", Some(json)) => agent()
                .post(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .send_json(json),
            ("PATCH", Some(json)) => agent()
                .patch(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .send_json(json),
            _ => unreachable!("unsupported request shape: {method} with body: {}", body.is_some()),
        };

        response.with_context(|| format!("could not reach {}", self.host))
    }

    /// Writebook answers a download with a redirect or two, to the stored
    /// file. The token goes along only while they stay on this site (same
    /// scheme, host, and port); a file served from elsewhere, like S3, gets
    /// a signed URL that needs no token.
    fn send_download(&self, path: &str) -> Result<ureq::http::Response<ureq::Body>> {
        let agent = agent_without_redirects();
        let mut url = self.url(path)?;

        for _ in 0..MAX_REDIRECTS {
            let mut request = agent.get(&url);

            if origin_of(&url) == self.host {
                request = request.header("Authorization", &format!("Bearer {}", self.token));
            }

            let response = request.call().with_context(|| format!("could not reach {}", self.host))?;

            match response.headers().get("location").and_then(|it| it.to_str().ok()) {
                Some(location) if response.status().is_redirection() => url = self.redirect_target(location)?,
                _ => return Ok(response),
            }
        }

        anyhow::bail!("{} redirected {path} too many times", self.host)
    }

    fn redirect_target(&self, location: &str) -> Result<String> {
        if location.starts_with("https://") || location.starts_with("http://") {
            Ok(location.to_string())
        } else {
            self.url(location)
        }
    }

    fn multipart_request(&self, path: &str, content_type: &str, body: &[u8]) -> Result<ureq::http::Response<ureq::Body>> {
        self.multipart_request_with("POST", path, content_type, body)
    }

    fn multipart_request_with(&self, method: &str, path: &str, content_type: &str, body: &[u8]) -> Result<ureq::http::Response<ureq::Body>> {
        let url = self.url(path)?;
        let authorization = format!("Bearer {}", self.token);

        let response = match method {
            "POST" => agent()
                .post(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .header("Content-Type", content_type)
                .send(body),
            "PATCH" => agent()
                .patch(&url)
                .header("Authorization", &authorization)
                .header("Accept", "application/json")
                .header("Content-Type", content_type)
                .send(body),
            _ => unreachable!("unsupported multipart method: {method}"),
        };

        response.with_context(|| format!("could not reach {}", self.host))
    }

    fn renew_token(&mut self) -> Result<()> {
        self.token = creds::renewed_access_token(&self.host)?;
        Ok(())
    }

    fn url(&self, path: &str) -> Result<String> {
        url_on(&self.host, path)
    }
}

/// A path on the host as a full URL. Paths come from the server too (image
/// URLs in a manifest), and the token goes wherever they point, so only a
/// path on this host is taken: never a full URL or a `//other.host` one.
pub fn url_on(host: &str, path: &str) -> Result<String> {
    if path.starts_with('/') && !path.starts_with("//") && !path.contains('\\') {
        Ok(format!("{host}{path}"))
    } else {
        anyhow::bail!("refusing to request {path:?}: only paths on {host} are requested")
    }
}

fn judge(mut response: ureq::http::Response<ureq::Body>) -> Result<Value> {
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);

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

fn boundary() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock before 1970").as_nanos();
    format!("writebook-{nanos}-{}", std::process::id())
}
