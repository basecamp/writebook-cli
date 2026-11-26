use super::hygiene::refuse;
use super::{Refusal, dependencies, name, packages};

const MOCKS: [&str; 8] = [
    "mockall",
    "mockers",
    "mock-it",
    "faux",
    "mocktopus",
    "double",
    "mry",
    "unimock",
];

pub(super) fn no_crate_takes_a_mocking_library() -> Option<Refusal> {
    let mut mocking = Vec::new();
    for package in packages() {
        for dependency in dependencies(&package) {
            if MOCKS.contains(&dependency.as_str()) {
                mocking.push(format!("{}: {dependency}", name(&package)));
            }
        }
    }
    refuse(
        &mocking,
        "crates take a mocking library: test against real things, a real database or a real \
         socket",
    )
}
