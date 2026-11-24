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

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::checkout;
use crate::creds;
use crate::inventory;
use crate::output::{self, Breadcrumb, Format};
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
struct Contents {
    title: String,
    url: String,
    leaves: Vec<Leaf>,
}

/// One entry in a book: a page, a section or a picture.
#[derive(Serialize)]
struct Leaf {
    id: u64,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    url: String,
}

#[derive(Serialize)]
struct Hit {
    book: String,
    title: String,
    url: String,
    snippet: String,
}

/// A book in `/books.json`
#[derive(Deserialize)]
struct ApiBook {
    id: u64,
    title: String,
    author: Option<String>,
    url: String,
}

/// `/search.json` and `/books/:id/search.json`
#[derive(Deserialize)]
struct ApiSearch {
    results: Vec<ApiHit>,
}

#[derive(Deserialize)]
struct ApiHit {
    title: String,
    url: String,
    snippet: Option<String>,
    book: ApiHitBook,
}

#[derive(Deserialize)]
struct ApiHitBook {
    title: String,
}

pub fn read(url: &str, format: Format) -> Result<()> {
    let target = web::parse_target(url)?;

    if target.book_id.is_none() {
        bail!("'{url}' is a site, not a book or page — run `writebook book list --instance {url}` to see its books");
    }

    let web = connect(&target.host)?;

    let document = match from_api(&web, &target)? {
        Some(document) => document,
        None => from_export(&web, &target)?,
    };

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [
                Breadcrumb {
                    action: "contents",
                    command: format!("writebook book contents {}", document.url),
                    description: "List the book's pages, sections and pictures",
                },
                Breadcrumb {
                    action: "search",
                    command: format!("writebook search \"<query>\" --book {}", document.url),
                    description: "Search the book",
                },
            ];
            output::print_json(&document, &breadcrumbs, jq.as_deref())?;
        }
        _ => print!("{}", as_markdown(&document)),
    }

    Ok(())
}

pub fn books(host: &str, format: Format) -> Result<()> {
    let web = connect(host)?;
    let books = list_books(&web)?;

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [
                Breadcrumb {
                    action: "contents",
                    command: format!("writebook book contents <id> --instance {}", inventory::name_for(host)),
                    description: "List a book's pages, sections and pictures",
                },
                Breadcrumb { action: "read", command: "writebook read <url>".to_string(), description: "Print a book as Markdown" },
                Breadcrumb { action: "clone", command: "writebook clone <url>".to_string(), description: "Clone a book to edit it locally" },
            ];
            output::print_json(&books, &breadcrumbs, jq.as_deref())?;
        }
        _ if books.is_empty() => println!("No books."),
        Format::Table => print_books_table(&books),
        Format::Markdown => print_books_markdown(&books),
    }

    Ok(())
}

fn print_books_table(books: &[Book]) {
    let mut rows = vec![["ID".to_string(), "Title".to_string(), "Author".to_string(), "URL".to_string()]];
    rows.extend(books.iter().map(|book| [book.id.to_string(), book.title.clone(), book.author.clone(), book.url.clone()]));
    output::print_table(&rows);
}

fn print_books_markdown(books: &[Book]) {
    for book in books {
        if book.author.is_empty() {
            println!("- [{}]({})", book.title, book.url);
        } else {
            println!("- [{}]({}) by {}", book.title, book.url, book.author);
        }
    }
}

