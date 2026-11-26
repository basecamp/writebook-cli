use super::*;
use crate::lint::tree::in_tree;

#[test]
fn a_file_a_test_module_declares_is_test_code() {
    let _tree = in_tree(&[
        ("src/main.rs", "#[cfg(test)]\nmod support;\nmod tui;\n"),
        ("src/support.rs", "fn a() {}\n"),
        ("src/tui.rs", "#[cfg(test)]\nmod flows;\n"),
        ("src/tui/flows.rs", "fn a() {}\n"),
        ("src/tui/flows/people.rs", "fn a() {}\n"),
        ("src/tui/flowsheet.rs", "fn a() {}\n"),
    ]);

    assert!(is_declared_by_a_test_module("src/support.rs"));
    assert!(is_declared_by_a_test_module("src/tui/flows.rs"));
    assert!(is_declared_by_a_test_module("src/tui/flows/people.rs"));
    assert!(!is_declared_by_a_test_module("src/tui/flowsheet.rs"));
    assert!(!is_declared_by_a_test_module("src/tui.rs"));
}

#[test]
fn a_source_splits_into_production_and_tests() {
    let _tree = in_tree(&[("src/a.rs", "fn a() {}\n#[cfg(test)]\nmod tests { fn b() {} }\n")]);

    let sources = sources();

    assert_eq!(1, sources.len());
    assert_eq!(1, sources[0].production().len());
    assert_eq!(1, sources[0].tests().len());
}
