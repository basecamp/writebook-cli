//! Read-only commands for people and agents who only need to look things up:
//! list a site's books and a book's contents, print a book or page as
//! Markdown, and search.
//! Nothing here touches a checkout or starts a browser sign-in.
//!
//! Reading prefers the CLI API (the book manifest and leaf JSON) when a token
//! is at hand, and falls back to Writebook's Markdown export (any reading link
//! plus `.md`), which every recent Writebook serves for published books.
//! Listing books and searching use Writebook's JSON endpoints (`/books.json`,
//! `/search.json` and `/books/:id/search.json`), which read-only tokens may
//! call, and scrape the HTML pages on Writebooks without them.

mod contents;
mod document;
mod library;
mod search;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::api::TokenRejected;
use crate::creds;
use crate::inventory;
use crate::web::{self, Web};

pub use contents::{book_contents, contents};
pub use document::{read, whole_book};
pub use library::{books, list_books};
pub use search::{find, search};

fn connect(host: &str) -> Result<Web> {
    Ok(Web::new(host, creds::existing_access_token(host)?))
}

fn book_on_instance(book: &str, instance: Option<&str>) -> Result<(String, u64)> {
    if book.contains("://") {
        let target = web::parse_target(book)?;

        match target.book_id {
            Some(book_id) => Ok((target.host, book_id)),
            None => {
                bail!("'{book}' is a site, not a book — run `writebook book list --instance {book}` to see its books")
            }
        }
    } else {
        match book.parse() {
            Ok(book_id) => Ok((inventory::host(instance)?, book_id)),
            Err(_) => bail!("'{book}' is neither a book id nor a book URL"),
        }
    }
}

fn cli_api_json(web: &Web, path: &str) -> Result<Option<Value>> {
    let page = web.get(path, "application/json")?;

    if page.status == 401 {
        Err(TokenRejected { host: web.host.clone() }.into())
    } else if matches!(page.status, 404 | 406) {
        Ok(None)
    } else if !page.ok() {
        bail!("{}", unexpected(web, &page, path))
    } else {
        Ok(serde_json::from_str(&page.body).ok())
    }
}

fn json_endpoint(web: &Web, path: &str, query: &[(&str, &str)]) -> Result<Option<Value>> {
    let page = web.get_query(path, "application/json", query)?;

    if page.status == 401 {
        Err(TokenRejected { host: web.host.clone() }.into())
    } else if matches!(page.status, 404 | 406) || (300..400).contains(&page.status) {
        Ok(None)
    } else if !page.ok() {
        bail!("{}", unexpected(web, &page, path))
    } else {
        Ok(serde_json::from_str(&page.body).ok())
    }
}

fn unexpected(web: &Web, page: &web::Page, path: &str) -> String {
    match &page.location {
        Some(location) if (300..400).contains(&page.status) => {
            format!(
                "{} answered {} for {path}, sending it to {location} — try that address",
                web.host, page.status
            )
        }
        _ => format!("{} answered {} for {path}", web.host, page.status),
    }
}

fn not_readable(web: &Web, what: &str, url: &str) -> String {
    if web.signed_in() {
        format!(
            "{url} was not found, or you can't read it: the {what} may not exist, \
             or {} may not accept tokens yet (it needs the CLI API)",
            web.host
        )
    } else {
        format!(
            "{url} was not found, or it's private: the {what} may not exist, or the book isn't published. \
             Private books need a token (WRITEBOOK_TOKEN) and a Writebook with the CLI API"
        )
    }
}
