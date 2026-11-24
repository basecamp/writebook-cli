//! Pull works like git: for every file changed on both sides it runs a
//! three-way merge between the base snapshot (the common ancestor), your
//! working copy, and the remote version. Clean merges just land; conflicts
//! get standard markers, and the base still advances to the remote version —
//! resolving the markers and pushing finishes the job.

use anyhow::{Result, bail};

use crate::api;
use crate::checkout::{Checkout, State};
use crate::index::{self, Entry};
use crate::merge;
use crate::plan::{self, ManifestLeaf};
use crate::sync::{self, RemoteLeafContent};

pub fn run() -> Result<()> {
    let checkout = Checkout::find()?;
    let origin = checkout.origin()?;
    let mut state = checkout.state()?;
    let document = index::parse(&checkout.read_working("index.md")?)?;

    plan::ensure_no_conflict_markers(&checkout, &document.entries)?;

    let mut client = api::Client::connect(&origin.url)?;
    let manifest = sync::fetch_manifest(&mut client, origin.book_id)?;
    let comparison = plan::compare(&checkout, &state, &document, Some(&manifest))?;

    if !comparison.changed_remotely() {
        println!("Already up to date.");
        return Ok(());
    }

    let mut conflicts = Vec::new();
    let mut taken = plan::taken_filenames(&checkout, &state, &document.entries)?;

    if comparison.book_changed_remotely {
        state.book_fingerprint = manifest.book.fingerprint.clone();
    }

    let mut remote_entries = Vec::new();

    for leaf in &manifest.leaves {
        match state.leaves.iter().position(|it| it.id == leaf.id) {
            None => {
                if let Some(entry) =
                    sync::materialize_leaf(&mut client, &checkout, &mut state, origin.book_id, leaf, &mut taken)?
                {
                    println!("  new: {}", entry.filename().unwrap_or(entry.title()));
                    remote_entries.push(entry);
                }
            }
            Some(position) => {
                if state.leaves[position].fingerprint == leaf.fingerprint {
                    if let Some(entry) = state.leaves[position].entry() {
                        remote_entries.push(entry);
                    }
                } else {
                    let content = sync::fetch_leaf(&mut client, origin.book_id, &leaf.kind, leaf.id)?;

                    if let Some(entry) = apply_remote_change(
                        &mut client, &checkout, &mut state, position, leaf, content, &mut taken, &mut conflicts,
                    )? {
                        remote_entries.push(entry);
                    }
                }
            }
        }
    }

    carry_over_deleted_leaves(&checkout, &state, &comparison, &mut remote_entries)?;

    let kept_local_cover = sync::mirror_cover(&mut client, &checkout, &mut state, &manifest)?;

    let mut meta = sync::book_meta(&manifest, &state);

    if kept_local_cover {
        meta.cover = state.cover.as_ref().map(|it| it.filename.clone());
    }

    let remote_index = index::serialize(&meta, &remote_entries);
    merge_file(&checkout, "index.md", &remote_index, &mut conflicts)?;

    state.leaves = manifest
        .leaves
        .iter()
        .filter_map(|remote| state.leaves.iter().find(|it| it.id == remote.id).cloned())
        .collect();
    checkout.save_state(&state)?;

    if conflicts.is_empty() {
        println!("Pulled.");
        Ok(())
    } else {
        bail!("merge conflicts in {} — resolve the markers, then push", conflicts.join(", "))
    }
}

/// Land a remote text change: fast-forward when only one side moved, merge
/// when both did. Either way the base advances to the remote version, so the
/// working copy's difference from base is exactly what's left to push.
fn merge_file(checkout: &Checkout, filename: &str, remote_text: &str, conflicts: &mut Vec<String>) -> Result<()> {
    let base = checkout.read_base(filename).unwrap_or_default();
    let ours = checkout.read_working(filename).unwrap_or_default();

    if ours == base || ours == remote_text {
        checkout.write_working(filename, remote_text)?;
    } else {
        match merge::merge(&base, &ours, remote_text) {
            Ok(clean) => checkout.write_working(filename, &clean)?,
            Err(marked) => {
                checkout.write_working(filename, &marked)?;
                conflicts.push(filename.to_string());
            }
        }
    }

    checkout.write_base(filename, remote_text)
}

fn apply_remote_change(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    position: usize,
    leaf: &ManifestLeaf,
    content: RemoteLeafContent,
    taken: &mut Vec<String>,
    conflicts: &mut Vec<String>,
) -> Result<Option<Entry>> {
    let entry = match leaf.kind.as_str() {
        "Page" => {
            let body = merge::localize_upload_urls(&content.body.clone().unwrap_or_default());
            sync::mirror_uploads(client, checkout, state, &body)?;

            let filename = state.leaves[position].filename.clone().expect("a page always has a file");
            merge_file(checkout, &filename, &body, conflicts)?;

            Some(Entry::Page { title: content.title.clone(), filename })
        }
        "Section" => {
            let body = content.body.clone().unwrap_or_default();
            let filename = apply_section_body(checkout, state, position, leaf, &content, &body, taken, conflicts)?;
            state.leaves[position].theme = content.theme.clone();

            Some(Entry::Section { title: content.title.clone(), filename, theme: content.theme.clone() })
        }
        "Picture" => {
            let caption = content.caption.clone().unwrap_or_default();
            let filename = apply_picture_image(client, checkout, state, position, &content, taken)?;

            filename.map(|filename| Entry::Picture { caption, filename })
        }
        other => bail!("the server sent an unknown leaf type: {other}"),
    };

    let leaf_state = &mut state.leaves[position];
    leaf_state.fingerprint = content.fingerprint;

    leaf_state.title = match leaf.kind.as_str() {
        "Picture" => content.caption.unwrap_or_default(),
        _ => content.title,
    };

    Ok(entry)
}

