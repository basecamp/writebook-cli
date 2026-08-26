//! Shared machinery for bringing remote content onto disk: fetching leaf
//! content, materializing new leaves as local files, and keeping embedded
//! uploads mirrored in assets/.

use anyhow::Result;
use serde_json::Value;

use crate::api;
use crate::checkout::{self, Checkout, LeafState, State};
use crate::fsutil;
use crate::index::Entry;
use crate::merge;
use crate::plan::{Manifest, ManifestLeaf, RemoteFile};

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

pub fn leaf_path(book_id: u64, kind: &str, leaf_id: u64) -> String {
    format!("/books/{book_id}/{}s/{leaf_id}.json", kind.to_lowercase())
}

pub fn fetch_leaf(client: &mut api::Client, book_id: u64, kind: &str, leaf_id: u64) -> Result<RemoteLeafContent> {
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

    match leaf.kind.as_str() {
        "Page" => {
            let filename = claim_filename(page_filename(&leaf.title, leaf.id), taken);
            let body = merge::localize_upload_urls(&content.body.unwrap_or_default());
            mirror_uploads(client, checkout, state, &body)?;
            checkout.write_working(&filename, &body)?;
            checkout.write_base(&filename, &body)?;

            state.leaves.push(LeafState {
                id: leaf.id,
                kind: leaf.kind.clone(),
                title: content.title.clone(),
                fingerprint: content.fingerprint,
                filename: Some(filename.clone()),
                theme: None,
                image_remote_checksum: None,
                image_sha256: None,
            });

            Ok(Some(Entry::Page { title: content.title, filename }))
        }
        "Section" => {
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
                id: leaf.id,
                kind: leaf.kind.clone(),
                title: content.title.clone(),
                fingerprint: content.fingerprint,
                filename: filename.clone(),
                theme: content.theme,
                image_remote_checksum: None,
                image_sha256: None,
            });

            Ok(Some(Entry::Section { title: content.title, filename }))
        }
        "Picture" => {
            // A picture's index line carries its caption, so that's the title
            // we track; the leaf's own title stays server-side.
            let caption = content.caption.unwrap_or_default();

            let Some(image) = content.image else {
                // A picture with no image has nothing to edit locally. It stays
                // in state so we don't mistake it for a deletion, but gets no
                // file and no index line.
                state.leaves.push(LeafState {
                    id: leaf.id,
                    kind: leaf.kind.clone(),
                    title: caption,
                    fingerprint: content.fingerprint,
                    filename: None,
                    theme: None,
                    image_remote_checksum: None,
                    image_sha256: None,
                });

                return Ok(None);
            };

            let filename = claim_filename(format!("assets/{}", sanitize_filename(&image.filename)), taken);
            let bytes = client.download(&image.url)?;
            fsutil::write_atomically(&checkout.working_path(&filename), &bytes)?;

            state.leaves.push(LeafState {
                id: leaf.id,
                kind: leaf.kind.clone(),
                title: caption.clone(),
                fingerprint: content.fingerprint,
                filename: Some(filename.clone()),
                theme: None,
                image_remote_checksum: Some(image.checksum),
                image_sha256: Some(checkout::sha256(&bytes)),
            });

            Ok(Some(Entry::Picture { caption, filename }))
        }
        other => anyhow::bail!("the server sent an unknown leaf type: {other}"),
    }
}

/// Download every /u/ upload a page body references that assets/ doesn't hold yet.
pub fn mirror_uploads(client: &mut api::Client, checkout: &Checkout, state: &mut State, localized_body: &str) -> Result<()> {
    for slug in merge::referenced_assets(localized_body) {
        let path = checkout.working_path(&format!("assets/{slug}"));

        if state.uploads.contains_key(&slug) && path.exists() {
            continue;
        }

        let bytes = client.download(&format!("/u/{slug}"))?;
        fsutil::write_atomically(&path, &bytes)?;
        state.uploads.insert(slug, checkout::sha256(&bytes));
    }

    Ok(())
}

pub fn book_meta(manifest: &Manifest) -> checkout::BookMeta {
    checkout::BookMeta {
        title: manifest.book.title.clone(),
        subtitle: manifest.book.subtitle.clone().unwrap_or_default(),
        author: manifest.book.author.clone().unwrap_or_default(),
        theme: manifest.book.theme.clone(),
    }
}

/// Mirror the book cover into assets/, tracking it in state. Returns quietly
/// when the local copy was modified by hand — push owns that direction.
pub fn mirror_cover(client: &mut api::Client, checkout: &Checkout, state: &mut State, manifest: &Manifest) -> Result<()> {
    match (&manifest.book.cover, state.cover.clone()) {
        (Some(remote), current) => {
            if current.as_ref().is_some_and(|it| it.remote_checksum == remote.checksum) {
                return Ok(());
            }

            if let Some(current) = &current {
                if cover_modified_locally(checkout, current) {
                    eprintln!("warning: the cover changed both locally and remotely — keeping your copy");
                    return Ok(());
                }
            }

            let filename = format!("assets/cover-{}", sanitize_filename(&remote.filename));
            let bytes = client.download(&remote.url)?;

            if let Some(current) = &current {
                if current.filename != filename {
                    let _ = std::fs::remove_file(checkout.working_path(&current.filename));
                }
            }

            fsutil::write_atomically(&checkout.working_path(&filename), &bytes)?;

            state.cover = Some(checkout::CoverState {
                filename,
                remote_checksum: remote.checksum.clone(),
                sha256: checkout::sha256(&bytes),
            });
        }
        (None, Some(current)) => {
            if cover_modified_locally(checkout, &current) {
                eprintln!("warning: the cover was removed remotely but modified locally — keeping your copy");
            } else {
                let _ = std::fs::remove_file(checkout.working_path(&current.filename));
                state.cover = None;
            }
        }
        (None, None) => {}
    }

    Ok(())
}

fn cover_modified_locally(checkout: &Checkout, cover: &checkout::CoverState) -> bool {
    match std::fs::read(checkout.working_path(&cover.filename)) {
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

pub fn claim_filename(wanted: String, taken: &mut Vec<String>) -> String {
    let filename = checkout::unique_filename(&wanted, taken);
    taken.push(filename.clone());
    filename
}

pub fn sanitize_filename(filename: &str) -> String {
    let cleaned: String = filename
        .chars()
        .map(|it| if it.is_alphanumeric() || it == '.' || it == '-' || it == '_' { it } else { '-' })
        .collect();

    cleaned.trim_matches('-').to_string()
}
