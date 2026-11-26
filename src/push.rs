//! Push refuses to run when the remote moved since the last sync (pull first,
//! like git), then applies local changes in a safe order: create new leaves,
//! update changed ones (each guarded by its base fingerprint, so a concurrent
//! web edit answers 409 instead of being clobbered), trash deleted ones, fix
//! the order with one moves call, and finally sync book metadata and cover.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::api::{self, ApiError, Part, PartData};
use crate::checkout::{self, Checkout, LeafState, State};
use crate::creds;
use crate::index::{self, BookMeta, Entry};
use crate::merge;
use crate::plan;
use crate::sync;

pub fn run() -> Result<()> {
    let checkout = Checkout::find()?;
    let origin = checkout.origin()?;
    creds::ensure_can_write(&origin.url)?;
    let state = checkout.state()?;
    let document = index::parse(&checkout.read_working("index.md")?)?;

    plan::ensure_no_conflict_markers(&checkout, &document.entries)?;

    let mut client = api::Client::connect(&origin.url)?;
    let manifest = sync::fetch_manifest(&mut client, origin.book_id)?;
    let comparison = plan::compare(&checkout, &state, &document, Some(&manifest))?;

    if comparison.changed_remotely() {
        bail!("the remote has changes you don't have yet — run `writebook pull` first");
    }

    if comparison.changed_locally() {
        apply_local_changes(&checkout, origin.book_id, state, client, &document, &comparison)
    } else {
        println!("Nothing to push.");
        Ok(())
    }
}

fn apply_local_changes(
    checkout: &Checkout,
    book_id: u64,
    mut state: State,
    mut client: api::Client,
    document: &index::Document,
    comparison: &plan::Comparison,
) -> Result<()> {
    let mut created: Vec<(Entry, u64)> = Vec::new();

    for entry in &comparison.added_locally {
        let id = create_leaf(&mut client, checkout, &mut state, book_id, entry)?;
        println!("  created: {}", entry.filename().unwrap_or(entry.title()));
        created.push((entry.clone(), id));
    }

    for compared in &comparison.leaves {
        if compared.changed_locally {
            let entry = compared.entry.clone().expect("a changed leaf still has its entry");
            update_leaf(&mut client, checkout, &mut state, book_id, &compared.state, &entry)?;
            println!("  updated: {}", entry.filename().unwrap_or(entry.title()));
        }
    }

    for compared in &comparison.leaves {
        if compared.deleted_locally() {
            client.delete(&sync::leaf_path(book_id, &compared.state.kind, compared.state.id))?;
            println!("  trashed: {}", compared.state.title);

            if let Some(filename) = &compared.state.filename {
                checkout.remove_base(filename)?;
            }

            state.leaves.retain(|it| it.id != compared.state.id);
        }
    }

    reorder(
        &mut client,
        &mut state,
        book_id,
        &document.entries,
        comparison,
        &created,
    )?;

    if comparison.book_changed_locally {
        push_book_metadata(&mut client, &mut state, book_id, &document.meta)?;
    }

    push_cover(
        &mut client,
        checkout,
        &mut state,
        book_id,
        document.meta.cover.as_deref(),
    )?;

    checkout.write_base("index.md", &checkout.read_working("index.md")?)?;
    checkout.save_state(&state)?;

    println!("Pushed.");
    Ok(())
}

