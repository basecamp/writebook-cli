use super::*;
use crate::lint::tree::violations;

#[test]
fn no_crate_may_take_a_mocking_library() {
    let workspace = "[workspace]\nmembers = [\"app\", \"plain\"]\nresolver = \"3\"\n";
    let app = "[package]\nname = \"app\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
               [dependencies]\nserde = \"1\"\n\n[dev-dependencies]\nmockall = \"0.13\"\n";
    let plain = "[package]\nname = \"plain\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                 [dev-dependencies]\nmock-it-not = \"1\"\n";
    let files = [
        ("Cargo.toml", workspace),
        ("app/Cargo.toml", app),
        ("app/src/lib.rs", ""),
        ("plain/Cargo.toml", plain),
        ("plain/src/lib.rs", ""),
    ];

    assert_eq!(violations(&files, no_crate_takes_a_mocking_library), ["app: mockall"]);
}
