use super::*;
use crate::checkout::CoverState;
use crate::scratch::TempDir;

const META: &str = "---\ntitle: Book\nsubtitle: \nauthor: \ntheme: blue\n---\n\n";

struct ScratchCheckout {
    checkout: Checkout,
    _dir: TempDir,
}

impl std::ops::Deref for ScratchCheckout {
    type Target = Checkout;

    fn deref(&self) -> &Checkout {
        &self.checkout
    }
}

fn scratch_checkout() -> ScratchCheckout {
    let dir = TempDir::new();
    ScratchCheckout {
        checkout: Checkout::create(&dir.join("book")).unwrap(),
        _dir: dir,
    }
}

fn fixture() -> (ScratchCheckout, State, Document) {
    let checkout = scratch_checkout();

    let index = format!("{META}# Part One\n[Intro](intro.md)\n![A pic](assets/pic.jpg)\n");
    checkout.write_working("index.md", &index).unwrap();
    checkout.write_base("index.md", &index).unwrap();
    checkout.write_working("intro.md", "hello\n").unwrap();
    checkout.write_base("intro.md", "hello\n").unwrap();
    std::fs::write(checkout.working_path("assets/pic.jpg").unwrap(), b"JPEG").unwrap();

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
fn a_new_section_theme_is_a_local_change_and_a_missing_one_is_not() {
    let (checkout, mut state, _) = fixture();
    state.leaves[0].theme = Some("green".into());

    let unchanged = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();
    assert!(!compare(&checkout, &state, &unchanged, None).unwrap().changed_locally());

    checkout
        .write_working(
            "index.md",
            &checkout
                .read_working("index.md")
                .unwrap()
                .replace("# Part One", "# Part One {theme=violet}"),
        )
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.leaves[0].changed_locally);
    assert!(!comparison.leaves[1].changed_locally);
}

#[test]
fn a_cover_named_in_front_matter_is_a_local_cover_change() {
    let (checkout, state, _) = fixture();
    std::fs::write(checkout.working_path("assets/cover.png").unwrap(), b"PNG").unwrap();
    checkout
        .write_working(
            "index.md",
            &checkout
                .read_working("index.md")
                .unwrap()
                .replace("theme: blue\n", "theme: blue\ncover: assets/cover.png\n"),
        )
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.cover_changed_locally);
    assert!(
        !comparison.book_changed_locally,
        "the cover alone doesn't touch the title, author or theme"
    );

    std::fs::remove_file(checkout.working_path("assets/cover.png").unwrap()).unwrap();
    let error = compare(&checkout, &state, &working, None).err().unwrap();
    assert!(error.to_string().contains("assets/cover.png"), "{error}");

    std::fs::create_dir(checkout.working_path("assets/cover.png").unwrap()).unwrap();
    let error = compare(&checkout, &state, &working, None).err().unwrap();
    assert!(error.to_string().contains("is not a file"), "{error}");
}

#[test]
fn a_replaced_cover_file_is_a_local_cover_change() {
    let (checkout, mut state, working) = fixture();
    std::fs::write(checkout.working_path("assets/cover.png").unwrap(), b"PNG").unwrap();
    state.cover = Some(CoverState {
        filename: "assets/cover.png".into(),
        remote_checksum: "r".into(),
        sha256: crate::checkout::sha256(b"PNG"),
    });
    assert!(
        !compare(&checkout, &state, &working, None)
            .unwrap()
            .cover_changed_locally
    );

    std::fs::write(checkout.working_path("assets/cover.png").unwrap(), b"NEW PNG").unwrap();
    assert!(
        compare(&checkout, &state, &working, None)
            .unwrap()
            .cover_changed_locally
    );
}

