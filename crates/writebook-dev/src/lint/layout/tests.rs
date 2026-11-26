use super::*;

#[test]
fn the_layout_readers_refuse_what_they_should() {
    let laid_out = syn::parse_file(
        "use std::fmt;
         const FIRST: u8 = 1;
         struct Posting;
         impl fmt::Debug for Posting {
             fn fmt(&self, _: &mut fmt::Formatter) -> fmt::Result { Ok(()) }
         }
         impl Posting {
             pub fn seen(&self) -> bool { true }
             pub fn new() -> Self { Posting }
             fn private(&self) {}
             pub fn late(&self) {}
         }
         const LATE: u8 = 2;",
    )
    .expect("the example parses");
    let mut constants = Modules {
        found: Found::new("a.rs"),
        check: constants_after_code,
    };
    let mut impls = Impls {
        found: Found::new("a.rs"),
    };
    let mut traits = Modules {
        found: Found::new("a.rs"),
        check: trait_impls_before_the_inherent_one,
    };

    constants.visit_file(&laid_out);
    impls.visit_file(&laid_out);
    traits.visit_file(&laid_out);

    assert_eq!(vec!["a.rs:13: LATE after code"], constants.found.lines);
    assert_eq!(
        vec!["a.rs:9: new out of order", "a.rs:11: late out of order"],
        impls.found.lines
    );
    assert_eq!(
        vec!["a.rs:4: Posting's trait impl before its inherent impl"],
        traits.found.lines
    );
}
