//! The classification engine behind status, pull, and push: given the base
//! snapshot, the working files, and (when online) the remote manifest, work
//! out what changed on which side.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::checkout::{Checkout, LeafState, State};
use crate::index::{Document, Entry};

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
    pub kind: String,
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

/// One known leaf, seen from all three sides.
pub struct LeafComparison {
    pub state: LeafState,
    pub entry: Option<Entry>,
    pub changed_locally: bool,
    pub changed_remotely: bool,
    pub deleted_remotely: bool,
}

impl LeafComparison {
    pub fn deleted_locally(&self) -> bool {
        self.entry.is_none()
    }

    pub fn conflicted(&self) -> bool {
        let local = self.changed_locally || self.deleted_locally();
        let remote = self.changed_remotely || self.deleted_remotely;
        local && remote
    }
}

pub struct Comparison {
    pub leaves: Vec<LeafComparison>,
    pub added_locally: Vec<Entry>,
    pub added_remotely: Vec<ManifestLeaf>,
    pub order_changed_locally: bool,
    pub order_changed_remotely: bool,
    pub book_changed_locally: bool,
    pub book_changed_remotely: bool,
    pub cover_changed_locally: bool,
}

impl Comparison {
    pub fn changed_locally(&self) -> bool {
        self.book_changed_locally
            || self.cover_changed_locally
            || self.order_changed_locally
            || !self.added_locally.is_empty()
            || self.leaves.iter().any(|it| it.changed_locally || it.deleted_locally())
    }

    pub fn changed_remotely(&self) -> bool {
        self.book_changed_remotely
            || self.order_changed_remotely
            || !self.added_remotely.is_empty()
            || self.leaves.iter().any(|it| it.changed_remotely || it.deleted_remotely)
    }
}

pub fn compare(
    checkout: &Checkout,
    state: &State,
    document: &Document,
    manifest: Option<&Manifest>,
) -> Result<Comparison> {
    let working = &document.entries;
    ensure_wellformed(checkout, working)?;
    ensure_cover_exists(checkout, document)?;

    let base_meta = crate::index::parse(&checkout.read_base("index.md")?)?.meta;

    let claims = claim_entries(state, working, checkout)?;

    let mut leaves = Vec::new();

    for (position, leaf) in state.leaves.iter().enumerate() {
        let entry = claims[position].clone();

        let (deleted_remotely, changed_remotely) = match manifest {
            Some(manifest) => match manifest.leaves.iter().find(|it| it.id == leaf.id) {
                Some(remote) => (false, remote.fingerprint != leaf.fingerprint),
                None => (true, false),
            },
            None => (false, false),
        };

        let changed_locally = match &entry {
            Some(entry) => entry_changed(checkout, leaf, entry)?,
            None => false,
        };

        leaves.push(LeafComparison {
            changed_locally,
            changed_remotely,
            deleted_remotely,
            state: leaf.clone(),
            entry,
        });
    }

    let added_locally = working
        .iter()
        .filter(|entry| !claims.iter().flatten().any(|claimed| claimed == *entry))
        .cloned()
        .collect();

    let added_remotely = match manifest {
        Some(manifest) => manifest
            .leaves
            .iter()
            .filter(|leaf| !state.leaves.iter().any(|known| known.id == leaf.id))
            .cloned()
            .collect(),
        None => Vec::new(),
    };

    let base_order: Vec<u64> = state.leaves.iter().map(|it| it.id).collect();

    let local_order: Vec<u64> = working
        .iter()
        .filter_map(|entry| {
            claims
                .iter()
                .position(|claimed| claimed.as_ref() == Some(entry))
                .map(|position| state.leaves[position].id)
        })
        .collect();

    let remote_order: Vec<u64> = match manifest {
        Some(manifest) => manifest.leaves.iter().map(|it| it.id).collect(),
        None => base_order.clone(),
    };

    let surviving_base_local: Vec<u64> = base_order
        .iter()
        .copied()
        .filter(|id| local_order.contains(id))
        .collect();
    let surviving_base_remote: Vec<u64> = base_order
        .iter()
        .copied()
        .filter(|id| remote_order.contains(id))
        .collect();
    let surviving_remote: Vec<u64> = remote_order
        .iter()
        .copied()
        .filter(|id| base_order.contains(id))
        .collect();

    Ok(Comparison {
        leaves,
        added_locally,
        added_remotely,
        order_changed_locally: local_order != surviving_base_local,
        order_changed_remotely: manifest.is_some() && surviving_remote != surviving_base_remote,
        book_changed_locally: document.meta.without_cover() != base_meta.without_cover(),
        book_changed_remotely: manifest.is_some_and(|it| it.book.fingerprint != state.book_fingerprint),
        cover_changed_locally: document.meta.cover != base_meta.cover || cover_file_changed(checkout, state),
    })
}

