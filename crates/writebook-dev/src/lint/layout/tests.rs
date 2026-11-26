use super::*;
use crate::lint::tree::violations;

const LAID_OUT: &str = "use std::fmt;
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
         const LATE: u8 = 2;";

#[test]
fn constants_after_other_code_are_refused() {
    assert_eq!(
        violations(&[("src/a.rs", LAID_OUT)], constants_come_before_any_other_code),
        ["src/a.rs:13: LATE after code"]
    );
}

#[test]
fn an_impls_functions_out_of_order_are_refused() {
    assert_eq!(
        violations(
            &[("src/a.rs", LAID_OUT)],
            an_impl_starts_with_new_then_associated_functions_then_public_then_private_methods
        ),
        ["src/a.rs:9: new out of order", "src/a.rs:11: late out of order"]
    );
}

#[test]
fn a_trait_impl_before_the_inherent_impl_is_refused() {
    assert_eq!(
        violations(&[("src/a.rs", LAID_OUT)], trait_impls_come_after_the_inherent_impl),
        ["src/a.rs:4: Posting's trait impl before its inherent impl"]
    );
}