/// The pages, sections and pictures in a book, in book order. The book is a
/// URL, or an id on the instance from --instance (or the default one).
pub fn contents(book: &str, instance: Option<&str>, format: Format) -> Result<()> {
    let (host, book_id) = book_on_instance(book, instance)?;
    let web = connect(&host)?;

    if !web.signed_in() {
        bail!(
            "listing a book's contents needs a sign-in to {host}. Sign in with `{}`, or set WRITEBOOK_TOKEN",
            inventory::sign_in_command(&host, &inventory::profile_for(&host)?)
        );
    }

    let Some(manifest) = get_json(&web, &format!("/books/{book_id}/manifest.json"))? else {
        bail!("{host} has no book {book_id}, or it doesn't have the CLI API");
    };
    let manifest = Manifest::parse(&manifest)?;
    let book_url = web.url(&format!("/{book_id}/{}", manifest.book.slug));

    let leaves = manifest
        .leaves
        .iter()
        .map(|leaf| Leaf {
            id: leaf.id,
            kind: leaf.kind.to_lowercase(),
            title: leaf.title.clone(),
            url: format!("{book_url}/{}/{}", leaf.id, checkout::slugify(&leaf.title)),
        })
        .collect();

    let contents = Contents { title: manifest.book.title.clone(), url: book_url, leaves };

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [
                Breadcrumb { action: "read", command: "writebook read <url>".to_string(), description: "Print a page as Markdown" },
                Breadcrumb {
                    action: "search",
                    command: format!("writebook search \"<query>\" --book {}", contents.url),
                    description: "Search the book",
                },
                Breadcrumb {
                    action: "clone",
                    command: format!("writebook clone {}", contents.url),
                    description: "Clone the book to edit it locally",
                },
            ];
            output::print_json(&contents, &breadcrumbs, jq.as_deref())?;
        }
        _ if contents.leaves.is_empty() => println!("{} has no pages yet.", contents.title),
        Format::Table => print_contents_table(&contents),
        Format::Markdown => print_contents_markdown(&contents),
    }

    Ok(())
}

fn book_on_instance(book: &str, instance: Option<&str>) -> Result<(String, u64)> {
    if book.contains("://") {
        let target = web::parse_target(book)?;

        match target.book_id {
            Some(book_id) => Ok((target.host, book_id)),
            None => bail!("'{book}' is a site, not a book — run `writebook book list --instance {book}` to see its books"),
        }
    } else {
        match book.parse() {
            Ok(book_id) => Ok((inventory::host(instance)?, book_id)),
            Err(_) => bail!("'{book}' is neither a book id nor a book URL"),
        }
    }
}

fn print_contents_table(contents: &Contents) {
    let mut rows = vec![["ID".to_string(), "Type".to_string(), "Title".to_string(), "URL".to_string()]];
    rows.extend(contents.leaves.iter().map(|leaf| [leaf.id.to_string(), capitalize(&leaf.kind), leaf.title.clone(), leaf.url.clone()]));
    output::print_table(&rows);
}

fn print_contents_markdown(contents: &Contents) {
    println!("# [{}]({})", contents.title, contents.url);
    println!();

    for (number, leaf) in contents.leaves.iter().enumerate() {
        if leaf.kind == "page" {
            println!("{}. [{}]({})", number + 1, leaf.title, leaf.url);
        } else {
            println!("{}. [{}]({}) ({})", number + 1, leaf.title, leaf.url, leaf.kind);
        }
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();

    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Search one book (a book id or URL), or every book on the instance.
pub fn search(query: &str, book: Option<&str>, instance: Option<&str>, format: Format) -> Result<()> {
    if query.trim().is_empty() {
        bail!("nothing to search for");
    }

    let (host, book_id) = match book {
        Some(book) => {
            let (host, book_id) = book_on_instance(book, instance)?;
            (host, Some(book_id))
        }
        None => (inventory::host(instance)?, None),
    };
    let web = connect(&host)?;

    let hits = match search_api(&web, book_id, query)? {
        Some(hits) => hits,
        None => search_pages(&web, book_id, query)?,
    };

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [Breadcrumb { action: "read", command: "writebook read <url>".to_string(), description: "Print a page as Markdown" }];
            output::print_json(&hits, &breadcrumbs, jq.as_deref())?;
        }
        _ if hits.is_empty() => {
            println!("No matches.");

            if !web.signed_in() {
                eprintln!("note: this searched published books only. Private ones need a token (WRITEBOOK_TOKEN)");
            }
        }
        Format::Table => print_hits_table(&hits),
        Format::Markdown => print_hits_markdown(&hits),
    }

    Ok(())
}

