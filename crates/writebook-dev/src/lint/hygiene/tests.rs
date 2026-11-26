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
    let workspace = "[workspace]\nmembers = [\"a\", \"b\", \"c\", \"d\", \"e\"]\nresolver = \"3\"\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n";
    let package = |name: &str, tail: &str| {
        format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{tail}")
    };
    let a = package("a", "\n[lints]\nworkspace = true\n");
    let b = package("b", "lints.workspace = true\n");
    let c = package("c", "\n[lints]\n# why\nworkspace = true\n");
    let d = package("d", "");
    let e = package("e", "\n[lints.rust]\nunsafe_code = \"allow\"\n");
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
    ];

    assert_eq!(violations(&files, every_crate_takes_the_workspace_lints), ["d", "e"]);
}
