//! Plain web requests for the read-only commands: no sign-in flow, no
//! redirects followed (a redirect to the sign-in page means "private"), and
//! the small bits of HTML scraping that stand in for API endpoints Writebook
//! doesn't have yet (the book list and search).

use anyhow::{Context, Result, bail};

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
        let agent =
            ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).user_agent(USER_AGENT).build().new_agent();

        Web { host: host.to_string(), token, agent }
    }

    pub fn signed_in(&self) -> bool {
        self.token.is_some()
    }

    pub fn get(&self, path: &str, accept: &str) -> Result<Page> {
        let mut request = self.agent.get(&self.url(path)).header("Accept", accept);

        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }

        read(request.call(), &self.host)
    }

    pub fn post_form(&self, path: &str, form: &[(&str, &str)], cookie: Option<&str>) -> Result<Page> {
        let mut request = self.agent.post(&self.url(path)).header("Accept", "text/html");

        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }

        if let Some(cookie) = cookie.filter(|it| !it.is_empty()) {
            request = request.header("Cookie", cookie);
        }

        read(request.send_form(form.iter().copied()), &self.host)
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", self.host, path)
        }
    }
}

fn read(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>, host: &str) -> Result<Page> {
    let mut response = response.with_context(|| format!("could not reach {host}"))?;
    let headers = response.headers();

    let location = headers.get("location").and_then(|it| it.to_str().ok()).map(str::to_string);
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

    Ok(Page { status, location, cookies, body })
}

/// What a URL points at: a whole book, or one leaf (page, section, picture)
/// in it. Accepts reading links (/4/handbook/12/on-call), editing links
/// (/books/4/pages/12/edit), `.md` links, and ignores ?query and #anchor.
#[derive(Debug, PartialEq)]
pub struct Target {
    pub host: String,
    pub book_id: Option<u64>,
    pub leaf_id: Option<u64>,
    pub kind: Option<String>,
    /// Reading path without `.md`, e.g. /4/handbook/12/on-call
    pub path: String,
}

pub fn parse_target(url: &str) -> Result<Target> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else {
        bail!("'{url}' is not an http(s) URL");
    };

    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = format!("{scheme}://{host}");

    let path = path.split(['#', '?']).next().unwrap_or_default();
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".md").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|it| !it.is_empty()).collect();

    let number = |segment: Option<&&str>| segment.and_then(|it| it.parse::<u64>().ok());

    match segments.as_slice() {
        [] => Ok(Target { host, book_id: None, leaf_id: None, kind: None, path: String::new() }),
        ["books", book, rest @ ..] => {
            let book_id = number(Some(book)).with_context(|| format!("'{url}' has no book id"))?;

            match rest {
                [kind @ ("pages" | "sections" | "pictures"), leaf, ..] => {
                    let leaf_id = number(Some(leaf)).with_context(|| format!("'{url}' has no page id"))?;
                    let kind = kind.trim_end_matches('s');
                    let kind = format!("{}{}", kind[..1].to_uppercase(), &kind[1..]);

                    Ok(Target {
                        host,
                        book_id: Some(book_id),
                        leaf_id: Some(leaf_id),
                        kind: Some(kind),
                        path: format!("/{book_id}/book/{leaf_id}/page"),
                    })
                }
                _ => Ok(Target { host, book_id: Some(book_id), leaf_id: None, kind: None, path: format!("/{book_id}/book") }),
            }
        }
        [book, rest @ ..] => {
            let book_id = number(Some(book))
                .with_context(|| format!("'{url}' has no book id — expected something like https://site/4/some-book"))?;
            let book_slug = rest.first().copied().unwrap_or("book");

            match rest {
                [_, leaf, leaf_rest @ ..] => {
                    let leaf_id = number(Some(leaf)).with_context(|| format!("'{url}' has no page id"))?;
                    let leaf_slug = leaf_rest.first().copied().unwrap_or("page");

                    Ok(Target {
                        host,
                        book_id: Some(book_id),
                        leaf_id: Some(leaf_id),
                        kind: None,
                        path: format!("/{book_id}/{book_slug}/{leaf_id}/{leaf_slug}"),
                    })
                }
                _ => Ok(Target {
                    host,
                    book_id: Some(book_id),
                    leaf_id: None,
                    kind: None,
                    path: format!("/{book_id}/{book_slug}"),
                }),
            }
        }
    }
}

