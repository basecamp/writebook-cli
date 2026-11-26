use super::*;

#[test]
fn a_file_a_test_module_declares_is_test_code() {
    assert_eq!("src/tui", module_directory("src/tui.rs"));
    assert_eq!("src", module_directory("src/main.rs"));
    assert!(declares("src/tui/flows", "src/tui/flows.rs"));
    assert!(declares("src/tui/flows", "src/tui/flows/people.rs"));
    assert!(!declares("src/tui/flows", "src/tui/flowsheet.rs"));
}

#[test]
fn production_leaves_out_the_tests_module() {
    let file = syn::parse_file("fn a() {}\n#[cfg(test)]\nmod tests { fn b() {} }").expect("the example parses");

    assert_eq!(1, without_tests(&file.items).len());
    assert_eq!(1, tests_among(&file.items).len());
}
