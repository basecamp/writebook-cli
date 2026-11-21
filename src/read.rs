//! Read-only commands for people and agents who only need to look things up:
//! list a site's books, print a book or page as Markdown, and search.
//! Nothing here touches a checkout or starts a browser sign-in.
//!
//! Reading prefers the CLI API (the book manifest and leaf JSON) when a token
//! is at hand, and falls back to Writebook's Markdown export (any reading link
//! plus `.md`), which every recent Writebook serves for published books.
//! Listing books and searching scrape the HTML pages until Writebook has JSON
//! endpoints for them.

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::Value;

use crate::checkout;
use crate::creds;
use crate::plan::Manifest;
use crate::sync;
use crate::web::{self, Target, Web};

#[derive(Serialize)]
struct Document {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    url: String,
    markdown: String,
    /// Where it came from: "api" or "export"
    source: &'static str,
}

#[derive(Serialize)]
struct Book {
    id: u64,
    title: String,
    author: String,
    url: String,
}

#[derive(Serialize)]
struct Hit {
    book: String,
    title: String,
    url: String,
    snippet: String,
}

pub fn read(url: &str, json: bool) -> Result<()> {
    let target = web::parse_target(url)?;

    if target.book_id.is_none() {
        bail!("'{url}' is a site, not a book or page — run `writebook books {url}` to see its books");
    }

    let web = connect(&target.host);

    let document = match from_api(&web, &target)? {
        Some(document) => document,
        None => from_export(&web, &target)?,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&document)?);
    } else {
        print!("{}", as_markdown(&document));
    }

    Ok(())
}

pub fn books(url: &str, json: bool) -> Result<()> {
    let target = web::parse_target(url)?;
    let web = connect(&target.host);
    let books = list_books(&web)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&books)?);
    } else if books.is_empty() {
        println!("No books.");
    } else {
        for book in &books {
            if book.author.is_empty() {
                println!("- [{}]({})", book.title, book.url);
            } else {
                println!("- [{}]({}) by {}", book.title, book.url, book.author);
            }
        }
    }

    Ok(())
}

pub fn search(url: &str, query: &str, json: bool) -> Result<()> {
    let target = web::parse_target(url)?;
    let web = connect(&target.host);

    if query.trim().is_empty() {
        bail!("nothing to search for");
    }

    let books = match target.book_id {
        Some(id) => vec![(id, String::new())],
        None => list_books(&web)?.into_iter().map(|book| (book.id, book.title)).collect(),
    };

    let mut hits = Vec::new();

    for (book_id, book_title) in books {
        for hit in search_book(&web, book_id, query)? {
            hits.push(Hit { book: book_title.clone(), title: hit.title, url: web.url(&hit.path), snippet: hit.snippet });
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
    } else if hits.is_empty() {
        println!("No matches.");
    } else {
        for hit in &hits {
            let book = if hit.book.is_empty() { String::new() } else { format!(" ({})", hit.book) };
            println!("- [{}]({}){book}: {}", hit.title, hit.url, hit.snippet);
        }
    }

    Ok(())
}

fn connect(host: &str) -> Web {
    Web::new(host, creds::existing_access_token(host))
}

/// Read through the CLI API. None when there's no token or the site doesn't
/// have the API, so the caller falls back to the Markdown export.
fn from_api(web: &Web, target: &Target) -> Result<Option<Document>> {
    if !web.signed_in() {
        return Ok(None);
    }

    let book_id = target.book_id.expect("a book target");
    let Some(manifest) = get_json(web, &format!("/books/{book_id}/manifest.json"))? else {
        return Ok(None);
    };
    let Ok(manifest) = Manifest::parse(&manifest) else {
        return Ok(None);
    };

    let book_url = web.url(&format!("/{book_id}/{}", manifest.book.slug));

    match target.leaf_id {
        Some(leaf_id) => {
            let Some(leaf) = manifest.leaves.iter().find(|leaf| leaf.id == leaf_id) else {
                bail!("{} has no page {leaf_id} — it may have been deleted", book_url);
            };

            let markdown = leaf_markdown(web, book_id, &leaf.kind, leaf.id)?;
            let slug = Some(checkout::slugify(&leaf.title)).filter(|it| !it.is_empty()).unwrap_or_else(|| "page".to_string());

            Ok(Some(Document {
                title: leaf.title.clone(),
                author: None,
                url: format!("{book_url}/{leaf_id}/{slug}"),
                markdown,
                source: "api",
            }))
        }
        None => {
            // The same shape as Writebook's own book export: every leaf's
            // Markdown, in book order, a blank line apart.
            let mut parts = Vec::new();

            for leaf in &manifest.leaves {
                parts.push(leaf_markdown(web, book_id, &leaf.kind, leaf.id)?);
            }

            Ok(Some(Document {
                title: manifest.book.title.clone(),
                author: manifest.book.author.clone(),
                url: book_url,
                markdown: parts.join("\n\n"),
                source: "api",
            }))
        }
    }
}

fn leaf_markdown(web: &Web, book_id: u64, kind: &str, leaf_id: u64) -> Result<String> {
    let path = sync::leaf_path(book_id, kind, leaf_id);
    let Some(value) = get_json(web, &path)? else {
        bail!("{} could not be read", web.url(&path));
    };
    let content = sync::parse_leaf_content(&value);

    Ok(match kind {
        "Picture" => content.caption.unwrap_or_default(),
        _ => content.body.unwrap_or_default(),
    })
}

/// GET a JSON document. None when the site doesn't have that endpoint.
fn get_json(web: &Web, path: &str) -> Result<Option<Value>> {
    let page = web.get(path, "application/json")?;

    if page.status == 401 {
        bail!("{} did not accept the token — sign in again, or check --token / WRITEBOOK_TOKEN", web.host);
    }

    if !page.ok() {
        return Ok(None);
    }

    Ok(serde_json::from_str(&page.body).ok())
}

fn from_export(web: &Web, target: &Target) -> Result<Document> {
    let page = web.get(&format!("{}.md", target.path), "text/markdown")?;
    let what = if target.leaf_id.is_some() { "page" } else { "book" };

    if page.status == 404 || page.wants_sign_in() {
        bail!("{}", not_readable(web, what, &web.url(&target.path)));
    }

    if !page.ok() {
        bail!("{} answered {} for {}.md", web.host, page.status, web.url(&target.path));
    }

    let (fields, markdown) = web::split_front_matter(&page.body);
    let field = |name: &str| fields.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());

    Ok(Document {
        title: field("title").unwrap_or_default(),
        author: field("author"),
        url: field("url").unwrap_or_else(|| web.url(&target.path)),
        markdown,
        source: "export",
    })
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
             Private books need a token (--token or WRITEBOOK_TOKEN) and a Writebook with the CLI API"
        )
    }
}

