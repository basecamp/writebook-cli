//! Helpers for the CLI's tests: a fake Writebook, the CLI run with an empty
//! config, and folders that clean up after themselves.

#![allow(
    dead_code,
    unused_imports,
    reason = "each test binary includes this module and uses a different part of it"
)]

mod server;

pub use server::*;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Run the CLI with no terminal attached, an empty config dir, and only the
/// environment given (no WRITEBOOK_TOKEN leaking in from the developer's shell).
pub fn writebook(args: &[&str], env: &[(&str, &str)]) -> Output {
    writebook_in(&std::env::current_dir().unwrap(), args, env)
}

/// Like writebook, run in the given directory.
pub fn writebook_in(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let config = tempdir();
    let mut command = Command::new(env!("CARGO_BIN_EXE_writebook"));

    command
        .current_dir(dir)
        .args(args)
        .env_remove("WRITEBOOK_TOKEN")
        .env_remove("WRITEBOOK_TOKEN_HOST")
        .env_remove("WRITEBOOK_PROFILE")
        .env_remove("WRITEBOOK_NO_SIGN_IN")
        .env_remove("CLAUDECODE")
        .env("XDG_CONFIG_HOME", &config)
        .env("WRITEBOOK_NO_KEYRING", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in env {
        command.env(key, value);
    }

    command.output().unwrap()
}

/// A folder that's deleted when it goes out of scope. Tests run side by side,
/// so the counter keeps two from getting the same one.
pub struct TempDir(PathBuf);

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for TempDir {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_os_str()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("could not remove {}: {error}", self.0.display());
        }
    }
}

pub fn tempdir() -> TempDir {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("writebook-test-{}-{count}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
