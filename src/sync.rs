//! Shared machinery for bringing remote content onto disk: reading the book's
//! manifest, fetching leaf content, materializing new leaves as local files,
//! and keeping embedded uploads mirrored in assets/.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::api;
use crate::checkout::{self, Checkout, LeafState, State};
use crate::fsutil;
use crate::index::{BookMeta, Entry};
use crate::leaf_kind::LeafKind;
use crate::merge;

#[derive(Deserialize)]
pub struct Manifest {
    pub book: ManifestBook,
    pub leaves: Vec<ManifestLeaf>,
}

#[derive(Deserialize)]
pub struct ManifestBook {
    pub title: String,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub theme: String,
    pub slug: String,
    pub fingerprint: String,
    pub cover: Option<RemoteFile>,
}

#[derive(Deserialize, Clone)]
pub struct ManifestLeaf {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: LeafKind,
    pub title: String,
    pub fingerprint: String,
}

#[derive(Deserialize, Clone)]
pub struct RemoteFile {
    pub checksum: String,
    pub filename: String,
    pub url: String,
}

impl Manifest {
    pub fn parse(value: &serde_json::Value) -> Result<Manifest> {
        serde_json::from_value(value.clone()).context("could not understand the server's manifest")
    }
}

pub struct RemoteLeafContent {
    pub title: String,
    pub fingerprint: String,
    pub body: Option<String>,
    pub theme: Option<String>,
    pub caption: Option<String>,
    pub image: Option<RemoteFile>,
}

pub fn fetch_manifest(client: &mut api::Client, book_id: u64) -> Result<Manifest> {
    let value = client.get(&format!("/books/{book_id}/manifest.json"))?;
    Manifest::parse(&value)
}

pub fn leaf_path(book_id: u64, kind: &LeafKind, leaf_id: u64) -> String {
    format!("/books/{book_id}/{}s/{leaf_id}.json", kind.name().to_lowercase())
}

pub fn fetch_leaf(client: &mut api::Client, book_id: u64, kind: &LeafKind, leaf_id: u64) -> Result<RemoteLeafContent> {
    let value = client.get(&leaf_path(book_id, kind, leaf_id))?;
    Ok(parse_leaf_content(&value))
}

pub fn parse_leaf_content(value: &Value) -> RemoteLeafContent {
    RemoteLeafContent {
        title: value["title"].as_str().unwrap_or_default().to_string(),
        fingerprint: value["fingerprint"].as_str().unwrap_or_default().to_string(),
        body: value["body"].as_str().map(str::to_string),
        theme: value["theme"].as_str().map(str::to_string),
        caption: value["caption"].as_str().map(str::to_string),
        image: serde_json::from_value(value["image"].clone()).ok(),
    }
}

/// Bring a leaf the checkout doesn't have yet onto disk: fetch its content,
/// pick a free filename, write the working and base copies, record it in
/// state, and hand back its index entry.
pub fn materialize_leaf(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    book_id: u64,
    leaf: &ManifestLeaf,
    taken: &mut Vec<String>,
) -> Result<Option<Entry>> {
    let content = fetch_leaf(client, book_id, &leaf.kind, leaf.id)?;

    match &leaf.kind {
        LeafKind::Page => {
            let filename = claim_filename(page_filename(&leaf.title, leaf.id), taken);
            let body = merge::localize_upload_urls(&content.body.unwrap_or_default());
            mirror_uploads(client, checkout, state, &body)?;
            checkout.write_working(&filename, &body)?;
            checkout.write_base(&filename, &body)?;

            state.leaves.push(LeafState {
                filename: Some(filename.clone()),
                ..new_leaf_state(leaf, content.title.clone(), content.fingerprint)
            });

            Ok(Some(Entry::Page {
                title: content.title,
                filename,
            }))
        }
        LeafKind::Section => {
            let body = content.body.unwrap_or_default();

            let filename = if body.is_empty() {
                None
            } else {
                let filename = claim_filename(page_filename(&leaf.title, leaf.id), taken);
                checkout.write_working(&filename, &body)?;
                checkout.write_base(&filename, &body)?;
                Some(filename)
            };

            state.leaves.push(LeafState {
                filename: filename.clone(),
                theme: content.theme.clone(),
                ..new_leaf_state(leaf, content.title.clone(), content.fingerprint)
            });

            Ok(Some(Entry::Section {
                title: content.title,
                filename,
                theme: content.theme,
            }))
        }
        LeafKind::Picture => {
            // A picture's index line carries its caption, so that's the title
            // we track; the leaf's own title stays server-side.
            let caption = content.caption.unwrap_or_default();

            match content.image {
                None => {
                    // A picture with no image has nothing to edit locally. It stays
                    // in state so we don't mistake it for a deletion, but gets no
                    // file and no index line.
                    state.leaves.push(new_leaf_state(leaf, caption, content.fingerprint));

                    Ok(None)
                }
                Some(image) => {
                    let filename = claim_filename(picture_filename(&image), taken);
                    let bytes = client.download(&image.url)?;
                    fsutil::write_atomically(&checkout.working_path(&filename)?, &bytes)?;

                    state.leaves.push(LeafState {
                        filename: Some(filename.clone()),
                        image_remote_checksum: Some(image.checksum),
                        image_sha256: Some(checkout::sha256(&bytes)),
                        ..new_leaf_state(leaf, caption.clone(), content.fingerprint)
                    });

                    Ok(Some(Entry::Picture { caption, filename }))
                }
            }
        }
        LeafKind::Unknown(other) => anyhow::bail!("the server sent an unknown leaf type: {other}"),
    }
}