fn list_books(web: &Web) -> Result<Vec<Book>> {
    let page = web.get("/", "text/html")?;

    if page.wants_sign_in() {
        if web.signed_in() {
            bail!("{} did not list any books for this token — it may not accept tokens yet", web.host);
        } else {
            bail!("{} has no published books. Listing private ones needs a token (--token or WRITEBOOK_TOKEN)", web.host);
        }
    }

    if !page.ok() {
        bail!("{} answered {} for its book list", web.host, page.status);
    }

    Ok(web::parse_library(&page.body)
        .into_iter()
        .map(|book| {
            // The library page doesn't carry slugs; Writebook finds books by
            // id and ignores the slug, so the title's slug is good enough.
            let slug = Some(checkout::slugify(&book.title)).filter(|it| !it.is_empty()).unwrap_or_else(|| "book".to_string());

            Book { url: web.url(&format!("/{}/{slug}", book.id)), id: book.id, title: book.title, author: book.author }
        })
        .collect())
}

/// Search one book through its search form. A token request skips the
/// forgery check on Writebooks with the CLI API; otherwise this does what a
/// browser does: open the book for a session cookie and form token, then post.
fn search_book(web: &Web, book_id: u64, query: &str) -> Result<Vec<web::SearchHit>> {
    let book_path = format!("/{book_id}/book");
    let book = web.get(&book_path, "text/html")?;

    if book.status == 404 || book.wants_sign_in() {
        bail!("{}", not_readable(web, "book", &web.url(&book_path)));
    }

    if !book.ok() {
        bail!("{} answered {} for {}", web.host, book.status, web.url(&book_path));
    }

    let cookie = book.cookies.join("; ");
    let form_token = web::csrf_token(&book.body).unwrap_or_default();

    let mut form = vec![("search", query)];
    if !form_token.is_empty() {
        form.push(("authenticity_token", form_token.as_str()));
    }

    let results = web.post_form(&format!("/books/{book_id}/search"), &form, Some(&cookie))?;

    if !results.ok() {
        bail!("{} answered {} to a search in book {book_id}", web.host, results.status);
    }

    Ok(web::parse_search_results(&results.body))
}

fn as_markdown(document: &Document) -> String {
    let mut text = String::from("---\n");
    text.push_str(&format!("title: {}\n", quoted(&document.title)));

    if let Some(author) = &document.author {
        text.push_str(&format!("author: {}\n", quoted(author)));
    }

    text.push_str(&format!("url: {}\n", quoted(&document.url)));
    text.push_str("---\n\n");
    text.push_str(&document.markdown);

    if !text.ends_with('\n') {
        text.push('\n');
    }

    text
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_documents_as_markdown_with_front_matter() {
        let document = Document {
            title: "On \"call\"".to_string(),
            author: None,
            url: "https://x/4/b/12/p".to_string(),
            markdown: "# Hello".to_string(),
            source: "export",
        };

        assert_eq!(as_markdown(&document), "---\ntitle: \"On \\\"call\\\"\"\nurl: \"https://x/4/b/12/p\"\n---\n\n# Hello\n");
    }
}
