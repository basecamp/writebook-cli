use anyhow::Result;

use crate::api;
use crate::checkout::Checkout;
use crate::index::{self, Entry};
use crate::merge;
use crate::plan;
use crate::sync;

pub fn run(remote: bool) -> Result<()> {
    if remote {
        diff_remote()
    } else {
        diff_local()
    }
}

/// What you changed since the last sync: base snapshot on the left, working
/// files on the right.
fn diff_local() -> Result<()> {
    let checkout = Checkout::find()?;
    let state = checkout.state()?;
    let document = index::parse(&checkout.read_working("index.md")?)?;
    let comparison = plan::compare(&checkout, &state, &document, None)?;

    print_diff("index.md", &checkout.read_base("index.md")?, &checkout.read_working("index.md")?);

    for compared in &comparison.leaves {
        let Some(filename) = &compared.state.filename else {
            continue;
        };

        if !filename.ends_with(".md") {
            if compared.changed_locally {
                println!("binary file {filename} changed");
            }
            continue;
        }

        let base = checkout.read_base(filename).unwrap_or_default();

        let current = match &compared.entry {
            Some(entry) => match entry.filename() {
                Some(current_filename) => checkout.read_working(current_filename).unwrap_or_default(),
                None => String::new(),
            },
            None => String::new(),
        };

        print_diff(filename, &base, &current);
    }

    for entry in &comparison.added_locally {
        if let Some(filename) = entry.filename() {
            if filename.ends_with(".md") {
                print_diff(filename, "", &checkout.read_working(filename).unwrap_or_default());
            } else {
                println!("binary file {filename} added");
            }
        }
    }

    Ok(())
}

/// What the remote changed since the last sync: base snapshot on the left,
/// remote content on the right.
fn diff_remote() -> Result<()> {
    let checkout = Checkout::find()?;
    let origin = checkout.origin()?;
    let state = checkout.state()?;
    let document = index::parse(&checkout.read_working("index.md")?)?;

    let mut client = api::Client::connect(&origin.url)?;
    let manifest = sync::fetch_manifest(&mut client, origin.book_id)?;
    let comparison = plan::compare(&checkout, &state, &document, Some(&manifest))?;

    let mut remote_entries = Vec::new();

    for leaf in &manifest.leaves {
        match state.leaves.iter().find(|it| it.id == leaf.id) {
            None => {
                let content = sync::fetch_leaf(&mut client, origin.book_id, &leaf.kind, leaf.id)?;
                let filename = sync::page_filename(&leaf.title, leaf.id);

                match leaf.kind.as_str() {
                    "Page" => {
                        let body = merge::localize_upload_urls(&content.body.unwrap_or_default());
                        print_diff(&filename, "", &body);
                        remote_entries.push(Entry::Page { title: content.title, filename });
                    }
                    "Section" => {
                        let body = content.body.unwrap_or_default();
                        let filename = if body.is_empty() { None } else { Some(filename) };

                        if let Some(filename) = &filename {
                            print_diff(filename, "", &body);
                        }

                        remote_entries.push(Entry::Section { title: content.title, filename, theme: content.theme });
                    }
                    "Picture" => {
                        if let Some(image) = &content.image {
                            println!("binary file assets/{} added", sync::sanitize_filename(&image.filename));
                            remote_entries.push(Entry::Picture {
                                caption: content.caption.unwrap_or_default(),
                                filename: format!("assets/{}", sync::sanitize_filename(&image.filename)),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Some(known) => {
                let mut entry = known.entry();

                if known.fingerprint != leaf.fingerprint {
                    let content = sync::fetch_leaf(&mut client, origin.book_id, &leaf.kind, leaf.id)?;

                    match (known.kind.as_str(), &known.filename) {
                        ("Page", Some(filename)) => {
                            let body = merge::localize_upload_urls(&content.body.clone().unwrap_or_default());
                            print_diff(filename, &checkout.read_base(filename).unwrap_or_default(), &body);
                        }
                        ("Section", Some(filename)) => {
                            let body = content.body.clone().unwrap_or_default();
                            print_diff(filename, &checkout.read_base(filename).unwrap_or_default(), &body);
                        }
                        ("Picture", Some(filename)) => {
                            if content.image.as_ref().map(|it| &it.checksum) != known.image_remote_checksum.as_ref() {
                                println!("binary file {filename} changed");
                            }
                        }
                        _ => {}
                    }

                    entry = remote_entry(&known.kind, &content, known.filename.clone());
                }

                if let Some(entry) = entry {
                    remote_entries.push(entry);
                }
            }
        }
    }

    print_diff("index.md", &checkout.read_base("index.md")?, &index::serialize(&sync::book_meta(&manifest, &state), &remote_entries));

    for compared in &comparison.leaves {
        if compared.deleted_remotely {
            if let Some(filename) = &compared.state.filename {
                if filename.ends_with(".md") {
                    print_diff(filename, &checkout.read_base(filename).unwrap_or_default(), "");
                } else {
                    println!("binary file {filename} deleted");
                }
            }
        }
    }

    Ok(())
}

fn remote_entry(kind: &str, content: &sync::RemoteLeafContent, filename: Option<String>) -> Option<Entry> {
    match kind {
        "Page" => filename.map(|filename| Entry::Page { title: content.title.clone(), filename }),
        "Section" => Some(Entry::Section {
            title: content.title.clone(),
            filename: if content.body.as_deref().unwrap_or_default().is_empty() { None } else { filename },
            theme: content.theme.clone(),
        }),
        "Picture" => filename.map(|filename| Entry::Picture { caption: content.caption.clone().unwrap_or_default(), filename }),
        _ => None,
    }
}

fn print_diff(filename: &str, from: &str, to: &str) {
    if from == to {
        return;
    }

    println!("diff --writebook a/{filename} b/{filename}");
    print!("{}", diffy::create_patch(from, to));
    println!();
}