/// Split Writebook's Markdown export into its front matter (title, url,
/// author) and body.
pub fn split_front_matter(text: &str) -> (Vec<(String, String)>, String) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (Vec::new(), text.to_string());
    };

    let Some((header, body)) = rest.split_once("\n---\n").or_else(|| rest.split_once("\n---").filter(|(_, it)| it.is_empty()))
    else {
        return (Vec::new(), text.to_string());
    };

    let fields = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| {
            let value = value.trim();
            let value = value.strip_prefix('"').and_then(|it| it.strip_suffix('"')).unwrap_or(value);
            (key.trim().to_string(), unescape(value))
        })
        .collect();

    (fields, body.trim_start_matches('\n').to_string())
}

#[derive(Debug, PartialEq)]
pub struct BookLink {
    pub id: u64,
    pub title: String,
    pub author: String,
}

/// The books on a Writebook library page (`/`). Each one renders as a
/// `library__book` figure with a lazy bookmark frame carrying its id.
pub fn parse_library(html: &str) -> Vec<BookLink> {
    html.split("<figure class=\"library__book")
        .skip(1)
        .filter_map(|chunk| {
            let id = between(chunk, "id=\"bookmark_book_", "\"")?.parse().ok()?;
            let title = text_of(between(chunk, "<strong>", "</strong>").unwrap_or_default());
            let after_title = chunk.split_once("</strong>").map(|(_, it)| it).unwrap_or_default();
            let author = text_of(between(after_title, "<span class=\"overflow-line-clamp\">", "</span>").unwrap_or_default());

            Some(BookLink { id, title, author })
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct SearchHit {
    pub title: String,
    pub path: String,
    pub snippet: String,
}

/// The hits on a book's search results page. Each hit renders twice, once
/// for reading mode and once for edit mode; keep the reading-mode link.
pub fn parse_search_results(html: &str) -> Vec<SearchHit> {
    html.split("<a ")
        .skip(1)
        .filter_map(|chunk| {
            let (tag, rest) = chunk.split_once('>')?;

            if !(tag.contains("search__result") && tag.contains("hide_from_edit_mode")) {
                return None;
            }

            let href = unescape(between(tag, "href=\"", "\"")?);
            let path = href.split(['?', '#']).next().unwrap_or_default().to_string();
            let content = rest.split_once("</a>").map(|(it, _)| it).unwrap_or(rest);

            let title = text_of(between(content, "<strong>", "</strong>").unwrap_or_default());
            let title = title.trim_end_matches(':').trim().to_string();
            let snippet = text_of(content.split_once("</strong>").map(|(_, it)| it).unwrap_or(content));

            Some(SearchHit { title, path, snippet })
        })
        .collect()
}

/// The CSRF token Rails puts in every page's head.
pub fn csrf_token(html: &str) -> Option<String> {
    let tag = html.split("<meta ").find(|it| it.starts_with("name=\"csrf-token\""))?;
    between(tag, "content=\"", "\"").map(unescape)
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(start)?;
    rest.split_once(end).map(|(it, _)| it)
}

/// Visible text of an HTML fragment: tags dropped, entities decoded,
/// whitespace collapsed.
fn text_of(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;

    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }

    unescape(&text).split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn unescape(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];

        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|decimal| decimal.parse().ok()))
                    .and_then(char::from_u32),
            };
            character.map(|it| (it, end))
        });

        match decoded {
            Some((character, end)) => {
                result.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                result.push('&');
                rest = &rest[1..];
            }
        }
    }

    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(host: &str, book: Option<u64>, leaf: Option<u64>, kind: Option<&str>, path: &str) -> Target {
        Target { host: host.to_string(), book_id: book, leaf_id: leaf, kind: kind.map(str::to_string), path: path.to_string() }
    }

    #[test]
    fn parses_reading_links() {
        assert_eq!(
            parse_target("https://books.example.com/4/handbook/12/on-call").unwrap(),
            target("https://books.example.com", Some(4), Some(12), None, "/4/handbook/12/on-call")
        );
        assert_eq!(
            parse_target("https://books.example.com/4/handbook/12/on-call.md?search=x#part").unwrap(),
            target("https://books.example.com", Some(4), Some(12), None, "/4/handbook/12/on-call")
        );
        assert_eq!(
            parse_target("http://localhost:3000/4/handbook/").unwrap(),
            target("http://localhost:3000", Some(4), None, None, "/4/handbook")
        );
        assert_eq!(
            parse_target("https://books.example.com/4").unwrap(),
            target("https://books.example.com", Some(4), None, None, "/4/book")
        );
    }

    #[test]
    fn parses_editing_links() {
        assert_eq!(
            parse_target("https://books.example.com/books/4/pages/12/edit").unwrap(),
            target("https://books.example.com", Some(4), Some(12), Some("Page"), "/4/book/12/page")
        );
        assert_eq!(
            parse_target("https://books.example.com/books/4/sections/7").unwrap(),
            target("https://books.example.com", Some(4), Some(7), Some("Section"), "/4/book/7/page")
        );
        assert_eq!(
            parse_target("https://books.example.com/books/4/edit").unwrap(),
            target("https://books.example.com", Some(4), None, None, "/4/book")
        );
    }

    #[test]
    fn parses_site_links() {
        assert_eq!(parse_target("https://books.example.com").unwrap(), target("https://books.example.com", None, None, None, ""));
        assert_eq!(
            parse_target("https://books.example.com/").unwrap(),
            target("https://books.example.com", None, None, None, "")
        );
    }

    #[test]
    fn rejects_bad_links() {
        assert!(parse_target("books.example.com/4/x").is_err());
        assert!(parse_target("https://books.example.com/users/4").is_err());
        assert!(parse_target("https://books.example.com/4/x/nope").is_err());
    }

    #[test]
    fn splits_front_matter() {
        let (fields, body) =
            split_front_matter("---\ntitle: \"On &amp; off call\"\nurl: \"http://x/4/b/12/p\"\n---\n\n# Hello\n");

        assert_eq!(
            fields,
            vec![("title".to_string(), "On & off call".to_string()), ("url".to_string(), "http://x/4/b/12/p".to_string())]
        );
        assert_eq!(body, "# Hello\n");
    }

    #[test]
    fn leaves_text_without_front_matter_alone() {
        assert_eq!(split_front_matter("# Hello"), (Vec::new(), "# Hello".to_string()));
    }

    #[test]
    fn parses_the_library() {
        let html = r#"
          <div class="library">
            <figure class="library__book theme--blue">
              <turbo-frame id="bookmark_book_4" src="/books/4/bookmark" target="_top"></turbo-frame>
              <h2><strong>Ops &amp; On-call</strong>
                <span class="overflow-line-clamp">37signals</span></h2>
            </figure>
            <figure class="library__book position-relative"><div class="library__book--empty center"></div></figure>
          </div>"#;

        assert_eq!(
            parse_library(html),
            vec![BookLink { id: 4, title: "Ops & On-call".to_string(), author: "37signals".to_string() }]
        );
    }

    #[test]
    fn parses_search_results() {
        let html = r#"
          <a class="search__result hide_from_reading_mode txt-ink" data-turbo-frame="_top" href="/books/4/pages/12/edit?search=deploy">
            <strong><mark>Deploy</mark>s:</strong> how to <mark>deploy</mark> &amp; roll back</a>
          <a class="search__result hide_from_edit_mode txt-ink" data-turbo-frame="_top" href="/4/handbook/12/deploys?search=deploy">
            <strong><mark>Deploy</mark>s:</strong> how to <mark>deploy</mark> &amp; roll back</a>"#;

        assert_eq!(
            parse_search_results(html),
            vec![SearchHit {
                title: "Deploys".to_string(),
                path: "/4/handbook/12/deploys".to_string(),
                snippet: "how to deploy & roll back".to_string(),
            }]
        );
    }

    #[test]
    fn finds_the_csrf_token() {
        let html = r#"<meta name="csrf-param" content="authenticity_token" /><meta name="csrf-token" content="abc+/=" />"#;
        assert_eq!(csrf_token(html), Some("abc+/=".to_string()));
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(unescape("a &amp; b &lt;c&gt; &#39;d&#x27; &bogus; & e"), "a & b <c> 'd' &bogus; & e");
    }
}
