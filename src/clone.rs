use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::api;
use crate::checkout::{Checkout, Config, Remote, State};
use crate::index;
use crate::remote;
use crate::sync;

pub fn run(url: &str, directory: Option<&Path>) -> Result<()> {
    let (host, book_id) = remote::parse_book_url(url)?;

    let mut client = api::Client::connect(&host)?;
    let manifest = sync::fetch_manifest(&mut client, book_id)?;

    let root = match directory {
        Some(directory) => directory.to_path_buf(),
        None => PathBuf::from(directory_for(&manifest.book.slug, book_id)),
    };

    if fs::symlink_metadata(&root).is_ok_and(|it| it.file_type().is_symlink()) {
        bail!("{} is a symlink; clone into a directory instead", root.display());
    }

    if root.exists()
        && fs::read_dir(&root)
            .context("could not inspect the target directory")?
            .next()
            .is_some()
    {
        bail!("{} already exists and is not empty", root.display());
    }

    crate::output::print_line(&format!(
        "Cloning \"{}\" into {}...",
        manifest.book.title,
        root.display()
    ));

    let checkout = Checkout::create(&root)?;

    let mut config = Config::default();
    config
        .remotes
        .insert("origin".to_string(), Remote { url: host, book_id });
    checkout.save_config(&config)?;

    let mut state = State {
        version: 1,
        book_fingerprint: manifest.book.fingerprint.clone(),
        ..State::default()
    };
    let mut taken = vec!["index.md".to_string()];
    let mut entries = Vec::new();

    for leaf in &manifest.leaves {
        if let Some(entry) = sync::materialize_leaf(&mut client, &checkout, &mut state, book_id, leaf, &mut taken)? {
            entries.push(entry);
        }
    }

    sync::mirror_cover(&mut client, &checkout, &mut state, &manifest)?;

    let index_md = index::serialize(&sync::book_meta(&manifest, &state), &entries);
    checkout.write_working("index.md", &index_md)?;
    checkout.write_base("index.md", &index_md)?;
    checkout.save_state(&state)?;

    println!("Done: {} leaves.", manifest.leaves.len());
    Ok(())
}

fn directory_for(slug: &str, book_id: u64) -> String {
    let name = sync::sanitize_filename(slug);

    if name.is_empty() {
        format!("book-{book_id}")
    } else {
        name
    }
}
