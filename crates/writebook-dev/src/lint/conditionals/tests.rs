use super::*;
use crate::lint::tree::violations;

#[test]
fn a_guard_clause_is_only_allowed_as_the_first_statement_of_a_function() {
    let source = "fn allowed(name: Option<&str>) -> usize {
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
         }";

    assert_eq!(
        violations(
            &[("src/a.rs", source)],
            a_guard_clause_is_only_the_first_statement_of_a_function
        ),
        ["src/a.rs:7: a guard clause", "src/a.rs:14: a guard clause"]
    );
}
