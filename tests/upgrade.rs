//! `writebook upgrade`, with a fake mise on the PATH.
//!
//! One test, run step by step: on Linux, a file that is being written can't
//! be run while another thread starts a process, because the new process
//! briefly holds it open ("Text file busy"). Tests run in parallel threads, so
//! separate tests that write and run executables fail now and then.

#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use support::{stderr, tempdir, writebook};

#[test]
fn upgrades_only_mise_installs_through_mise() {
    let dir = tempdir();
    let mise = fake_mise(&dir, 0);
    let output = writebook(&["upgrade"], &[("PATH", dir.to_str().unwrap())]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("wasn't installed with mise"), "{}", stderr(&output));
    assert!(stderr(&output).contains("mise use -g github:basecamp/writebook-cli"), "{}", stderr(&output));
    assert!(!mise.with_extension("args").exists());

    let executable = install_like_mise(&dir);
    let output = Command::new(&executable).arg("upgrade").env("PATH", &dir).env("WRITEBOOK_TOKEN", "secret").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read_to_string(mise.with_extension("args")).unwrap(), "upgrade github:basecamp/writebook-cli\n");
    assert!(!fs::read_to_string(mise.with_extension("env")).unwrap().contains("secret"), "mise never sees the token");

    fake_mise(&dir, 1);
    let output = Command::new(&executable).arg("upgrade").env("PATH", &dir).output().unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("`mise upgrade github:basecamp/writebook-cli` failed"), "{}", stderr(&output));
}

fn fake_mise(dir: &Path, exit_code: u8) -> PathBuf {
    let mise = dir.join("mise");
    fs::write(&mise, format!("#!/bin/sh\necho \"$@\" > \"$0.args\"\nexport -p > \"$0.env\"\nexit {exit_code}\n")).unwrap();
    fs::set_permissions(&mise, fs::Permissions::from_mode(0o755)).unwrap();
    mise
}

fn install_like_mise(dir: &Path) -> PathBuf {
    let install = dir.join("installs/github-basecamp-writebook-cli/0.1.0");
    fs::create_dir_all(&install).unwrap();
    let executable = install.join("writebook");
    fs::copy(env!("CARGO_BIN_EXE_writebook"), &executable).unwrap();
    executable
}
