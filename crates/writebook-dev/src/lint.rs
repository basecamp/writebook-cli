//! `bin/dev code lint`: the formatting checked, clippy with every warning as an error, then the
//! style rules a script can check.

mod comments;
mod concurrency;
mod conditionals;
mod dependencies;
mod hygiene;
mod layout;
mod parsed;
mod test_code;
#[cfg(test)]
mod tree;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

const SHOWN: usize = 10;

const SCANNED: [&str; 3] = ["src", "tests", "crates/writebook-dev"];

struct Check {
    name: &'static str,
    run: fn() -> Option<Refusal>,
}

struct Refusal {
    rule: String,
    violations: Vec<String>,
}

macro_rules! checks {
    ($($module:ident::$name:ident),* $(,)?) => {
        vec![$(Check { name: stringify!($name), run: $module::$name }),*]
    };
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let failures: Vec<String> = [formatting(), clippy(), style()]
        .into_iter()
        .filter_map(Result::err)
        .map(|error| error.to_string())
        .collect();

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; ").into())
    }
}

fn formatting() -> Result<(), Box<dyn Error>> {
    cargo(
        &["fmt", "--all", "--check"],
        "the code isn't formatted: run `cargo fmt --all`",
    )
}

fn clippy() -> Result<(), Box<dyn Error>> {
    cargo(
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        "clippy found problems",
    )
}

fn cargo(args: &[&str], failure: &str) -> Result<(), Box<dyn Error>> {
    if Command::new("cargo").args(args).status()?.success() {
        Ok(())
    } else {
        Err(failure.into())
    }
}

fn style() -> Result<(), Box<dyn Error>> {
    let list_all = std::env::var_os("WRITEBOOK_LINT_LIST_ALL").is_some();
    let mut refused = Vec::new();
    for check in checks() {
        if let Some(refusal) = (check.run)() {
            refused.push(refusal.report(check.name, list_all));
        }
    }
    if refused.is_empty() {
        Ok(())
    } else {
        eprintln!("{}", refused.join("\n\n"));
        Err(format!("{} lints failed", refused.len()).into())
    }
}

fn checks() -> Vec<Check> {
    checks![
        comments::no_comment_is_a_banner_a_marker_or_code,
        comments::no_test_holds_a_comment,
        comments::no_test_is_documented_without_a_link,
        comments::no_private_item_is_documented,
        comments::no_comment_tells_history_or_makes_an_excuse,
        concurrency::a_poisoned_lock_is_taken_back,
        conditionals::no_combinator_chain_stands_in_for_a_branch,
        conditionals::a_guard_clause_is_only_the_first_statement_of_a_function,
        dependencies::no_crate_takes_a_mocking_library,
        hygiene::no_rust_file_is_over_a_thousand_lines,
        hygiene::nothing_is_named_for_a_kind_of_code,
        hygiene::every_crate_takes_the_workspace_lints,
        hygiene::every_integration_test_says_it_is_test_code,
        layout::constants_come_before_any_other_code,
        layout::an_impl_starts_with_new_then_associated_functions_then_public_then_private_methods,
        layout::trait_impls_come_after_the_inherent_impl,
        test_code::no_test_sleeps,
        test_code::no_test_reads_how_long_something_took,
        test_code::a_long_tests_module_has_its_own_file,
    ]
}

impl Refusal {
    fn report(&self, name: &str, list_all: bool) -> String {
        let mut shown = SHOWN;
        if list_all {
            shown = self.violations.len();
        }
        format!(
            "{name}: {} {} (WRITEBOOK_LINT_LIST_ALL=1 lists them all):\n{}",
            self.violations.len(),
            self.rule,
            self.violations
                .iter()
                .take(shown)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn root() -> PathBuf {
    std::env::current_dir()
        .and_then(|directory| directory.canonicalize())
        .expect("the working directory, the workspace's root, resolves")
}

fn scanned() -> Vec<PathBuf> {
    let root = root();
    let mut files: Vec<PathBuf> = SCANNED
        .iter()
        .map(|directory| root.join(directory))
        .filter(|directory| directory.is_dir())
        .flat_map(|directory| files_under(&directory))
        .collect();
    files.sort();
    files
}

fn files_under(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory).expect("the directory reads") {
        let path = entry.expect("its entry reads").path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn packages() -> Vec<Value> {
    static PACKAGES: OnceLock<Vec<Value>> = OnceLock::new();
    PACKAGES.get_or_init(read_packages).clone()
}

fn read_packages() -> Vec<Value> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root())
        .output()
        .expect("cargo metadata runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("its output is JSON");
    metadata["packages"].as_array().expect("it lists the packages").clone()
}

fn name(package: &Value) -> String {
    package["name"].as_str().expect("a package has a name").to_string()
}

fn dependencies(package: &Value) -> Vec<String> {
    package["dependencies"]
        .as_array()
        .expect("a package lists its dependencies")
        .iter()
        .map(|dependency| {
            dependency["name"]
                .as_str()
                .expect("a dependency has a name")
                .to_string()
        })
        .collect()
}