fn create_leaf(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    book_id: u64,
    entry: &Entry,
) -> Result<u64> {
    match entry {
        Entry::Page { title, filename } => {
            let response = client.post(
                &format!("/books/{book_id}/pages.json"),
                &json!({ "leaf": { "title": title }, "page": { "body": "" } }),
            )?;

            let id = leaf_id(&response)?;
            let mut leaf = LeafState {
                id,
                kind: "Page".to_string(),
                title: title.clone(),
                fingerprint: fingerprint(&response),
                filename: Some(filename.clone()),
                theme: None,
                image_remote_checksum: None,
                image_sha256: None,
            };

            push_page_body(client, checkout, state, book_id, &mut leaf, entry)?;
            state.leaves.push(leaf);
            Ok(id)
        }
        Entry::Section { title, filename, theme } => {
            let body = match filename {
                Some(filename) => checkout.read_working(filename)?,
                None => String::new(),
            };

            let response = client.post(
                &format!("/books/{book_id}/sections.json"),
                &json!({ "leaf": { "title": title }, "section": section_params(&body, theme) }),
            )?;

            if let Some(filename) = filename {
                checkout.write_base(filename, &body)?;
            }

            let id = leaf_id(&response)?;
            state.leaves.push(LeafState {
                id,
                kind: "Section".to_string(),
                title: title.clone(),
                fingerprint: fingerprint(&response),
                filename: filename.clone(),
                theme: response["theme"].as_str().map(str::to_string),
                image_remote_checksum: None,
                image_sha256: None,
            });
            Ok(id)
        }
        Entry::Picture { caption, filename } => {
            let bytes = std::fs::read(checkout.working_path(filename)?)
                .with_context(|| format!("could not read {filename}"))?;

            let response = client.post_multipart(
                &format!("/books/{book_id}/pictures.json"),
                &picture_parts(caption, filename, &bytes),
            )?;

            let id = leaf_id(&response)?;
            state.leaves.push(LeafState {
                id,
                kind: "Picture".to_string(),
                title: caption.clone(),
                fingerprint: fingerprint(&response),
                filename: Some(filename.clone()),
                theme: None,
                image_remote_checksum: response["image"]["checksum"].as_str().map(str::to_string),
                image_sha256: Some(checkout::sha256(&bytes)),
            });
            Ok(id)
        }
    }
}

fn update_leaf(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    book_id: u64,
    known: &LeafState,
    entry: &Entry,
) -> Result<()> {
    match entry {
        Entry::Page { .. } => {
            let mut leaf = known.clone();
            leaf.title = entry.title().to_string();
            leaf.filename = entry.filename().map(str::to_string);
            push_page_body(client, checkout, state, book_id, &mut leaf, entry)?;
            update_state_leaf(state, leaf);
        }
        Entry::Section { title, filename, theme } => {
            let body = match filename {
                Some(filename) => checkout.read_working(filename)?,
                None => String::new(),
            };

            let response = guarded_patch(
                client,
                &sync::leaf_path(book_id, "Section", known.id),
                &json!({
                    "leaf": { "title": title },
                    "section": section_params(&body, theme),
                    "base_fingerprint": known.fingerprint,
                }),
            )?;

            if let Some(filename) = filename {
                checkout.write_base(filename, &body)?;
            }

            let mut leaf = known.clone();
            leaf.title = title.clone();
            leaf.filename = filename.clone();
            leaf.fingerprint = fingerprint(&response);
            leaf.theme = response["theme"].as_str().map(str::to_string).or(leaf.theme);
            update_state_leaf(state, leaf);
        }
        Entry::Picture { caption, filename } => {
            let bytes = std::fs::read(checkout.working_path(filename)?)
                .with_context(|| format!("could not read {filename}"))?;
            let image_changed = Some(checkout::sha256(&bytes)) != known.image_sha256;

            let response = if image_changed {
                let mut parts = picture_parts(caption, filename, &bytes);
                parts.push(Part {
                    name: "base_fingerprint",
                    filename: None,
                    data: PartData::Text(&known.fingerprint),
                });

                guarded(client.patch_multipart(&sync::leaf_path(book_id, "Picture", known.id), &parts))?
            } else {
                guarded_patch(
                    client,
                    &sync::leaf_path(book_id, "Picture", known.id),
                    &json!({
                        "leaf": { "title": caption },
                        "picture": { "caption": caption },
                        "base_fingerprint": known.fingerprint,
                    }),
                )?
            };

            let mut leaf = known.clone();
            leaf.title = caption.clone();
            leaf.filename = Some(filename.clone());
            leaf.fingerprint = fingerprint(&response);
            leaf.image_remote_checksum = response["image"]["checksum"]
                .as_str()
                .map(str::to_string)
                .or(leaf.image_remote_checksum);
            leaf.image_sha256 = Some(checkout::sha256(&bytes));
            update_state_leaf(state, leaf);
        }
    }

    Ok(())
}

fn picture_parts<'a>(caption: &'a str, filename: &'a str, bytes: &'a [u8]) -> Vec<Part<'a>> {
    vec![
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
            filename: Some(filename.strip_prefix("assets/").unwrap_or(filename)),
            data: PartData::Bytes(bytes),
        },
    ]
}