fn print_hits_table(hits: &[Hit]) {
    let mut rows = vec![["Book".to_string(), "Title".to_string(), "URL".to_string()]];
    rows.extend(hits.iter().map(|hit| [hit.book.clone(), hit.title.clone(), hit.url.clone()]));
    output::print_table(&rows);
}

fn print_hits_markdown(hits: &[Hit]) {
    for hit in hits {
        let book = if hit.book.is_empty() { String::new() } else { format!(" ({})", hit.book) };
        println!("- [{}]({}){book}: {}", hit.title, hit.url, hit.snippet);
    }
}

/// Search through the JSON endpoint: one book's, or every book's the reader
/// can see in one request. None when the site doesn't have it.
fn search_api(web: &Web, book_id: Option<u64>, query: &str) -> Result<Option<Vec<Hit>>> {
    let path = match book_id {
        Some(id) => format!("/books/{id}/search.json"),
        None => "/search.json".to_string(),
    };

    let Some(value) = api_json(web, &path, &[("q", query)])? else {
        return Ok(None);
    };
    let Ok(search) = serde_json::from_value::<ApiSearch>(value) else {
        bail!("{} answered {path} with JSON this version of the CLI doesn't understand", web.host);
    };

    Ok(Some(
        search
            .results
            .into_iter()
            .map(|hit| Hit {
                // One book's search names no book, as the search page doesn't
                book: if book_id.is_some() { String::new() } else { hit.book.title },
                title: hit.title,
                url: hit.url,
                snippet: one_line(&hit.snippet.unwrap_or_default()),
            })
            .collect(),
    ))
}

