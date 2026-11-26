//! Pull works like git: for every file changed on both sides it runs a
//! three-way merge between the base snapshot (the common ancestor), your
//! working copy, and the remote version. Clean merges just land; conflicts
//! get standard markers, and the base still advances to the remote version —
//! resolving the markers and pushing finishes the job.

use anyhow::{Result, bail};

use crate::api;
use crate::checkout::{Checkout, State};
use crate::index::{self, Entry};
use crate::leaf_kind::LeafKind;
use crate::merge;
use crate::plan;
use crate::sync::{self, ManifestLeaf, RemoteLeafContent};

pub fn run() -> Result<()> {
    let checkout = Checkout::find()?;
    let origin = checkout.origin()?;
    let state = checkout.state()?;
    let document = index::parse(&checkout.read_working("index.md")?)?;

    plan::ensure_no_conflict_markers(&checkout, &document.entries)?;

    let mut client = api::Client::connect(&origin.url)?;
    let manifest = sync::fetch_manifest(&mut client, origin.book_id)?;
    let comparison = plan::compare(&checkout, &state, &document, Some(&manifest))?;

    if comparison.changed_remotely() {
        land_remote_changes(
            &checkout,
            origin.book_id,
            state,
            client,
            &document,
            &manifest,
            &comparison,
        )
    } else {
        println!("Already up to date.");
        Ok(())
    }
}

fn land_remote_changes(
    checkout: &Checkout,
    book_id: u64,
    mut state: State,
    mut client: api::Client,
    document: &index::Document,
    manifest: &sync::Manifest,
    comparison: &plan::Comparison,
) -> Result<()> {
    let mut conflicts = Vec::new();
    let mut taken = taken_filenames(checkout, &state, &document.entries)?;

    if comparison.book_changed_remotely {
        state.book_fingerprint = manifest.book.fingerprint.clone();
    }

    let mut remote_entries = Vec::new();

    for leaf in &manifest.leaves {
        match state.leaves.iter().position(|it| it.id == leaf.id) {
            None => {
                if let Some(entry) =
                    sync::materialize_leaf(&mut client, checkout, &mut state, book_id, leaf, &mut taken)?
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
                    let content = sync::fetch_leaf(&mut client, book_id, &leaf.kind, leaf.id)?;

                    if let Some(entry) = apply_remote_change(
                        &mut client,
                        checkout,
                        &mut state,
                        position,
                        leaf,
                        content,
                        &mut taken,
                        &mut conflicts,
                    )? {
                        remote_entries.push(entry);
                    }
                }
            }
        }
    }

    carry_over_deleted_leaves(checkout, &state, comparison, &mut remote_entries)?;

    let kept_local_cover = sync::mirror_cover(&mut client, checkout, &mut state, manifest)?;

    let mut meta = sync::book_meta(manifest, &state);

    if kept_local_cover {
        meta.cover = state.cover.as_ref().map(|it| it.filename.clone());
    }

    let remote_index = index::serialize(&meta, &remote_entries);
    merge_file(checkout, "index.md", &remote_index, &mut conflicts)?;

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
        bail!(
            "merge conflicts in {} — resolve the markers, then push",
            conflicts.join(", ")
        )
    }
}

fn taken_filenames(checkout: &Checkout, state: &State, working: &[Entry]) -> Result<Vec<String>> {
    let mut taken = vec!["index.md".to_string()];

    for entry in std::fs::read_dir(&checkout.root)? {
        let entry = entry?;

        if entry.file_type()?.is_file() {
            taken.push(entry.file_name().to_string_lossy().to_string());
        }
    }

    let assets = checkout.root.join("assets");

    if assets.is_dir() {
        for entry in std::fs::read_dir(&assets)? {
            taken.push(format!("assets/{}", entry?.file_name().to_string_lossy()));
        }
    }

    taken.extend(state.leaves.iter().filter_map(|it| it.filename.clone()));
    taken.extend(working.iter().filter_map(|it| it.filename()).map(str::to_string));
    taken.sort();
    taken.dedup();
    Ok(taken)
}

