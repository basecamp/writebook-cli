//! Plain web requests for the read-only commands: no sign-in flow, no
//! redirects followed (a redirect to the sign-in page means "private"), and
//! the small bits of HTML scraping that stand in for the book list and search
//! on Writebooks without their JSON endpoints.

mod html;
mod target;

use anyhow::{Context, Result};

pub use html::{SearchHit, csrf_token, parse_library, parse_search_results, unescape};
pub use target::{Target, parse_target};

const USER_AGENT: &str = concat!("writebook-cli/", env!("CARGO_PKG_VERSION"));
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

pub struct Web {
    pub host: String,
    token: Option<String>,
    agent: ureq::Agent,
}

pub struct Page {
    pub status: u16,
    pub location: Option<String>,
    pub cookies: Vec<String>,
    pub body: String,
}

impl Page {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Writebook answers a page you may not see with a redirect to its
    /// sign-in page (or, on a brand-new install, to the first-run page).
    pub fn wants_sign_in(&self) -> bool {
        (300..400).contains(&self.status)
            && self
                .location
                .as_deref()
                .is_some_and(|location| location.contains("/session/new") || location.contains("/first_run"))
    }
}

impl Web {
    pub fn new(host: &str, token: Option<String>) -> Web {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(std::time::Duration::from_secs(10)))
            .timeout_global(Some(std::time::Duration::from_secs(120)))
            .user_agent(USER_AGENT)
            .build()
            .new_agent();

        Web {
            host: host.to_string(),
            token,
            agent,
        }
    }

    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }

    pub fn get(&self, path: &str, accept: &str) -> Result<Page> {
        self.get_query(path, accept, &[])
    }

    /// GET with query parameters, encoded for the URL.
    pub fn get_query(&self, path: &str, accept: &str, query: &[(&str, &str)]) -> Result<Page> {
        let mut request = self
            .agent
            .get(&crate::api::url_on(&self.host, path)?)
            .header("Accept", accept)
            .query_pairs(query.iter().copied());

        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }

        into_page(request.call(), &self.host)
    }

    pub fn post_form(&self, path: &str, form: &[(&str, &str)], cookie: Option<&str>) -> Result<Page> {
        let mut request = self
            .agent
            .post(&crate::api::url_on(&self.host, path)?)
            .header("Accept", "text/html");

        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }

        if let Some(cookie) = cookie.filter(|it| !it.is_empty()) {
            request = request.header("Cookie", cookie);
        }

        into_page(request.send_form(form.iter().copied()), &self.host)
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.host, path)
    }
}

fn into_page(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>, host: &str) -> Result<Page> {
    let mut response = response.with_context(|| format!("could not reach {host}"))?;
    let headers = response.headers();

    let location = headers
        .get("location")
        .and_then(|it| it.to_str().ok())
        .map(str::to_string);
    let cookies = headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|it| it.to_str().ok())
        .filter_map(|it| it.split(';').next())
        .map(|it| it.trim().to_string())
        .collect();

    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_string()
        .with_context(|| format!("could not read the answer from {host}"))?;

    Ok(Page {
        status,
        location,
        cookies,
        body,
    })
}
