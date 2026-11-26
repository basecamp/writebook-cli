use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::{connect, json_endpoint, unexpected};
use crate::checkout;
use crate::inventory;
use crate::output::{self, Breadcrumb, Format};
use crate::web::{self, Web};

#[derive(Serialize)]
pub struct Book {
    pub id: u64,
    pub title: String,
    author: String,
    url: String,
}

#[derive(Deserialize)]
struct ApiBook {
    id: u64,
    title: String,
    author: Option<String>,
    url: String,
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
                Breadcrumb {
                    action: "read",
                    command: "writebook read <url>".to_string(),
                    description: "Print a book as Markdown",
                },
                Breadcrumb {
                    action: "clone",
                    command: "writebook clone <url>".to_string(),
                    description: "Clone a book to edit it locally",
                },
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
    let mut rows = vec![[
        "ID".to_string(),
        "Title".to_string(),
        "Author".to_string(),
        "URL".to_string(),
    ]];
    rows.extend(books.iter().map(|book| {
        [
            book.id.to_string(),
            book.title.clone(),
            book.author.clone(),
            book.url.clone(),
        ]
    }));
    output::print_table(&rows);
}

fn print_books_markdown(books: &[Book]) {
    for book in books {
        if book.author.is_empty() {
            output::print_line(&format!("- [{}]({})", book.title, book.url));
        } else {
            output::print_line(&format!("- [{}]({}) by {}", book.title, book.url, book.author));
        }
    }
}

pub fn list_books(web: &Web) -> Result<Vec<Book>> {
    match json_endpoint(web, "/books.json", &[])? {
        Some(value) => {
            let books = match serde_json::from_value::<Vec<ApiBook>>(value) {
                Ok(books) => books,
                Err(_) => bail!(
                    "{} answered /books.json with JSON this version of the CLI doesn't understand",
                    web.host
                ),
            };

            // As the library page does, which sends anonymous readers to sign in
            if books.is_empty() && !web.signed_in() {
                bail!(
                    "{} has no published books. Listing private ones needs a token (WRITEBOOK_TOKEN)",
                    web.host
                );
            }

            Ok(books
                .into_iter()
                .map(|book| Book {
                    id: book.id,
                    title: book.title,
                    author: book.author.unwrap_or_default(),
                    url: book.url,
                })
                .collect())
        }
        None => list_books_from_library(web),
    }
}

fn list_books_from_library(web: &Web) -> Result<Vec<Book>> {
    let page = web.get("/", "text/html")?;

    if page.wants_sign_in() {
        if web.signed_in() {
            bail!(
                "{} did not list any books for this token — it may not accept tokens yet",
                web.host
            );
        } else {
            bail!(
                "{} has no published books. Listing private ones needs a token (WRITEBOOK_TOKEN)",
                web.host
            );
        }
    }

    if !page.ok() {
        bail!("{}", unexpected(web, &page, "/"));
    }

    // The library shows a sign-in button to anonymous readers only.
    if web.signed_in() && page.body.contains("href=\"/session/new\"") {
        eprintln!(
            "note: {} did not accept the token (it may not take tokens yet), so this lists published books only",
            web.host
        );
    }

    Ok(web::parse_library(&page.body)
        .into_iter()
        .map(|book| {
            // The library page doesn't carry slugs; Writebook finds books by
            // id and ignores the slug, so the title's slug is good enough.
            let slug = Some(checkout::slugify(&book.title))
                .filter(|it| !it.is_empty())
                .unwrap_or_else(|| "book".to_string());

            Book {
                url: web.url(&format!("/{}/{slug}", book.id)),
                id: book.id,
                title: book.title,
                author: book.author,
            }
        })
        .collect())
}