/// Search the way a browser does, one book at a time.
fn search_pages(web: &Web, book_id: Option<u64>, query: &str) -> Result<Vec<Hit>> {
    let books = match book_id {
        Some(id) => vec![(id, String::new())],
        None => list_books(web)?.into_iter().map(|book| (book.id, book.title)).collect(),
    };

    let mut hits = Vec::new();

    for (book_id, book_title) in books {
        for hit in search_book(web, book_id, query)? {
            hits.push(Hit { book: book_title.clone(), title: hit.title, url: web.url(&hit.path), snippet: hit.snippet });
        }
    }

    Ok(hits)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn connect(host: &str) -> Result<Web> {
    Ok(Web::new(host, creds::existing_access_token(host)?))
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

/// GET a JSON document. None when the site doesn't have that endpoint (404,
/// or an answer that isn't JSON); any other failure is an error.
fn get_json(web: &Web, path: &str) -> Result<Option<Value>> {
    let page = web.get(path, "application/json")?;

    if page.status == 401 {
        bail!("{} did not accept the token — sign in again, or check WRITEBOOK_TOKEN", web.host);
    }

    if matches!(page.status, 404 | 406) {
        return Ok(None);
    }

    if !page.ok() {
        bail!("{}", unexpected(web, &page, path));
    }

    Ok(serde_json::from_str(&page.body).ok())
}

/// GET one of the JSON endpoints that stand in for HTML pages. None when the
/// site doesn't have it: not found, not acceptable (an older Writebook has
/// only the HTML page), a redirect, or an answer that isn't JSON. The caller
/// then reads the HTML page, which explains a private site or book.
fn api_json(web: &Web, path: &str, query: &[(&str, &str)]) -> Result<Option<Value>> {
    let page = web.get_query(path, "application/json", query)?;

    if page.status == 401 {
        bail!("{} did not accept the token — sign in again, or check WRITEBOOK_TOKEN", web.host);
    }

    if matches!(page.status, 404 | 406) || (300..400).contains(&page.status) {
        return Ok(None);
    }

    if !page.ok() {
        bail!("{}", unexpected(web, &page, path));
    }

    Ok(serde_json::from_str(&page.body).ok())
}

fn unexpected(web: &Web, page: &web::Page, path: &str) -> String {
    match &page.location {
        Some(location) if (300..400).contains(&page.status) => {
            format!("{} answered {} for {path}, sending it to {location} — try that address", web.host, page.status)
        }
        _ => format!("{} answered {} for {path}", web.host, page.status),
    }
}

/// With a token, make sure the site accepts it before reading pages that
/// also open to anonymous readers, which would otherwise quietly show less.
/// Sites without the CLI API don't take tokens at all; that's not an error.
fn check_token(web: &Web, book_id: u64) -> Result<()> {
    if web.signed_in() {
        get_json(web, &format!("/books/{book_id}/manifest.json"))?;
    }

    Ok(())
}

fn from_export(web: &Web, target: &Target) -> Result<Document> {
    let page = web.get(&format!("{}.md", target.path), "text/markdown")?;
    let what = if target.leaf_id.is_some() { "page" } else { "book" };

    if page.status == 404 || page.wants_sign_in() {
        bail!("{}", not_readable(web, what, &web.url(&target.path)));
    }

    if !page.ok() {
        bail!("{}", unexpected(web, &page, &format!("{}.md", target.path)));
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
             Private books need a token (WRITEBOOK_TOKEN) and a Writebook with the CLI API"
        )
    }
}

fn list_books(web: &Web) -> Result<Vec<Book>> {
    if let Some(value) = api_json(web, "/books.json", &[])? {
        let Ok(books) = serde_json::from_value::<Vec<ApiBook>>(value) else {
            bail!("{} answered /books.json with JSON this version of the CLI doesn't understand", web.host);
        };

        // As the library page does, which sends anonymous readers to sign in
        if books.is_empty() && !web.signed_in() {
            bail!("{} has no published books. Listing private ones needs a token (WRITEBOOK_TOKEN)", web.host);
        }

        return Ok(books
            .into_iter()
            .map(|book| Book { id: book.id, title: book.title, author: book.author.unwrap_or_default(), url: book.url })
            .collect());
    }

    list_books_from_library(web)
}

/// The books on the library page, for Writebooks without `/books.json`
fn list_books_from_library(web: &Web) -> Result<Vec<Book>> {
    let page = web.get("/", "text/html")?;

    if page.wants_sign_in() {
        if web.signed_in() {
            bail!("{} did not list any books for this token — it may not accept tokens yet", web.host);
        } else {
            bail!("{} has no published books. Listing private ones needs a token (WRITEBOOK_TOKEN)", web.host);
        }
    }

    if !page.ok() {
        bail!("{}", unexpected(web, &page, "/"));
    }

    // The library shows a sign-in button to anonymous readers only.
    if web.signed_in() && page.body.contains("href=\"/session/new\"") {
        eprintln!("note: {} did not accept the token (it may not take tokens yet), so this lists published books only", web.host);
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
    check_token(web, book_id)?;

    let book_path = format!("/{book_id}/book");
    let book = web.get(&book_path, "text/html")?;

    if book.status == 404 || book.wants_sign_in() {
        bail!("{}", not_readable(web, "book", &web.url(&book_path)));
    }

    if !book.ok() {
        bail!("{}", unexpected(web, &book, &book_path));
    }

    let cookie = book.cookies.join("; ");
    let form_token = web::csrf_token(&book.body).unwrap_or_default();

    let mut form = vec![("search", query)];
    if !form_token.is_empty() {
        form.push(("authenticity_token", form_token.as_str()));
    }

    let results = web.post_form(&format!("/books/{book_id}/search"), &form, Some(&cookie))?;

    if !results.ok() {
        bail!("{}", unexpected(web, &results, &format!("/books/{book_id}/search")));
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