fn section_params(body: &str, theme: &Option<String>) -> serde_json::Value {
    match theme {
        Some(theme) => json!({ "body": body, "theme": theme }),
        None => json!({ "body": body }),
    }
}

fn push_page_body(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    book_id: u64,
    leaf: &mut LeafState,
    entry: &Entry,
) -> Result<()> {
    let filename = entry.filename().expect("a page always has a file").to_string();
    let mut body = checkout.read_working(&filename)?;

    let shown = client.get(&sync::leaf_path(book_id, "Page", leaf.id))?;
    let record_gid = shown["record_gid"]
        .as_str()
        .context("the server sent no upload gid")?
        .to_string();

    let mut renamed = false;

    for name in merge::referenced_assets(&body) {
        let path = checkout.working_path(&format!("assets/{name}"))?;
        let bytes = std::fs::read(&path).with_context(|| format!("could not read assets/{name}"))?;
        let sha256 = checkout::sha256(&bytes);

        if state.uploads.get(&name) == Some(&sha256) {
            continue;
        }

        let response = client.post_multipart(
            "/uploads",
            &[
                Part {
                    name: "record_gid",
                    filename: None,
                    data: PartData::Text(&record_gid),
                },
                Part {
                    name: "attribute_name",
                    filename: None,
                    data: PartData::Text("body"),
                },
                Part {
                    name: "file",
                    filename: Some(&name),
                    data: PartData::Bytes(&bytes),
                },
            ],
        )?;

        let slug = response["fileUrl"]
            .as_str()
            .and_then(|it| it.strip_prefix("/u/"))
            .context("the server's upload response had no /u/ URL")?
            .to_string();

        if slug != name {
            let target = checkout.working_path(&format!("assets/{slug}"))?;

            if index_uses(checkout, &format!("assets/{name}"))? {
                std::fs::copy(&path, &target).with_context(|| format!("could not copy assets/{name}"))?;
            } else {
                std::fs::rename(&path, &target).with_context(|| format!("could not rename assets/{name}"))?;
            }

            body = body.replace(&format!("](assets/{name})"), &format!("](assets/{slug})"));
            renamed = true;
            state.uploads.remove(&name);
        }

        state.uploads.insert(slug, sha256);
    }

    if renamed {
        checkout.write_working(&filename, &body)?;
    }

    let response = guarded_patch(
        client,
        &sync::leaf_path(book_id, "Page", leaf.id),
        &json!({
            "leaf": { "title": leaf.title },
            "page": { "body": merge::globalize_upload_urls(&body) },
            "base_fingerprint": leaf.fingerprint,
        }),
    )?;

    checkout.write_base(&filename, &body)?;
    leaf.fingerprint = fingerprint(&response);
    Ok(())
}

fn index_uses(checkout: &Checkout, filename: &str) -> Result<bool> {
    let document = index::parse(&checkout.read_working("index.md")?)?;
    Ok(document.meta.cover.as_deref() == Some(filename)
        || document.entries.iter().any(|it| it.filename() == Some(filename)))
}

fn reorder(
    client: &mut api::Client,
    state: &mut State,
    book_id: u64,
    working: &[Entry],
    comparison: &plan::Comparison,
    created: &[(Entry, u64)],
) -> Result<()> {
    let mut used: Vec<u64> = Vec::new();
    let mut desired: Vec<u64> = Vec::new();

    for entry in working {
        let id = comparison
            .leaves
            .iter()
            .find(|it| it.entry.as_ref() == Some(entry) && !used.contains(&it.state.id))
            .map(|it| it.state.id)
            .or_else(|| {
                created
                    .iter()
                    .find(|(created_entry, id)| created_entry == entry && !used.contains(id))
                    .map(|(_, id)| *id)
            });

        if let Some(id) = id {
            used.push(id);
            desired.push(id);
        }
    }

    let synced: Vec<u64> = state
        .leaves
        .iter()
        .map(|it| it.id)
        .filter(|it| desired.contains(it))
        .collect();

    if desired != synced {
        client.post(
            &format!("/books/{book_id}/leaves/moves.json"),
            &json!({ "id": desired, "position": 0 }),
        )?;
        println!("  reordered");
    }

    state
        .leaves
        .sort_by_key(|leaf| desired.iter().position(|it| *it == leaf.id).unwrap_or(usize::MAX));
    Ok(())
}

