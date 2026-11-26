use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use super::{File, client, text};
use crate::api::{Part, PartData};
use crate::leaf_kind::LeafKind;
use crate::{creds, sync};

pub struct Book {
    pub id: u64,
    pub title: String,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub theme: Option<String>,
    pub cover: Option<String>,
    pub slug: String,
    pub published: bool,
    pub editable: bool,
    pub leaves: Vec<Leaf>,
}

#[derive(Deserialize, Clone)]
pub struct Leaf {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: LeafKind,
    pub title: String,
    pub word_count: Option<u64>,
    pub preview: Option<String>,
    pub image: Option<File>,
}

impl Leaf {
    pub fn is_section(&self) -> bool {
        self.kind == LeafKind::Section
    }

    pub fn is_picture(&self) -> bool {
        self.kind == LeafKind::Picture
    }
}

pub struct Publication {
    pub published: bool,
    pub slug: String,
    pub url: String,
}

pub enum NewLeaf {
    Page { title: String },
    Section { title: String },
    Picture { caption: String, path: String },
}

pub fn book(host: &str, book_id: u64) -> Result<Book> {
    let mut client = client(host)?;
    let manifest = client.get(&format!("/books/{book_id}/manifest.json"))?;
    let leaves: Vec<Leaf> =
        serde_json::from_value(manifest["leaves"].clone()).context("could not understand the book's pages")?;
    let book = &manifest["book"];

    Ok(Book {
        id: book_id,
        title: text(book, "title").unwrap_or_default(),
        subtitle: text(book, "subtitle"),
        author: text(book, "author"),
        theme: text(book, "theme"),
        cover: book["cover"]["url"].as_str().map(str::to_string),
        slug: text(book, "slug").unwrap_or_default(),
        published: book["published"].as_bool().unwrap_or(false),
        editable: book["editable"].as_bool().unwrap_or(false) && creds::can_write(host)?,
        leaves,
    })
}

pub fn add(host: &str, book_id: u64, leaf: &NewLeaf) -> Result<()> {
    let mut client = client(host)?;

    match leaf {
        NewLeaf::Page { title } => {
            client.post(
                &format!("/books/{book_id}/pages.json"),
                &json!({ "leaf": { "title": title }, "page": { "body": "" } }),
            )?;
        }
        NewLeaf::Section { title } => {
            client.post(
                &format!("/books/{book_id}/sections.json"),
                &json!({ "leaf": { "title": title }, "section": { "body": "" } }),
            )?;
        }
        NewLeaf::Picture { caption, path } => {
            let bytes = std::fs::read(path).with_context(|| format!("could not read {path}"))?;
            let filename = std::path::Path::new(&path)
                .file_name()
                .and_then(|it| it.to_str())
                .unwrap_or("picture");

            client.post_multipart(
                &format!("/books/{book_id}/pictures.json"),
                &[
                    Part {
                        name: "leaf[title]",
                        filename: None,
                        data: PartData::Text(caption),
                    },
                    Part {
                        name: "picture[caption]",
                        filename: None,
                        data: PartData::Text(caption),
                    },
                    Part {
                        name: "picture[image]",
                        filename: Some(filename),
                        data: PartData::Bytes(&bytes),
                    },
                ],
            )?;
        }
    }

    Ok(())
}

pub fn delete(host: &str, book_id: u64, leaf: &Leaf) -> Result<()> {
    client(host)?.delete(&sync::leaf_path(book_id, &leaf.kind, leaf.id))?;
    Ok(())
}

/// Put the book's leaves in this order.
pub fn arrange(host: &str, book_id: u64, order: &[u64]) -> Result<()> {
    client(host)?.post(
        &format!("/books/{book_id}/leaves/moves.json"),
        &json!({ "id": order, "position": 0 }),
    )?;
    Ok(())
}

pub fn publish(host: &str, book_id: u64, published: bool, slug: &str) -> Result<Publication> {
    let response = client(host)?.patch(
        &format!("/books/{book_id}/publication.json"),
        &json!({ "book": { "published": published, "slug": slug } }),
    )?;

    match (
        response["published"].as_bool(),
        text(&response, "slug"),
        text(&response, "url"),
    ) {
        (Some(published), Some(slug), Some(url)) => Ok(Publication { published, slug, url }),
        _ => bail!("{host} didn't say how the book is published now; it may need a newer Writebook"),
    }
}