fn new_leaf_state(leaf: &ManifestLeaf, title: String, fingerprint: String) -> LeafState {
    LeafState {
        id: leaf.id,
        kind: leaf.kind.clone(),
        title,
        fingerprint,
        filename: None,
        theme: None,
        image_remote_checksum: None,
        image_sha256: None,
    }
}

/// Download every /u/ upload a page body references that assets/ doesn't hold yet.
pub fn mirror_uploads(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    localized_body: &str,
) -> Result<()> {
    for slug in merge::referenced_assets(localized_body) {
        let path = checkout.working_path(&format!("assets/{slug}"))?;

        if state.uploads.contains_key(&slug) && path.exists() {
            continue;
        }

        let bytes = client.download(&format!("/u/{slug}"))?;
        fsutil::write_atomically(&path, &bytes)?;
        state.uploads.insert(slug, checkout::sha256(&bytes));
    }

    Ok(())
}

pub fn book_meta(manifest: &Manifest, state: &State) -> BookMeta {
    BookMeta {
        title: manifest.book.title.clone(),
        subtitle: manifest.book.subtitle.clone().unwrap_or_default(),
        author: manifest.book.author.clone().unwrap_or_default(),
        theme: manifest.book.theme.clone(),
        cover: Some(cover_filename(manifest, state)),
    }
}

fn cover_filename(manifest: &Manifest, state: &State) -> String {
    match (&manifest.book.cover, &state.cover) {
        (Some(remote), Some(current)) if current.remote_checksum == remote.checksum => current.filename.clone(),
        (Some(remote), _) => remote_cover_filename(remote),
        (None, _) => String::new(),
    }
}

/// Mirror the book cover into assets/, tracking it in state. Returns true
/// when it kept a cover you changed locally instead of the remote one; the
/// next push uploads it.
pub fn mirror_cover(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    manifest: &Manifest,
) -> Result<bool> {
    match (&manifest.book.cover, state.cover.clone()) {
        (Some(remote), current) => {
            if current.as_ref().is_some_and(|it| it.remote_checksum == remote.checksum) {
                Ok(false)
            } else if current.as_ref().is_some_and(|it| cover_modified_locally(checkout, it)) {
                eprintln!("warning: the cover changed both locally and remotely — keeping your copy");
                Ok(true)
            } else {
                let filename = remote_cover_filename(remote);
                let bytes = client.download(&remote.url)?;

                if let Some(current) = &current {
                    if current.filename != filename && !used_by_pages_or_pictures(state, &current.filename) {
                        checkout.remove_working_if_present(&current.filename)?;
                    }
                }

                fsutil::write_atomically(&checkout.working_path(&filename)?, &bytes)?;

                state.cover = Some(checkout::CoverState {
                    filename,
                    remote_checksum: remote.checksum.clone(),
                    sha256: checkout::sha256(&bytes),
                });
                Ok(false)
            }
        }
        (None, Some(current)) => {
            if cover_modified_locally(checkout, &current) {
                eprintln!("warning: the cover was removed remotely but modified locally — keeping your copy");
                Ok(true)
            } else {
                if !used_by_pages_or_pictures(state, &current.filename) {
                    checkout.remove_working_if_present(&current.filename)?;
                }

                state.cover = None;
                Ok(false)
            }
        }
        (None, None) => Ok(false),
    }
}

fn used_by_pages_or_pictures(state: &State, filename: &str) -> bool {
    let basename = filename.strip_prefix("assets/").unwrap_or(filename);
    state.leaves.iter().any(|it| it.filename.as_deref() == Some(filename)) || state.uploads.contains_key(basename)
}

fn cover_modified_locally(checkout: &Checkout, cover: &checkout::CoverState) -> bool {
    match checkout.read_working_bytes(&cover.filename) {
        Ok(bytes) => checkout::sha256(&bytes) != cover.sha256,
        Err(_) => false,
    }
}

pub fn page_filename(title: &str, id: u64) -> String {
    let slug = checkout::slugify(title);

    if slug.is_empty() {
        format!("page-{id}.md")
    } else {
        format!("{slug}.md")
    }
}

pub fn picture_filename(image: &RemoteFile) -> String {
    format!("assets/{}", sanitize_filename(&image.filename))
}

fn remote_cover_filename(cover: &RemoteFile) -> String {
    format!("assets/cover-{}", sanitize_filename(&cover.filename))
}

/// A filename nothing in `taken` has yet, which it then takes: the wanted
/// name as-is, or with -2, -3, ... spliced in before the extension.
pub fn claim_filename(wanted: String, taken: &mut Vec<String>) -> String {
    let filename = if taken.contains(&wanted) {
        let (stem, extension) = match wanted.rsplit_once('.') {
            Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
            None => (wanted.clone(), String::new()),
        };

        (2..)
            .map(|counter| format!("{stem}-{counter}{extension}"))
            .find(|candidate| !taken.contains(candidate))
            .expect("an unbounded counter always finds a free name")
    } else {
        wanted
    };

    taken.push(filename.clone());
    filename
}

pub fn sanitize_filename(filename: &str) -> String {
    let cleaned: String = filename
        .chars()
        .map(|it| {
            if it.is_alphanumeric() || it == '.' || it == '-' || it == '_' {
                it
            } else {
                '-'
            }
        })
        .collect();

    cleaned.trim_start_matches(['-', '.']).trim_end_matches('-').to_string()
}
