use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::parsed::is_declared_by_a_test_module;
use super::{Refusal, SCANNED, name, packages, root, scanned};

const MAX_LINES: usize = 1_000;

const LINT: &str = "crates/writebook-dev/src/lint";

const JUNK_DRAWERS: [&str; 7] = ["utils", "util", "helpers", "helper", "common", "shared", "misc"];

pub(super) fn no_rust_file_is_over_a_thousand_lines() -> Option<Refusal> {
    let root = root();
    let mut long = Vec::new();
    for file in scanned() {
        if file.extension().is_some_and(|extension| extension == "rs") {
            let lines = std::fs::read(&file)
                .expect("a Rust file reads")
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            if lines > MAX_LINES {
                long.push(format!("{} ({lines} lines)", relative(&root, &file)));
            }
        }
    }
    refuse(
        &long,
        "Rust files are over a thousand lines: split each by subject (the code-style skill, *Layout*)",
    )
}

pub(super) fn nothing_is_named_for_a_kind_of_code() -> Option<Refusal> {
    let root = root();
    let mut drawers = BTreeSet::new();
    for directory in SCANNED {
        let directory = root.join(directory);
        if directory.is_dir() {
            for path in paths_under(&directory) {
                let relative = relative(&root, &path);
                let mut prefix = PathBuf::new();
                for component in Path::new(&relative).components() {
                    prefix.push(component);
                    if is_junk_drawer(component.as_os_str().to_string_lossy().as_ref()) {
                        drawers.insert(prefix.to_string_lossy().to_string());
                    }
                }
                if path.extension().is_some_and(|extension| extension == "rs") {
                    for (number, line) in read_text(&path).lines().enumerate() {
                        if declares_a_junk_drawer(line) {
                            drawers.insert(format!("{relative}:{}: {}", number + 1, line.trim()));
                        }
                    }
                }
            }
        }
    }
    refuse(
        &drawers.into_iter().collect::<Vec<_>>(),
        "modules, files and directories are named for a kind of code, and a module is named only \
         for the one subject it holds (the code-style skill, *Names*)",
    )
}

pub(super) fn every_crate_takes_the_workspace_lints() -> Option<Refusal> {
    let mut without = Vec::new();
    for package in packages() {
        let manifest = package["manifest_path"].as_str().expect("a package has a manifest");
        if !takes_the_workspace_lints(&read_text(Path::new(manifest))) {
            without.push(name(&package));
        }
    }
    refuse(
        &without,
        "crates have no `[lints] workspace = true`, and so forbid no unsafe code (doc/review.md, SEC-3)",
    )
}

pub(super) fn every_integration_test_says_it_is_test_code() -> Option<Refusal> {
    let mut without = Vec::new();
    for package in packages() {
        let targets = package["targets"].as_array().expect("a package lists its targets");
        for target in targets {
            let root = target["src_path"].as_str().expect("a target has a root");
            if target["kind"][0] == "test" && !says_it_is_test_code(&read_text(Path::new(root))) {
                without.push(format!("{} ({root})", name(&package)));
            }
        }
    }
    refuse(
        &without,
        "integration tests have no `#![cfg(test)]`, so clippy holds their helpers to `unwrap_used` as if they were the app's (the code-style skill, *Fail fast*)",
    )
}

pub(super) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("a path under the root")
        .to_string_lossy()
        .replace('\\', "/")
}

fn paths_under(directory: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory).expect("the directory reads") {
        let path = entry.expect("its entry reads").path();
        paths.push(path.clone());
        if path.is_dir() {
            paths.extend(paths_under(&path));
        }
    }
    paths
}

pub(super) fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{} reads: {error}", path.display()))
}

pub(super) fn refuse(violations: &[String], rule: &str) -> Option<Refusal> {
    if violations.is_empty() {
        None
    } else {
        Some(Refusal {
            rule: rule.to_string(),
            violations: violations.to_vec(),
        })
    }
}

fn is_junk_drawer(component: &str) -> bool {
    Path::new(component)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| JUNK_DRAWERS.contains(&stem))
}

fn declares_a_junk_drawer(line: &str) -> bool {
    let line = line.trim();
    let line = line.strip_prefix("pub ").unwrap_or(line);
    let line = line
        .strip_prefix("pub(crate) ")
        .or_else(|| line.strip_prefix("pub(super) "))
        .unwrap_or(line);
    line.strip_prefix("mod ").is_some_and(|rest| {
        let module: String = rest
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        JUNK_DRAWERS.contains(&module.as_str())
    })
}

pub(super) fn is_rust(file: &Path) -> bool {
    file.extension().is_some_and(|extension| extension == "rs")
}

pub(super) fn is_test_code(path: &str) -> bool {
    path == format!("{LINT}.rs")
        || path.starts_with(&format!("{LINT}/"))
        || path.split('/').any(|component| {
            matches!(component, "tests" | "tests.rs" | "test_support" | "benches")
                || component.ends_with("_tests.rs")
                || component.ends_with("_tests")
        })
        || is_declared_by_a_test_module(path)
}

fn says_it_is_test_code(root: &str) -> bool {
    root.lines().any(|line| line.trim() == "#![cfg(test)]")
}

fn takes_the_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines().map(str::trim) {
        if line == "lints.workspace = true" {
            return true;
        } else if line.starts_with('[') {
            in_lints = line == "[lints]";
        } else if in_lints && line == "workspace = true" {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests;
