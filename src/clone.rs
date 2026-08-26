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
        None => PathBuf::from(&manifest.book.slug),
    };

    if root.exists() && fs::read_dir(&root).context("could not inspect the target directory")?.next().is_some() {
        bail!("{} already exists and is not empty", root.display());
    }

    println!("Cloning \"{}\" into {}...", manifest.book.title, root.display());

    let checkout = Checkout::create(&root)?;

    let mut config = Config::default();
    config.remotes.insert("origin".to_string(), Remote { url: host, book_id });
    checkout.save_config(&config)?;

    let mut state = State { version: 1, book_fingerprint: manifest.book.fingerprint.clone(), ..State::default() };
    let mut taken = vec!["index.md".to_string()];
    let mut entries = Vec::new();

    for leaf in &manifest.leaves {
        if let Some(entry) = sync::materialize_leaf(&mut client, &checkout, &mut state, book_id, leaf, &mut taken)? {
            entries.push(entry);
        }
    }

    let index_md = index::serialize(&sync::book_meta(&manifest), &entries);
    checkout.write_working("index.md", &index_md)?;
    checkout.write_base("index.md", &index_md)?;

    sync::mirror_cover(&mut client, &checkout, &mut state, &manifest)?;
    checkout.save_state(&state)?;

    println!("Done: {} leaves.", manifest.leaves.len());
    Ok(())
}
