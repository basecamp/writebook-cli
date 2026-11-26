use std::path::Path;

use tempfile::TempDir;

use super::Refusal;

pub(super) fn in_tree(files: &[(&str, &str)]) -> TempDir {
    let tree = tempfile::tempdir().expect("a temporary directory");
    for (path, contents) in files {
        let path = tree.path().join(path);
        std::fs::create_dir_all(path.parent().expect("a file has a directory")).expect("the directory is made");
        std::fs::write(path, contents).expect("the file is written");
    }
    std::env::set_current_dir(tree.path()).expect("the tree is entered");
    tree
}

pub(super) fn violations(files: &[(&str, &str)], check: fn() -> Option<Refusal>) -> Vec<String> {
    let tree = in_tree(files);
    let root = tree.path().canonicalize().expect("the tree resolves");
    match check() {
        Some(refusal) => refusal
            .violations
            .iter()
            .map(|violation| without(&root, violation))
            .collect(),
        None => Vec::new(),
    }
}

fn without(root: &Path, violation: &str) -> String {
    violation.replace(&root.display().to_string(), "")
}
