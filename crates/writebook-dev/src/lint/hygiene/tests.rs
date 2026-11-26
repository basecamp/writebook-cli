use super::*;

#[test]
fn the_test_code_matcher_refuses_what_it_should() {
    for (root, says) in [
        ("#![cfg(test)]\n\nuse std::fs;\n", true),
        ("//! The vectors.\n\n#![cfg(test)]\n", true),
        ("#![cfg(test)]\n#![cfg(target_os = \"linux\")]\n", true),
        ("use std::fs;\n", false),
        ("#[cfg(test)]\nmod tests;\n", false),
        ("#![cfg(any(test, feature = \"x\"))]\n", false),
    ] {
        assert_eq!(says_it_is_test_code(root), says, "{root}");
    }
}

#[test]
fn the_lints_matcher_refuses_what_it_should() {
    for (manifest, takes) in [
        ("[package]\nname = \"a\"\n\n[lints]\nworkspace = true\n", true),
        ("[package]\nname = \"a\"\nlints.workspace = true\n", true),
        ("[lints]\n# why\nworkspace = true\n", true),
        ("[package]\nname = \"a\"\n", false),
        ("[lints.rust]\nunsafe_code = \"allow\"\n", false),
        ("[lints]\nworkspace = false\n", false),
    ] {
        assert_eq!(takes_the_workspace_lints(manifest), takes, "{manifest}");
    }
}
