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
}

impl Comparison {
    pub fn changed_locally(&self) -> bool {
        self.book_changed_locally
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

pub fn compare(checkout: &Checkout, state: &State, document: &Document, manifest: Option<&Manifest>) -> Result<Comparison> {
    let working = &document.entries;
    ensure_wellformed(checkout, working)?;

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

        leaves.push(LeafComparison {
            changed_locally: entry.as_ref().is_some_and(|it| entry_changed(checkout, leaf, it)),
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

    let surviving_base_local: Vec<u64> = base_order.iter().copied().filter(|id| local_order.contains(id)).collect();
    let surviving_base_remote: Vec<u64> = base_order.iter().copied().filter(|id| remote_order.contains(id)).collect();
    let surviving_remote: Vec<u64> = remote_order.iter().copied().filter(|id| base_order.contains(id)).collect();

    Ok(Comparison {
        leaves,
        added_locally,
        added_remotely,
        order_changed_locally: local_order != surviving_base_local,
        order_changed_remotely: manifest.is_some() && surviving_remote != surviving_base_remote,
        book_changed_locally: document.meta != crate::index::parse(&checkout.read_base("index.md")?)?.meta,
        book_changed_remotely: manifest.is_some_and(|it| it.book.fingerprint != state.book_fingerprint),
    })
}

/// Refuse to sync while unresolved merge markers sit in any working text file.
pub fn ensure_no_conflict_markers(checkout: &Checkout, working: &[Entry]) -> Result<()> {
    let mut filenames = vec!["index.md".to_string()];
    filenames.extend(working.iter().filter_map(|it| it.filename()).filter(|it| it.ends_with(".md")).map(str::to_string));

    for filename in filenames {
        let contents = checkout.read_working(&filename).unwrap_or_default();

        if contents.lines().any(|it| it.starts_with("<<<<<<<")) {
            bail!("{filename} still contains merge conflict markers — resolve them first");
        }
    }

    Ok(())
}

/// Every filename the checkout already uses, so new files never collide.
pub fn taken_filenames(checkout: &Checkout, state: &State, working: &[Entry]) -> Result<Vec<String>> {
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

/// Working .md files in the checkout root that no index line references.
pub fn untracked_files(checkout: &Checkout, working: &[Entry]) -> Result<Vec<String>> {
    let mut untracked = Vec::new();

    for entry in std::fs::read_dir(&checkout.root)? {
        let entry = entry?;

        if !entry.file_type()?.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        if !name.ends_with(".md") || name == "index.md" {
            continue;
        }

        if !working.iter().any(|it| it.filename() == Some(name.as_str())) {
            untracked.push(name);
        }
    }

    untracked.sort();
    Ok(untracked)
}

fn ensure_wellformed(checkout: &Checkout, working: &[Entry]) -> Result<()> {
    let mut seen = Vec::new();

    for entry in working {
        if let Some(filename) = entry.filename() {
            if seen.contains(&filename) {
                bail!("index.md lists {filename} more than once");
            }

            seen.push(filename);

            if !checkout.working_path(filename).exists() {
                bail!("index.md references {filename}, which does not exist");
            }
        }
    }

    Ok(())
}

/// Match each state leaf to the working index entry that carries it now:
/// pages, pictures, and body-bearing sections by filename, bare sections by
/// title (in order, so duplicate titles pair up positionally). A page whose
/// file was renamed on disk still matches when its content is untouched.
fn claim_entries(state: &State, working: &[Entry], checkout: &Checkout) -> Result<Vec<Option<Entry>>> {
    let mut claims: Vec<Option<Entry>> = vec![None; state.leaves.len()];
    let mut claimed_entries: Vec<&Entry> = Vec::new();

    for (position, leaf) in state.leaves.iter().enumerate() {
        if let Some(filename) = &leaf.filename {
            if let Some(entry) = working
                .iter()
                .find(|it| it.kind() == leaf.kind && it.filename() == Some(filename.as_str()) && !claimed_entries.contains(it))
            {
                claims[position] = Some(entry.clone());
                claimed_entries.push(entry);
            }
        }
    }

    for (position, leaf) in state.leaves.iter().enumerate() {
        if claims[position].is_none() && leaf.kind == "Section" && leaf.filename.is_none() {
            if let Some(entry) = working
                .iter()
                .find(|it| matches!(it, Entry::Section { filename: None, .. }) && it.title() == leaf.title && !claimed_entries.contains(it))
            {
                claims[position] = Some(entry.clone());
                claimed_entries.push(entry);
            }
        }
    }

    for (position, leaf) in state.leaves.iter().enumerate() {
        if claims[position].is_none() && leaf.kind == "Page" {
            if let Some(base_filename) = &leaf.filename {
                let base_content = checkout.read_base(base_filename).unwrap_or_default();

                if let Some(entry) = working.iter().find(|it| {
                    matches!(it, Entry::Page { .. })
                        && !claimed_entries.contains(it)
                        && it.filename()
                            .is_some_and(|filename| checkout.read_working(filename).is_ok_and(|content| content == base_content))
                }) {
                    claims[position] = Some(entry.clone());
                    claimed_entries.push(entry);
                }
            }
        }
    }

    Ok(claims)
}

fn entry_changed(checkout: &Checkout, leaf: &LeafState, entry: &Entry) -> bool {
    if entry.title() != leaf.title {
        return true;
    }

    match entry.filename() {
        Some(filename) => {
            let working = checkout.read_working(filename).unwrap_or_default();
            let base = match &leaf.filename {
                Some(base_filename) => checkout.read_base(base_filename).unwrap_or_default(),
                None => String::new(),
            };

            match entry {
                Entry::Picture { .. } => picture_image_changed(checkout, leaf, filename),
                _ => working != base,
            }
        }
        None => match &leaf.filename {
            Some(_) => true,
            None => false,
        },
    }
}

fn picture_image_changed(checkout: &Checkout, leaf: &LeafState, filename: &str) -> bool {
    match (&leaf.image_sha256, std::fs::read(checkout.working_path(filename))) {
        (Some(synced), Ok(bytes)) => crate::checkout::sha256(&bytes) != *synced,
        (None, Ok(_)) => true,
        (_, Err(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkout::CoverState;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn scratch_checkout() -> Checkout {
        let root = std::env::temp_dir().join(format!(
            "writebook-plan-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));

        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }

        Checkout::create(&root).unwrap()
    }

    const META: &str = "---\ntitle: Book\nsubtitle: \nauthor: \ntheme: blue\n---\n\n";

    fn fixture() -> (Checkout, State, Document) {
        let checkout = scratch_checkout();

        let index = format!("{META}# Part One\n[Intro](intro.md)\n![A pic](assets/pic.jpg)\n");
        checkout.write_working("index.md", &index).unwrap();
        checkout.write_base("index.md", &index).unwrap();
        checkout.write_working("intro.md", "hello\n").unwrap();
        checkout.write_base("intro.md", "hello\n").unwrap();
        std::fs::write(checkout.working_path("assets/pic.jpg"), b"JPEG").unwrap();

        let state = State {
            version: 1,
            book_fingerprint: "book-f".into(),
            cover: None::<CoverState>,
            leaves: vec![
                LeafState {
                    id: 1,
                    kind: "Section".into(),
                    title: "Part One".into(),
                    fingerprint: "s1".into(),
                    filename: None,
                    theme: None,
                    image_remote_checksum: None,
                    image_sha256: None,
                },
                LeafState {
                    id: 2,
                    kind: "Page".into(),
                    title: "Intro".into(),
                    fingerprint: "p2".into(),
                    filename: Some("intro.md".into()),
                    theme: None,
                    image_remote_checksum: None,
                    image_sha256: None,
                },
                LeafState {
                    id: 3,
                    kind: "Picture".into(),
                    title: "A pic".into(),
                    fingerprint: "i3".into(),
                    filename: Some("assets/pic.jpg".into()),
                    theme: None,
                    image_remote_checksum: Some("remote".into()),
                    image_sha256: Some(crate::checkout::sha256(b"JPEG")),
                },
            ],
            uploads: Default::default(),
        };

        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();
        (checkout, state, working)
    }

    fn manifest_like(state: &State) -> Manifest {
        Manifest {
            book: ManifestBook {
                title: "Book".into(),
                subtitle: None,
                author: None,
                theme: "blue".into(),
                slug: "book".into(),
                fingerprint: "book-f".into(),
                cover: None,
            },
            leaves: state
                .leaves
                .iter()
                .map(|it| ManifestLeaf {
                    id: it.id,
                    kind: it.kind.clone(),
                    title: it.title.clone(),
                    fingerprint: it.fingerprint.clone(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_clean_checkout_reports_no_changes() {
        let (checkout, state, working) = fixture();
        let manifest = manifest_like(&state);
        let comparison = compare(&checkout, &state, &working, Some(&manifest)).unwrap();

        assert!(!comparison.changed_locally());
        assert!(!comparison.changed_remotely());
    }

    #[test]
    fn edited_front_matter_is_a_local_book_change() {
        let (checkout, state, _) = fixture();
        checkout
            .write_working("index.md", &checkout.read_working("index.md").unwrap().replace("title: Book", "title: Tome"))
            .unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.book_changed_locally);
        assert!(comparison.leaves.iter().all(|it| !it.changed_locally));
    }

    #[test]
    fn an_edited_page_is_locally_modified() {
        let (checkout, state, working) = fixture();
        checkout.write_working("intro.md", "hello world\n").unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.leaves[1].changed_locally);
        assert!(!comparison.leaves[0].changed_locally);
    }

    #[test]
    fn a_retitled_index_line_is_locally_modified() {
        let (checkout, state, _) = fixture();
        checkout
            .write_working("index.md", &format!("{META}# Part One\n[Introduction](intro.md)\n![A pic](assets/pic.jpg)\n"))
            .unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.leaves[1].changed_locally);
    }

    #[test]
    fn removed_lines_and_new_files_classify_as_deletes_and_adds() {
        let (checkout, state, _) = fixture();
        checkout.write_working("index.md", &format!("{META}# Part One\n![A pic](assets/pic.jpg)\n[Epilogue](epilogue.md)\n")).unwrap();
        checkout.write_working("epilogue.md", "bye\n").unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.leaves[1].deleted_locally());
        assert_eq!(comparison.added_locally.len(), 1);
        assert_eq!(comparison.added_locally[0].title(), "Epilogue");
    }

    #[test]
    fn reordering_lines_changes_only_the_order() {
        let (checkout, state, _) = fixture();
        checkout.write_working("index.md", &format!("{META}[Intro](intro.md)\n![A pic](assets/pic.jpg)\n\n# Part One\n")).unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.order_changed_locally);
        assert!(comparison.leaves.iter().all(|it| !it.changed_locally && !it.deleted_locally()));
    }

    #[test]
    fn a_renamed_page_file_with_identical_content_is_recognized() {
        let (checkout, state, _) = fixture();
        std::fs::rename(checkout.working_path("intro.md"), checkout.working_path("welcome.md")).unwrap();
        checkout.write_working("index.md", &format!("{META}# Part One\n[Intro](welcome.md)\n![A pic](assets/pic.jpg)\n")).unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.added_locally.is_empty());
        assert!(!comparison.leaves[1].deleted_locally());
        assert!(!comparison.leaves[1].changed_locally);
    }

    #[test]
    fn a_changed_picture_file_is_locally_modified() {
        let (checkout, state, working) = fixture();
        std::fs::write(checkout.working_path("assets/pic.jpg"), b"JPEG2").unwrap();

        let comparison = compare(&checkout, &state, &working, None).unwrap();
        assert!(comparison.leaves[2].changed_locally);
    }

    #[test]
    fn remote_fingerprints_drive_remote_classification() {
        let (checkout, state, working) = fixture();
        let mut manifest = manifest_like(&state);
        manifest.leaves[1].fingerprint = "p2-changed".into();
        manifest.leaves.remove(0);
        manifest.leaves.push(ManifestLeaf { id: 9, kind: "Page".into(), title: "New".into(), fingerprint: "n9".into() });
        manifest.book.fingerprint = "book-f2".into();

        let comparison = compare(&checkout, &state, &working, Some(&manifest)).unwrap();
        assert!(comparison.leaves[0].deleted_remotely);
        assert!(comparison.leaves[1].changed_remotely);
        assert_eq!(comparison.added_remotely.len(), 1);
        assert!(comparison.book_changed_remotely);
    }

    #[test]
    fn a_remote_reorder_is_just_a_reorder() {
        let (checkout, state, working) = fixture();
        let mut manifest = manifest_like(&state);
        manifest.leaves.swap(1, 2);

        let comparison = compare(&checkout, &state, &working, Some(&manifest)).unwrap();
        assert!(comparison.order_changed_remotely);
        assert!(!comparison.changed_locally());
    }

    #[test]
    fn conflicts_need_both_sides() {
        let (checkout, state, working) = fixture();
        checkout.write_working("intro.md", "hello world\n").unwrap();
        let mut manifest = manifest_like(&state);
        manifest.leaves[1].fingerprint = "p2-changed".into();

        let comparison = compare(&checkout, &state, &working, Some(&manifest)).unwrap();
        assert!(comparison.leaves[1].conflicted());
        assert!(!comparison.leaves[0].conflicted());
    }

    #[test]
    fn a_missing_referenced_file_is_a_hard_error() {
        let (checkout, state, _) = fixture();
        checkout.write_working("index.md", &format!("{META}[Gone](gone.md)\n")).unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        assert!(compare(&checkout, &state, &working, None).is_err());
    }

    #[test]
    fn conflict_markers_block_syncing_even_on_line_one() {
        let (checkout, _, working) = fixture();
        checkout.write_working("intro.md", "<<<<<<< ours\nmine\n=======\ntheirs\n>>>>>>> theirs\n").unwrap();

        assert!(ensure_no_conflict_markers(&checkout, &working.entries).is_err());
    }

    #[test]
    fn a_duplicate_filename_is_a_hard_error() {
        let (checkout, state, _) = fixture();
        checkout.write_working("index.md", &format!("{META}[Intro](intro.md)\n[Again](intro.md)\n")).unwrap();
        let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

        assert!(compare(&checkout, &state, &working, None).is_err());
    }
}