fn merge_file(checkout: &Checkout, filename: &str, remote_text: &str, conflicts: &mut Vec<String>) -> Result<()> {
    let base = checkout.read_base_or_empty(filename)?;
    let ours = checkout.read_working_or_empty(filename)?;

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
    let entry = match &leaf.kind {
        LeafKind::Page => {
            let body = merge::localize_upload_urls(&content.body.clone().unwrap_or_default());
            sync::mirror_uploads(client, checkout, state, &body)?;

            let filename = state.leaves[position]
                .filename
                .clone()
                .expect("a page always has a file");
            merge_file(checkout, &filename, &body, conflicts)?;

            Some(Entry::Page {
                title: content.title.clone(),
                filename,
            })
        }
        LeafKind::Section => {
            let body = content.body.clone().unwrap_or_default();
            let filename = apply_section_body(checkout, state, position, leaf, &content, &body, taken, conflicts)?;
            state.leaves[position].theme = content.theme.clone();

            Some(Entry::Section {
                title: content.title.clone(),
                filename,
                theme: content.theme.clone(),
            })
        }
        LeafKind::Picture => {
            let caption = content.caption.clone().unwrap_or_default();
            let filename = apply_picture_image(client, checkout, state, position, &content, taken)?;

            filename.map(|filename| Entry::Picture { caption, filename })
        }
        LeafKind::Unknown(other) => bail!("the server sent an unknown leaf type: {other}"),
    };

    let leaf_state = &mut state.leaves[position];
    leaf_state.fingerprint = content.fingerprint;

    leaf_state.title = match leaf.kind {
        LeafKind::Picture => content.caption.unwrap_or_default(),
        LeafKind::Page | LeafKind::Section | LeafKind::Unknown(_) => content.title,
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
            let modified = checkout.read_working_or_empty(&filename)? != checkout.read_base_or_empty(&filename)?;

            if modified {
                eprintln!(
                    "warning: section \"{}\" lost its body remotely, but {filename} is modified locally — keeping your file",
                    content.title
                );
                Ok(Some(filename))
            } else {
                checkout.remove_working_if_present(&filename)?;
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
                let modified = match std::fs::read(checkout.working_path(&filename)?) {
                    Ok(bytes) => Some(crate::checkout::sha256(&bytes)) != current.image_sha256,
                    Err(_) => false,
                };

                if modified {
                    eprintln!("warning: {filename} changed both locally and remotely — keeping your copy");
                } else {
                    let bytes = client.download(&image.url)?;
                    crate::fsutil::write_atomically(&checkout.working_path(&filename)?, &bytes)?;
                    state.leaves[position].image_sha256 = Some(crate::checkout::sha256(&bytes));
                    state.leaves[position].image_remote_checksum = Some(image.checksum.clone());
                }
            }

            Ok(Some(filename))
        }
        (Some(image), None) => {
            let filename = sync::claim_filename(sync::picture_filename(image), taken);
            let bytes = client.download(&image.url)?;
            crate::fsutil::write_atomically(&checkout.working_path(&filename)?, &bytes)?;

            state.leaves[position].filename = Some(filename.clone());
            state.leaves[position].image_sha256 = Some(crate::checkout::sha256(&bytes));
            state.leaves[position].image_remote_checksum = Some(image.checksum.clone());
            Ok(Some(filename))
        }
        (None, Some(filename)) => {
            eprintln!(
                "warning: the image of \"{}\" was removed remotely — keeping {filename}",
                content.title.clone()
            );
            Ok(Some(filename))
        }
        (None, None) => Ok(None),
    }
}

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
                checkout.remove_working_if_present(filename)?;
            }
        }

        if let Some(filename) = &leaf.filename {
            checkout.remove_base(filename)?;
        }
    }

    Ok(())
}

fn insertion_point(state: &State, position: usize, remote_entries: &[Entry]) -> usize {
    let preceding = state.leaves[..position]
        .iter()
        .rev()
        .filter_map(|earlier| earlier.entry())
        .find_map(|entry| remote_entries.iter().position(|it| *it == entry));

    match preceding {
        Some(found) => found + 1,
        None => 0,
    }
}
