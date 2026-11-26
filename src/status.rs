use anyhow::Result;

use crate::api;
use crate::checkout::{Checkout, Remote};
use crate::index;
use crate::plan::{self, Comparison};
use crate::sync::{self, Manifest};

pub fn run() -> Result<()> {
    let checkout = Checkout::find()?;
    let origin = checkout.origin()?;
    let state = checkout.state()?;
    let document = index::parse(&checkout.read_working("index.md")?)?;

    let manifest = fetch_manifest_if_reachable(&origin);
    let comparison = plan::compare(&checkout, &state, &document, manifest.as_ref())?;

    print_local_changes(&comparison);
    if manifest.is_some() {
        print_remote_changes(&comparison);
    }
    print_conflicts(&comparison);
    print_untracked(&checkout, &document.entries)?;

    if !comparison.changed_locally() && !comparison.changed_remotely() {
        println!("Everything up to date.");
    }

    Ok(())
}

fn fetch_manifest_if_reachable(origin: &Remote) -> Option<Manifest> {
    let attempt =
        api::Client::connect(&origin.url).and_then(|mut client| sync::fetch_manifest(&mut client, origin.book_id));

    match attempt {
        Ok(manifest) => Some(manifest),
        Err(error) => {
            eprintln!(
                "note: could not reach {} ({error:#}) — showing local changes only",
                origin.url
            );
            None
        }
    }
}

fn print_local_changes(comparison: &Comparison) {
    let mut lines = Vec::new();

    if comparison.book_changed_locally {
        lines.push("modified:   book metadata (index.md front matter)".to_string());
    }

    if comparison.cover_changed_locally {
        lines.push("modified:   cover".to_string());
    }

    for leaf in &comparison.leaves {
        if leaf.deleted_locally() {
            lines.push(format!(
                "deleted:    {}",
                label(&leaf.state.filename, &leaf.state.title)
            ));
        } else if leaf.changed_locally {
            lines.push(format!(
                "modified:   {}",
                label(&leaf.state.filename, &leaf.state.title)
            ));
        }
    }

    for entry in &comparison.added_locally {
        lines.push(format!(
            "added:      {}",
            label(&entry.filename().map(str::to_string), entry.title())
        ));
    }

    if comparison.order_changed_locally {
        lines.push("reordered:  index.md".to_string());
    }

    if !lines.is_empty() {
        println!("Local changes (upload with `writebook push`):");
        for line in lines {
            println!("  {line}");
        }
        println!();
    }
}

fn print_remote_changes(comparison: &Comparison) {
    let mut lines = Vec::new();

    if comparison.book_changed_remotely {
        lines.push("modified:   book metadata (index.md front matter)".to_string());
    }

    for leaf in &comparison.leaves {
        if leaf.deleted_remotely {
            lines.push(format!(
                "deleted:    {}",
                label(&leaf.state.filename, &leaf.state.title)
            ));
        } else if leaf.changed_remotely {
            lines.push(format!(
                "modified:   {}",
                label(&leaf.state.filename, &leaf.state.title)
            ));
        }
    }

    for leaf in &comparison.added_remotely {
        lines.push(format!(
            "added:      {} \"{}\"",
            leaf.kind.name().to_lowercase(),
            leaf.title
        ));
    }

    if comparison.order_changed_remotely {
        lines.push("reordered:  index.md".to_string());
    }

    if !lines.is_empty() {
        println!("Remote changes (download with `writebook pull`):");
        for line in lines {
            println!("  {line}");
        }
        println!();
    }
}

fn print_conflicts(comparison: &Comparison) {
    let conflicted: Vec<&plan::LeafComparison> = comparison.leaves.iter().filter(|it| it.conflicted()).collect();

    if !conflicted.is_empty() {
        println!("Changed on both sides (`writebook pull` will merge):");
        for leaf in conflicted {
            println!("  {}", label(&leaf.state.filename, &leaf.state.title));
        }
        println!();
    }
}

fn print_untracked(checkout: &Checkout, working: &[index::Entry]) -> Result<()> {
    let untracked = untracked_files(checkout, working)?;

    if !untracked.is_empty() {
        println!("Untracked files (add them to index.md to publish them):");
        for filename in untracked {
            println!("  {filename}");
        }
        println!();
    }

    Ok(())
}

fn untracked_files(checkout: &Checkout, working: &[index::Entry]) -> Result<Vec<String>> {
    let mut untracked = Vec::new();

    for entry in std::fs::read_dir(&checkout.root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        if entry.file_type()?.is_file()
            && name.ends_with(".md")
            && name != "index.md"
            && !working.iter().any(|it| it.filename() == Some(name.as_str()))
        {
            untracked.push(name);
        }
    }

    untracked.sort();
    Ok(untracked)
}

fn label(filename: &Option<String>, title: &str) -> String {
    match filename {
        Some(filename) => filename.clone(),
        None => format!("section \"{title}\""),
    }
}
