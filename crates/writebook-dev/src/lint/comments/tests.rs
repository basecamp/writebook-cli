use super::*;

#[test]
fn the_comment_reader_refuses_what_it_should() {
    for (line, refused) in [
        ("// ----------------", Some("banner")),
        ("    // ==== Postings ====", Some("banner")),
        ("// TODO: drop this", Some("marker")),
        ("/// For now this keeps the old order", None),
        ("// let seen = posting.seen;", Some("code")),
        ("    // }", Some("code")),
        ("// fn old() {", Some("code")),
        ("// The vendor's API sends a 202 for a refusal.", None),
        ("/// ```", None),
        ("/// let seen = posting.seen;", None),
        ("let url = \"https://example.com\"; // TODO", None),
        ("//! The postings.", None),
        ("//! - **Verified.** Every fact told is read back", None),
        ("/// goes by that name -- the grammar's --", None),
    ] {
        assert_eq!(refused_comment(line), refused, "{line}");
    }
}

#[test]
fn the_test_comment_reader_refuses_what_it_should() {
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
    let file = syn::parse_str::<syn::File>(&source).expect("the source parses");
    assert_eq!(test_comment_lines(&file, &source), [7, 14]);
}

#[test]
fn the_test_doc_reader_refuses_what_it_should() {
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
    let file = syn::parse_str::<syn::File>(&source).expect("the source parses");
    assert_eq!(unlinked_test_docs(&file), [4]);
}

#[test]
fn the_private_doc_reader_refuses_what_it_should() {
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
    let file = syn::parse_str::<syn::File>(&source).expect("the source parses");
    assert_eq!(private_docs(&file), [2, 9, 15]);
}

#[test]
fn the_history_reader_refuses_what_it_should() {
    for (line, refused) in [
        ("// for now this keeps the old order", true),
        ("/// Previously a Vec", true),
        ("// it used to retry", true),
        ("//! A Temporary table", true),
        ("// a workaround for the driver", true),
        ("// no workarounds here", false),
        ("// the temporal order", false),
        ("// reused to the end", false),
        ("let x = 1; // for now", false),
    ] {
        let found = comment_text(line).is_some_and(|text| tells_history_or_excuses(text).is_some());
        assert_eq!(found, refused, "{line}");
    }
}
