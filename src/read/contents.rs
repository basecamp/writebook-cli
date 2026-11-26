use anyhow::{Result, bail};
use serde::Serialize;

use super::{book_on_instance, cli_api_json, connect};
use crate::checkout;
use crate::inventory;
use crate::output::{self, Breadcrumb, Format};
use crate::plan::Manifest;
use crate::web::Web;

#[derive(Serialize)]
pub struct Contents {
    title: String,
    url: String,
    leaves: Vec<Leaf>,
}

/// One entry in a book: a page, a section or a picture.
#[derive(Serialize)]
pub struct Leaf {
    id: u64,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    url: String,
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

    let contents = book_contents(&web, book_id)?;

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [
                Breadcrumb {
                    action: "read",
                    command: "writebook read <url>".to_string(),
                    description: "Print a page as Markdown",
                },
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
        _ if contents.leaves.is_empty() => output::print_line(&format!("{} has no pages yet.", contents.title)),
        Format::Table => print_contents_table(&contents),
        Format::Markdown => print_contents_markdown(&contents),
    }

    Ok(())
}

/// Every page, section and picture in a book, in order. Needs a token.
pub fn book_contents(web: &Web, book_id: u64) -> Result<Contents> {
    let Some(manifest) = cli_api_json(web, &format!("/books/{book_id}/manifest.json"))? else {
        bail!("{} has no book {book_id}, or it doesn't have the CLI API", web.host);
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

    Ok(Contents {
        title: manifest.book.title.clone(),
        url: book_url,
        leaves,
    })
}

fn print_contents_table(contents: &Contents) {
    let mut rows = vec![[
        "ID".to_string(),
        "Type".to_string(),
        "Title".to_string(),
        "URL".to_string(),
    ]];
    rows.extend(contents.leaves.iter().map(|leaf| {
        [
            leaf.id.to_string(),
            capitalize(&leaf.kind),
            leaf.title.clone(),
            leaf.url.clone(),
        ]
    }));
    output::print_table(&rows);
}

fn print_contents_markdown(contents: &Contents) {
    output::print_line(&format!("# [{}]({})", contents.title, contents.url));
    println!();

    for (number, leaf) in contents.leaves.iter().enumerate() {
        if leaf.kind == "page" {
            output::print_line(&format!("{}. [{}]({})", number + 1, leaf.title, leaf.url));
        } else {
            output::print_line(&format!(
                "{}. [{}]({}) ({})",
                number + 1,
                leaf.title,
                leaf.url,
                leaf.kind
            ));
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
