#![cfg(test)]
#![cfg(unix)]

use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

const CARGO: &str = include_str!("test_run/cargo.sh");

struct Run {
    output: Output,
    arguments: Vec<String>,
}

fn test_run(ci: bool, cargo_status: i32) -> Run {
    use std::os::unix::fs::PermissionsExt;

    let bin = TempDir::new().expect("a temporary directory");
    let log = TempDir::new().expect("a temporary directory");
    let cargo = bin.path().join("cargo");
    fs::write(&cargo, CARGO).expect("the script is written");
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).expect("the script is executable");
    fs::write(log.path().join("cargo.status"), cargo_status.to_string()).expect("the status is written");

    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").expect("PATH is set")
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_writebook-dev"));
    command
        .args(["test", "run"])
        .env("PATH", path)
        .env("LOG", log.path())
        .env_remove("CI");
    if ci {
        command.env("CI", "true");
    }
    let output = command.output().expect("writebook-dev runs");

    let arguments = fs::read_to_string(log.path().join("cargo.args"))
        .expect("cargo was run")
        .lines()
        .map(String::from)
        .collect();
    Run { output, arguments }
}

#[test]
fn runs_the_whole_workspace_under_nextest_with_the_lockfile() {
    let run = test_run(false, 0);

    assert!(
        run.output.status.success(),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.arguments, ["nextest", "run", "--workspace", "--locked"]);
}

#[test]
fn runs_the_ci_profile_on_ci() {
    let run = test_run(true, 0);

    assert!(
        run.output.status.success(),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(
        run.arguments,
        ["nextest", "run", "--workspace", "--locked", "--profile", "ci"]
    );
}

#[test]
fn fails_when_the_tests_fail() {
    let run = test_run(false, 1);

    assert!(!run.output.status.success());
    assert!(
        String::from_utf8_lossy(&run.output.stderr).contains("the tests failed"),
        "{}",
        String::from_utf8_lossy(&run.output.stderr)
    );
    assert_eq!(run.arguments, ["nextest", "run", "--workspace", "--locked"]);
}
