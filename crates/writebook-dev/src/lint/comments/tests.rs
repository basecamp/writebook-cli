use super::*;
use crate::lint::tree::violations;

#[test]
fn a_comment_that_is_a_banner_a_marker_or_code_is_refused() {
    let source = [
        "// ----------------",
        "    // ==== Postings ====",
        "// TODO: drop this",
        "/// For now this keeps the old order",
        "// let seen = posting.seen;",
        "    // }",
        "// fn old() {",
        "// The vendor's API sends a 202 for a refusal.",
        "/// ```",
        "/// let seen = posting.seen;",
        "let url = \"https://example.com\"; // TODO",
        "//! The postings.",
        "//! - **Verified.** Every fact told is read back",
        "/// goes by that name -- the grammar's --",
    ]
    .join("\n");

    assert_eq!(
        violations(&[("src/a.rs", &source)], no_comment_is_a_banner_a_marker_or_code),
        [
            "src/a.rs:1: banner: // ----------------",
            "src/a.rs:2: banner: // ==== Postings ====",
            "src/a.rs:3: marker: // TODO: drop this",
            "src/a.rs:5: code: // let seen = posting.seen;",
            "src/a.rs:6: code: // }",
            "src/a.rs:7: code: // fn old() {",
        ]
    );
}

#[test]
fn a_comment_inside_a_test_is_refused() {
    let source = [
        "fn helper() {",
        "    // fine outside a test",
        "}",
        "",
        "#[test]",
        "fn works() {",
        "    // refused",
        "    let seen = 1;",
        "}",
        "",
        "#[cfg(test)]",
        "mod tests {",
        "    fn helper() {",
        "        // refused",
        "    }",
        "}",
    ]
    .join("\n");

    assert_eq!(
        violations(&[("src/a.rs", &source)], no_test_holds_a_comment),
        ["src/a.rs:7: // refused", "src/a.rs:14: // refused"]
    );
}

#[test]
fn a_doc_comment_on_a_test_must_link_something() {
    let source = [
        "/// Holds https://example.com/issues/1",
        "#[test]",
        "fn linked() {}",
        "/// Says what it does.",
        "#[tokio::test]",
        "fn unlinked() {}",
        "#[test]",
        "fn plain() {}",
        "/// Docs a helper.",
        "fn helper() {}",
    ]
    .join("\n");

    assert_eq!(
        violations(&[("src/a.rs", &source)], no_test_is_documented_without_a_link),
        ["src/a.rs:4"]
    );
}

#[test]
fn a_doc_comment_on_a_private_item_is_refused() {
    let source = [
        "//! Module docs.",
        "/// Doc.",
        "fn private() {}",
        "/// Doc.",
        "pub fn public() {}",
        "/// Doc.",
        "pub(crate) fn crate_wide() {}",
        "pub struct Thing {",
        "    /// Doc.",
        "    field: u8,",
        "    /// Doc.",
        "    pub open: u8,",
        "}",
        "impl Thing {",
        "    /// Doc.",
        "    fn hidden(&self) {}",
        "    /// Doc.",
        "    pub fn shown(&self) {}",
        "}",
        "impl Default for Thing {",
        "    /// Doc.",
        "    fn default() -> Self {",
        "        todo!()",
        "    }",
        "}",
    ]
    .join("\n");

    assert_eq!(
        violations(&[("src/a.rs", &source)], no_private_item_is_documented),
        ["src/a.rs:2", "src/a.rs:9", "src/a.rs:15"]
    );
}

#[test]
fn a_comment_that_tells_history_or_makes_an_excuse_is_refused() {
    let source = [
        "// for now this keeps the old order",
        "/// Previously a Vec",
        "// it used to retry",
        "//! A Temporary table",
        "// a workaround for the driver",
        "// no workarounds here",
        "// the temporal order",
        "// reused to the end",
        "let x = 1; // for now",
    ]
    .join("\n");

    assert_eq!(
        violations(&[("src/a.rs", &source)], no_comment_tells_history_or_makes_an_excuse),
        [
            "src/a.rs:1: history or an excuse: // for now this keeps the old order",
            "src/a.rs:2: history or an excuse: /// Previously a Vec",
            "src/a.rs:3: history or an excuse: // it used to retry",
            "src/a.rs:4: history or an excuse: //! A Temporary table",
            "src/a.rs:5: history or an excuse: // a workaround for the driver",
        ]
    );
}
