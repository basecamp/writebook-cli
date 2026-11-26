use super::*;
use crate::lint::tree::violations;

const MANIFEST: &str = "[package]\nname = \"demo\"\nversion = \"0.0.0\"\nedition = \"2024\"\n";

#[test]
fn an_integration_test_must_say_it_is_test_code() {
    let files = [
        ("Cargo.toml", MANIFEST),
        ("src/lib.rs", ""),
        ("tests/a.rs", "#![cfg(test)]\n\nuse std::fs;\n"),
        ("tests/b.rs", "//! The vectors.\n\n#![cfg(test)]\n"),
        ("tests/c.rs", "#![cfg(test)]\n#![cfg(target_os = \"linux\")]\n"),
        ("tests/d.rs", "use std::fs;\n"),
        ("tests/e.rs", "#[cfg(test)]\nmod tests;\n"),
        ("tests/f.rs", "#![cfg(any(test, feature = \"x\"))]\n"),
    ];

    assert_eq!(
        violations(&files, every_integration_test_says_it_is_test_code),
        ["demo (/tests/d.rs)", "demo (/tests/e.rs)", "demo (/tests/f.rs)"]
    );
}

#[test]
fn every_crate_must_take_the_workspace_lints() {
    let workspace = "[workspace]\nmembers = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\nresolver = \"3\"\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n";
    let package = |name: &str, tail: &str| {
        format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{tail}")
    };
    let a = package("a", "\n[lints]\nworkspace = true\n");
    let b = package("b", "lints.workspace = true\n");
    let c = package("c", "\n[lints]\n# why\nworkspace = true\n");
    let d = package("d", "");
    let e = package("e", "\n[lints.rust]\nunsafe_code = \"allow\"\n");
    let f = format!("lints.workspace = true\n\n{}", package("f", ""));
    let files = [
        ("Cargo.toml", workspace),
        ("a/Cargo.toml", &a),
        ("a/src/lib.rs", ""),
        ("b/Cargo.toml", &b),
        ("b/src/lib.rs", ""),
        ("c/Cargo.toml", &c),
        ("c/src/lib.rs", ""),
        ("d/Cargo.toml", &d),
        ("d/src/lib.rs", ""),
        ("e/Cargo.toml", &e),
        ("e/src/lib.rs", ""),
        ("f/Cargo.toml", &f),
        ("f/src/lib.rs", ""),
    ];

    assert_eq!(
        violations(&files, every_crate_takes_the_workspace_lints),
        ["b", "d", "e"]
    );
}

#[test]
fn a_rust_file_may_be_a_thousand_lines_but_no_more() {
    let long = "fn a() {}\n".repeat(1_001);
    let longest = "fn a() {}\n".repeat(1_000);
    let notes = "words\n".repeat(2_000);

    assert_eq!(
        violations(
            &[
                ("src/long.rs", &long),
                ("src/longest.rs", &longest),
                ("tests/notes.md", &notes)
            ],
            no_rust_file_is_over_a_thousand_lines
        ),
        ["src/long.rs (1001 lines)"]
    );
}

#[test]
fn nothing_may_be_named_for_a_kind_of_code() {
    let files = [
        ("src/utils.rs", ""),
        ("src/helpers/mod.rs", ""),
        (
            "src/main.rs",
            "mod misc;\npub(crate) mod common;\nmod commonplace;\nmod sign_in;\n",
        ),
        ("tests/support/shared.rs", ""),
        ("src/sign_in.rs", ""),
        ("docs/util.rs", ""),
    ];

    assert_eq!(
        violations(&files, nothing_is_named_for_a_kind_of_code),
        [
            "src/helpers",
            "src/main.rs:1: mod misc;",
            "src/main.rs:2: pub(crate) mod common;",
            "src/utils.rs",
            "tests/support/shared.rs",
        ]
    );
}