fn push_book_metadata(client: &mut api::Client, state: &mut State, book_id: u64, meta: &BookMeta) -> Result<()> {
    let response = client.patch(
        &format!("/books/{book_id}.json"),
        &json!({
            "book": {
                "title": meta.title,
                "subtitle": meta.subtitle,
                "author": meta.author,
                "theme": meta.theme,
            }
        }),
    )?;

    state.book_fingerprint = fingerprint(&response);
    println!("  updated: book metadata");
    Ok(())
}

fn push_cover(
    client: &mut api::Client,
    checkout: &Checkout,
    state: &mut State,
    book_id: u64,
    cover: Option<&str>,
) -> Result<()> {
    match cover {
        Some("") => {
            if state.cover.is_some() {
                remove_cover(client, state, book_id)?;
            }
        }
        Some(filename) => {
            let bytes = std::fs::read(checkout.working_path(filename)?)
                .with_context(|| format!("could not read the cover, {filename}"))?;
            upload_cover(client, state, book_id, filename, &bytes)?;
        }
        None => {
            if let Some(tracked) = state.cover.clone() {
                match std::fs::read(checkout.working_path(&tracked.filename)?) {
                    Ok(bytes) => upload_cover(client, state, book_id, &tracked.filename, &bytes)?,
                    Err(_) => remove_cover(client, state, book_id)?,
                }
            }
        }
    }

    Ok(())
}

fn upload_cover(client: &mut api::Client, state: &mut State, book_id: u64, filename: &str, bytes: &[u8]) -> Result<()> {
    let sha256 = checkout::sha256(bytes);

    let unchanged = state
        .cover
        .as_ref()
        .is_some_and(|it| it.filename == filename && it.sha256 == sha256);

    if unchanged {
        Ok(())
    } else {
        let basename = filename.strip_prefix("assets/").unwrap_or(filename);
        let response = client.patch_multipart(
            &format!("/books/{book_id}.json"),
            &[Part {
                name: "book[cover]",
                filename: Some(basename),
                data: PartData::Bytes(bytes),
            }],
        )?;

        state.cover = Some(checkout::CoverState {
            filename: filename.to_string(),
            remote_checksum: String::new(),
            sha256,
        });
        record_remote_cover(client, state, book_id, &fingerprint(&response))?;
        println!("  updated: cover");
        Ok(())
    }
}

fn remove_cover(client: &mut api::Client, state: &mut State, book_id: u64) -> Result<()> {
    let response = client.patch(&format!("/books/{book_id}.json"), &json!({ "remove_cover": "true" }))?;
    state.cover = None;
    record_remote_cover(client, state, book_id, &fingerprint(&response))?;
    println!("  removed: cover");
    Ok(())
}

fn record_remote_cover(
    client: &mut api::Client,
    state: &mut State,
    book_id: u64,
    book_fingerprint: &str,
) -> Result<()> {
    state.book_fingerprint = book_fingerprint.to_string();
    let manifest = sync::fetch_manifest(client, book_id)?;

    if manifest.book.fingerprint == book_fingerprint {
        if let (Some(cover), Some(remote)) = (state.cover.as_mut(), manifest.book.cover) {
            cover.remote_checksum = remote.checksum;
        }
    }

    Ok(())
}

fn guarded_patch(client: &mut api::Client, path: &str, body: &Value) -> Result<Value> {
    guarded(client.patch(path, body))
}

fn guarded(response: Result<Value>) -> Result<Value> {
    match response {
        Err(error) if error.downcast_ref::<ApiError>().is_some_and(|it| it.status == 409) => {
            bail!("the remote changed while pushing — run `writebook pull`, then push again")
        }
        other => other,
    }
}

fn update_state_leaf(state: &mut State, leaf: LeafState) {
    let synced = state
        .leaves
        .iter_mut()
        .find(|it| it.id == leaf.id)
        .expect("an updated leaf is one the checkout already tracks");
    *synced = leaf;
}

fn leaf_id(response: &Value) -> Result<u64> {
    response["id"].as_u64().context("the server's response had no leaf id")
}

fn fingerprint(response: &Value) -> String {
    response["fingerprint"].as_str().unwrap_or_default().to_string()
}
