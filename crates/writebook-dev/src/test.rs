//! `bin/dev test run`: the whole suite under nextest, a process per test.

use std::error::Error;
use std::process::Command;

pub fn run() -> Result<(), Box<dyn Error>> {
    let mut nextest = Command::new("cargo");
    nextest.args(["nextest", "run", "--workspace", "--locked"]);

    if std::env::var_os("CI").is_some() {
        nextest.args(["--profile", "ci"]);
    }

    if nextest.status()?.success() {
        Ok(())
    } else {
        Err("the tests failed".into())
    }
}
