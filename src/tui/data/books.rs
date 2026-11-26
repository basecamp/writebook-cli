use anyhow::{Context, Result};
use serde::Deserialize;

use super::{File, client};
use crate::creds;

#[derive(Deserialize, Clone)]
pub struct BookCard {
    pub id: u64,
    pub title: String,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub theme: Option<String>,
    pub cover: Option<File>,
    #[serde(default)]
    pub editable: bool,
}

/// An instance's books, and whether the sign-in may change them or make new ones.
pub struct Library {
    pub books: Vec<BookCard>,
    pub can_write: bool,
}

pub fn books(host: &str) -> Result<Library> {
    let mut client = client(host)?;
    let can_write = creds::can_write(host)?;
    let mut books: Vec<BookCard> =
        serde_json::from_value(client.get("/books.json")?).context("could not understand the list of books")?;

    for book in &mut books {
        book.editable &= can_write;
    }

    Ok(Library { books, can_write })
}
