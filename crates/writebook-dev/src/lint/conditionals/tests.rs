use super::*;

#[test]
fn the_guard_reader_refuses_what_it_should() {
    let guarded = syn::parse_file(
        "fn allowed(name: Option<&str>) -> usize {
             let Some(name) = name else { return 0 };
             name.len()
         }
         fn stacked(name: Option<&str>, empty: bool) -> usize {
             let Some(name) = name else { return 0 };
             if empty { return 0; }
             name.len()
         }
         fn nested(names: &[Option<&str>]) -> usize {
             for name in names {
                 let Some(name) = name else { continue };
                 if name.is_empty() { continue; }
                 let Some(first) = name.chars().next() else { return 0 };
             }
             0
         }
         fn valued(name: Option<&str>) -> usize {
             if let Some(name) = name { name.len() } else { 0 }
         }",
    )
    .expect("the example parses");
    let mut guards = Guards {
        found: Found::new("a.rs"),
        body: None,
    };

    guards.visit_file(&guarded);

    assert_eq!(
        vec!["a.rs:7: a guard clause", "a.rs:14: a guard clause"],
        guards.found.lines
    );
}
