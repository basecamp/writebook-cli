//! What the TUI shows, fetched from the instances in the inventory, and the
//! changes it makes. Everything here blocks, so the TUI runs it on a thread.

mod book;
mod book_form;
mod books;
mod images;
mod instances;
mod leaf;
mod people;
mod profile;
mod search;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::api;
use crate::creds;
use crate::inventory;

pub use book::{Book, Leaf, NewLeaf, Publication, add, arrange, book, delete, publish};
pub use book_form::{BookForm, Role, book_form, delete_book, save_book};
pub use books::{BookCard, Library, books};
pub use images::{image, image_file};
pub use instances::{Instance, instances};
pub use leaf::{LeafContent, leaf, save_leaf, upload};
pub use people::{People, Person, people, remove_person, reset_invite, set_role};
pub use profile::{Profile, profile, save_profile};
pub use search::{Found, search};

#[derive(Deserialize, Clone)]
pub struct File {
    pub url: String,
}

fn client(host: &str) -> Result<api::Client> {
    Ok(api::Client::with_token(host, token(host)?))
}

fn token(host: &str) -> Result<creds::Token> {
    match creds::signed_in_token(host)? {
        Some(token) => Ok(token),
        None => bail!(
            "you're not signed in to {host}. Sign in with `{}`",
            inventory::sign_in_command(host, &inventory::profile_for(host)?)
        ),
    }
}

fn text(value: &Value, field: &str) -> Option<String> {
    value[field].as_str().map(str::to_string).filter(|it| !it.is_empty())
}