/// Refuse to sync while unresolved merge markers sit in any working text file.
pub fn ensure_no_conflict_markers(checkout: &Checkout, working: &[Entry]) -> Result<()> {
    let mut filenames = vec!["index.md".to_string()];
    filenames.extend(
        working
            .iter()
            .filter_map(|it| it.filename())
            .filter(|it| it.ends_with(".md"))
            .map(str::to_string),
    );

    for filename in filenames {
        let contents = checkout.read_working_or_empty(&filename)?;

        if contents.lines().any(|it| it.starts_with("<<<<<<<")) {
            bail!("{filename} still contains merge conflict markers — resolve them first");
        }
    }

    Ok(())
}

fn ensure_wellformed(checkout: &Checkout, working: &[Entry]) -> Result<()> {
    let mut seen = Vec::new();

    for entry in working {
        if let Some(filename) = entry.filename() {
            if seen.contains(&filename) {
                bail!("index.md lists {filename} more than once");
            }

            seen.push(filename);

            if !checkout.working_path(filename)?.exists() {
                bail!("index.md references {filename}, which does not exist");
            }
        }
    }

    Ok(())
}

fn ensure_cover_exists(checkout: &Checkout, document: &Document) -> Result<()> {
    if let Some(cover) = document.meta.cover.as_deref().filter(|it| !it.is_empty()) {
        if !checkout.working_path(cover)?.is_file() {
            bail!("index.md names {cover} as the cover, which is not a file");
        }
    }

    Ok(())
}

fn cover_file_changed(checkout: &Checkout, state: &State) -> bool {
    match &state.cover {
        Some(cover) => match checkout.read_working_bytes(&cover.filename) {
            Ok(bytes) => crate::checkout::sha256(&bytes) != cover.sha256,
            Err(_) => true,
        },
        None => false,
    }
}

fn claim_entries(state: &State, working: &[Entry], checkout: &Checkout) -> Result<Vec<Option<Entry>>> {
    let mut claims: Vec<Option<Entry>> = vec![None; state.leaves.len()];
    let mut claimed_entries: Vec<&Entry> = Vec::new();

    for (position, leaf) in state.leaves.iter().enumerate() {
        if let Some(filename) = &leaf.filename {
            if let Some(entry) = working.iter().find(|it| {
                it.kind() == leaf.kind && it.filename() == Some(filename.as_str()) && !claimed_entries.contains(it)
            }) {
                claims[position] = Some(entry.clone());
                claimed_entries.push(entry);
            }
        }
    }

    for (position, leaf) in state.leaves.iter().enumerate() {
        if claims[position].is_none() && leaf.kind == "Section" && leaf.filename.is_none() {
            if let Some(entry) = working.iter().find(|it| {
                matches!(it, Entry::Section { filename: None, .. })
                    && it.title() == leaf.title
                    && !claimed_entries.contains(it)
            }) {
                claims[position] = Some(entry.clone());
                claimed_entries.push(entry);
            }
        }
    }

    for (position, leaf) in state.leaves.iter().enumerate() {
        if claims[position].is_none() && leaf.kind == "Page" {
            if let Some(base_filename) = &leaf.filename {
                let base_content = checkout.read_base_or_empty(base_filename)?;

                if let Some(entry) = working.iter().find(|it| {
                    matches!(it, Entry::Page { .. })
                        && !claimed_entries.contains(it)
                        && it.filename().is_some_and(|filename| {
                            checkout
                                .read_working(filename)
                                .is_ok_and(|content| content == base_content)
                        })
                }) {
                    claims[position] = Some(entry.clone());
                    claimed_entries.push(entry);
                }
            }
        }
    }

    Ok(claims)
}

fn entry_changed(checkout: &Checkout, leaf: &LeafState, entry: &Entry) -> Result<bool> {
    let theme_changed = match entry {
        Entry::Section { theme: Some(theme), .. } => leaf.theme.as_ref() != Some(theme),
        _ => false,
    };

    if entry.title() != leaf.title || theme_changed {
        Ok(true)
    } else if let Entry::Picture { filename, .. } = entry {
        Ok(picture_image_changed(checkout, leaf, filename))
    } else {
        match entry.filename() {
            Some(filename) => {
                let working = checkout.read_working_or_empty(filename)?;
                let base = match &leaf.filename {
                    Some(base_filename) => checkout.read_base_or_empty(base_filename)?,
                    None => String::new(),
                };
                Ok(working != base)
            }
            None => Ok(leaf.filename.is_some()),
        }
    }
}

fn picture_image_changed(checkout: &Checkout, leaf: &LeafState, filename: &str) -> bool {
    match (&leaf.image_sha256, checkout.read_working_bytes(filename)) {
        (Some(synced), Ok(bytes)) => crate::checkout::sha256(&bytes) != *synced,
        (None, Ok(_)) => true,
        (_, Err(_)) => false,
    }
}

#[cfg(test)]
mod tests;