fn apply_section_body(
    checkout: &Checkout,
    state: &mut State,
    position: usize,
    leaf: &ManifestLeaf,
    content: &RemoteLeafContent,
    body: &str,
    taken: &mut Vec<String>,
    conflicts: &mut Vec<String>,
) -> Result<Option<String>> {
    match (body.is_empty(), state.leaves[position].filename.clone()) {
        (false, Some(filename)) => {
            merge_file(checkout, &filename, body, conflicts)?;
            Ok(Some(filename))
        }
        (false, None) => {
            let filename = sync::claim_filename(sync::page_filename(&content.title, leaf.id), taken);
            checkout.write_working(&filename, body)?;
            checkout.write_base(&filename, body)?;
            state.leaves[position].filename = Some(filename.clone());
            Ok(Some(filename))
        }
        (true, Some(filename)) => {
            let modified = checkout.read_working(&filename).unwrap_or_default()
                != checkout.read_base(&filename).unwrap_or_default();

            if modified {
                eprintln!("warning: section \"{}\" lost its body remotely, but {filename} is modified locally — keeping your file", content.title);
                Ok(Some(filename))
            } else {
                let _ = std::fs::remove_file(checkout.working_path(&filename));
                checkout.remove_base(&filename)?;
                state.leaves[position].filename = None;
                Ok(None)
            }
        }
        (true, None) => Ok(None),
    }
}

fn apply_picture_image(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    position: usize,
    content: &RemoteLeafContent,
    taken: &mut Vec<String>,
) -> Result<Option<String>> {
    let current = state.leaves[position].clone();

    match (&content.image, current.filename) {
        (Some(image), Some(filename)) => {
            if Some(&image.checksum) != current.image_remote_checksum.as_ref() {
                let modified = match std::fs::read(checkout.working_path(&filename)) {
                    Ok(bytes) => Some(crate::checkout::sha256(&bytes)) != current.image_sha256,
                    Err(_) => false,
                };

                if modified {
                    eprintln!("warning: {filename} changed both locally and remotely — keeping your copy");
                } else {
                    let bytes = client.download(&image.url)?;
                    crate::fsutil::write_atomically(&checkout.working_path(&filename), &bytes)?;
                    state.leaves[position].image_sha256 = Some(crate::checkout::sha256(&bytes));
                    state.leaves[position].image_remote_checksum = Some(image.checksum.clone());
                }
            }

            Ok(Some(filename))
        }
        (Some(image), None) => {
            let filename = sync::claim_filename(format!("assets/{}", sync::sanitize_filename(&image.filename)), taken);
            let bytes = client.download(&image.url)?;
            crate::fsutil::write_atomically(&checkout.working_path(&filename), &bytes)?;

            state.leaves[position].filename = Some(filename.clone());
            state.leaves[position].image_sha256 = Some(crate::checkout::sha256(&bytes));
            state.leaves[position].image_remote_checksum = Some(image.checksum.clone());
            Ok(Some(filename))
        }
        (None, Some(filename)) => {
            eprintln!("warning: the image of \"{}\" was removed remotely — keeping {filename}", content.title.clone());
            Ok(Some(filename))
        }
        (None, None) => Ok(None),
    }
}

/// Leaves deleted remotely: an untouched one takes its files and index line
/// with it (the merged index simply loses the line); a locally modified one
/// keeps its file and line, and push will re-create it as a new leaf.
fn carry_over_deleted_leaves(
    checkout: &Checkout,
    state: &State,
    comparison: &plan::Comparison,
    remote_entries: &mut Vec<Entry>,
) -> Result<()> {
    for (position, leaf) in state.leaves.iter().enumerate() {
        let Some(compared) = comparison.leaves.iter().find(|it| it.state.id == leaf.id) else {
            continue;
        };

        if !compared.deleted_remotely {
            continue;
        }

        if compared.changed_locally {
            eprintln!(
                "warning: \"{}\" was deleted remotely but modified locally — keeping it; push will re-create it",
                leaf.title
            );

            if let Some(entry) = compared.entry.clone() {
                let insert_at = insertion_point(state, position, remote_entries);
                remote_entries.insert(insert_at, entry);
            }
        } else {
            if let Some(filename) = &leaf.filename {
                let _ = std::fs::remove_file(checkout.working_path(filename));
            }
        }

        if let Some(filename) = &leaf.filename {
            checkout.remove_base(filename)?;
        }
    }

    Ok(())
}

/// Keep a carried-over line near its old neighbors: right after the closest
/// preceding leaf that still has a line in the merged index.
fn insertion_point(state: &State, position: usize, remote_entries: &[Entry]) -> usize {
    for earlier in state.leaves[..position].iter().rev() {
        if let Some(entry) = earlier.entry() {
            if let Some(found) = remote_entries.iter().position(|it| *it == entry) {
                return found + 1;
            }
        }
    }

    0
}
