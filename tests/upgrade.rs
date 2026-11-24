//! `writebook upgrade`, with a fake mise on the PATH.

#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use support::{stderr, tempdir, writebook};

#[test]
fn upgrades_a_mise_install_through_mise() {
    let dir = tempdir();
    let mise = fake_mise(&dir, 0);
    let executable = install_like_mise(&dir);

    let output = Command::new(&executable).arg("upgrade").env("PATH", &dir).output().unwrap();

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read_to_string(mise.with_extension("args")).unwrap(), "upgrade github:basecamp/writebook-cli\n");
}

#[test]
fn fails_when_mise_fails() {
    let dir = tempdir();
    fake_mise(&dir, 1);
    let executable = install_like_mise(&dir);

    let output = Command::new(&executable).arg("upgrade").env("PATH", &dir).output().unwrap();

    assert!(!output.status.success());
    assert!(stderr(&output).contains("`mise upgrade github:basecamp/writebook-cli` failed"), "{}", stderr(&output));
}

#[test]
fn tells_other_installs_to_use_mise() {
    let dir = tempdir();
    let mise = fake_mise(&dir, 0);

    let output = writebook(&["upgrade"], &[("PATH", dir.to_str().unwrap())]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("wasn't installed with mise"), "{}", stderr(&output));
    assert!(stderr(&output).contains("mise use -g github:basecamp/writebook-cli"), "{}", stderr(&output));
    assert!(!mise.with_extension("args").exists());
}

fn fake_mise(dir: &Path, exit_code: u8) -> std::path::PathBuf {
    let mise = dir.join("mise");
    fs::write(&mise, format!("#!/bin/sh\necho \"$@\" > \"$0.args\"\nexit {exit_code}\n")).unwrap();
    fs::set_permissions(&mise, fs::Permissions::from_mode(0o755)).unwrap();
    mise
}

fn install_like_mise(dir: &Path) -> std::path::PathBuf {
    let install = dir.join("installs/github-basecamp-writebook-cli/0.1.0");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("writebook");
    fs::copy(env!("CARGO_BIN_EXE_writebook"), &executable).unwrap();
    executable
}