#[test]
fn edited_front_matter_is_a_local_book_change() {
    let (checkout, state, _) = fixture();
    checkout
        .write_working(
            "index.md",
            &checkout
                .read_working("index.md")
                .unwrap()
                .replace("title: Book", "title: Tome"),
        )
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
        .write_working(
            "index.md",
            &format!("{META}# Part One\n[Introduction](intro.md)\n![A pic](assets/pic.jpg)\n"),
        )
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.leaves[1].changed_locally);
}

#[test]
fn removed_lines_and_new_files_classify_as_deletes_and_adds() {
    let (checkout, state, _) = fixture();
    checkout
        .write_working(
            "index.md",
            &format!("{META}# Part One\n![A pic](assets/pic.jpg)\n[Epilogue](epilogue.md)\n"),
        )
        .unwrap();
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
    checkout
        .write_working(
            "index.md",
            &format!("{META}[Intro](intro.md)\n![A pic](assets/pic.jpg)\n\n# Part One\n"),
        )
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.order_changed_locally);
    assert!(
        comparison
            .leaves
            .iter()
            .all(|it| !it.changed_locally && !it.deleted_locally())
    );
}

#[test]
fn a_renamed_page_file_with_identical_content_is_recognized() {
    let (checkout, state, _) = fixture();
    std::fs::rename(
        checkout.working_path("intro.md").unwrap(),
        checkout.working_path("welcome.md").unwrap(),
    )
    .unwrap();
    checkout
        .write_working(
            "index.md",
            &format!("{META}# Part One\n[Intro](welcome.md)\n![A pic](assets/pic.jpg)\n"),
        )
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.added_locally.is_empty());
    assert!(!comparison.leaves[1].deleted_locally());
    assert!(!comparison.leaves[1].changed_locally);
}

#[test]
fn a_changed_picture_file_is_locally_modified() {
    let (checkout, state, working) = fixture();
    std::fs::write(checkout.working_path("assets/pic.jpg").unwrap(), b"JPEG2").unwrap();

    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.leaves[2].changed_locally);
}

#[test]
fn a_picture_is_compared_by_its_bytes_even_when_they_are_not_text() {
    let (checkout, mut state, working) = fixture();
    let png = [0x89, b'P', b'N', b'G'];
    std::fs::write(checkout.working_path("assets/pic.jpg").unwrap(), png).unwrap();

    state.leaves[2].image_sha256 = Some(crate::checkout::sha256(&png));
    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(!comparison.leaves[2].changed_locally);

    state.leaves[2].image_sha256 = Some(crate::checkout::sha256(b"JPEG"));
    let comparison = compare(&checkout, &state, &working, None).unwrap();
    assert!(comparison.leaves[2].changed_locally);
}

#[test]
fn remote_fingerprints_drive_remote_classification() {
    let (checkout, state, working) = fixture();
    let mut manifest = manifest_like(&state);
    manifest.leaves[1].fingerprint = "p2-changed".into();
    manifest.leaves.remove(0);
    manifest.leaves.push(ManifestLeaf {
        id: 9,
        kind: "Page".into(),
        title: "New".into(),
        fingerprint: "n9".into(),
    });
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
    checkout
        .write_working("index.md", &format!("{META}[Gone](gone.md)\n"))
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    assert!(compare(&checkout, &state, &working, None).is_err());
}

#[test]
fn conflict_markers_block_syncing_even_on_line_one() {
    let (checkout, _, working) = fixture();
    checkout
        .write_working("intro.md", "<<<<<<< ours\nmine\n=======\ntheirs\n>>>>>>> theirs\n")
        .unwrap();

    assert!(ensure_no_conflict_markers(&checkout, &working.entries).is_err());
}

#[test]
fn a_duplicate_filename_is_a_hard_error() {
    let (checkout, state, _) = fixture();
    checkout
        .write_working("index.md", &format!("{META}[Intro](intro.md)\n[Again](intro.md)\n"))
        .unwrap();
    let working = crate::index::parse(&checkout.read_working("index.md").unwrap()).unwrap();

    assert!(compare(&checkout, &state, &working, None).is_err());
}
