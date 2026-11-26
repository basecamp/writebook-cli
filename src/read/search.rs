use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::{book_on_instance, cli_api_json, connect, json_endpoint, list_books, not_readable, unexpected};
use crate::inventory;
use crate::output::{self, Breadcrumb, Format};
use crate::web::{self, Web};

#[derive(Serialize)]
pub struct Hit {
    pub book: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// The snippet as HTML with only the matched words marked, for the TUI.
    /// Writebooks from before it was added don't send it.
    #[serde(skip)]
    pub marked_snippet: Option<String>,
    #[serde(skip)]
    pub book_id: Option<u64>,
    #[serde(skip)]
    pub leaf_id: Option<u64>,
}

#[derive(Deserialize)]
struct ApiSearch {
    results: Vec<ApiHit>,
}

#[derive(Deserialize)]
struct ApiHit {
    title: String,
    url: String,
    snippet: Option<String>,
    marked_snippet: Option<String>,
    book: ApiHitBook,
}

#[derive(Deserialize)]
struct ApiHitBook {
    title: String,
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
    let hits = find(&web, book_id, query)?;

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [Breadcrumb {
                action: "read",
                command: "writebook read <url>".to_string(),
                description: "Print a page as Markdown",
            }];
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

/// Search one book, or every book the reader can see.
pub fn find(web: &Web, book_id: Option<u64>, query: &str) -> Result<Vec<Hit>> {
    match search_api(web, book_id, query)? {
        Some(hits) => Ok(hits),
        None => search_pages(web, book_id, query),
    }
}

fn print_hits_table(hits: &[Hit]) {
    let mut rows = vec![["Book".to_string(), "Title".to_string(), "URL".to_string()]];
    rows.extend(
        hits.iter()
            .map(|hit| [hit.book.clone(), hit.title.clone(), hit.url.clone()]),
    );
    output::print_table(&rows);
}

fn print_hits_markdown(hits: &[Hit]) {
    for hit in hits {
        let book = if hit.book.is_empty() {
            String::new()
        } else {
            format!(" ({})", hit.book)
        };
        output::print_line(&format!("- [{}]({}){book}: {}", hit.title, hit.url, hit.snippet));
    }
}

fn search_api(web: &Web, book_id: Option<u64>, query: &str) -> Result<Option<Vec<Hit>>> {
    let path = match book_id {
        Some(id) => format!("/books/{id}/search.json"),
        None => "/search.json".to_string(),
    };

    match json_endpoint(web, &path, &[("q", query)])? {
        None => Ok(None),
        Some(value) => {
            let search = match serde_json::from_value::<ApiSearch>(value) {
                Ok(search) => search,
                Err(_) => bail!(
                    "{} answered {path} with JSON this version of the CLI doesn't understand",
                    web.host
                ),
            };

            Ok(Some(
                search
                    .results
                    .into_iter()
                    .map(|hit| {
                        let target = web::parse_target(&hit.url).ok();

                        Hit {
                            // One book's search names no book, as the search page doesn't
                            book: if book_id.is_some() {
                                String::new()
                            } else {
                                hit.book.title
                            },
                            title: hit.title,
                            url: hit.url,
                            snippet: one_line(&hit.snippet.unwrap_or_default()),
                            marked_snippet: hit.marked_snippet,
                            book_id: target.as_ref().and_then(|it| it.book_id),
                            leaf_id: target.and_then(|it| it.leaf_id),
                        }
                    })
                    .collect(),
            ))
        }
    }
}

fn search_pages(web: &Web, book_id: Option<u64>, query: &str) -> Result<Vec<Hit>> {
    let books = match book_id {
        Some(id) => vec![(id, String::new())],
        None => list_books(web)?.into_iter().map(|book| (book.id, book.title)).collect(),
    };

    let mut hits = Vec::new();

    for (book_id, book_title) in books {
        for hit in search_book(web, book_id, query)? {
            let url = web.url(&hit.path);
            let leaf_id = web::parse_target(&url).ok().and_then(|it| it.leaf_id);

            hits.push(Hit {
                book: book_title.clone(),
                title: hit.title,
                url,
                snippet: hit.snippet,
                marked_snippet: None,
                book_id: Some(book_id),
                leaf_id,
            });
        }
    }

    Ok(hits)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn check_token(web: &Web, book_id: u64) -> Result<()> {
    if web.signed_in() {
        cli_api_json(web, &format!("/books/{book_id}/manifest.json"))?;
    }

    Ok(())
}

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
