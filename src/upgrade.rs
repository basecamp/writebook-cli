//! Upgrade through mise, the only install method with releases to upgrade to.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

const MISE_TOOL: &str = "github:basecamp/writebook-cli";

pub fn run() -> Result<()> {
    let executable = std::env::current_exe().context("could not find the writebook executable")?;

    if installed_by_mise(&executable) {
        upgrade_with_mise()
    } else {
        bail!(
            "{} wasn't installed with mise, so it can't upgrade itself. Install it with mise instead:\n\n  mise use -g {MISE_TOOL}",
            executable.display()
        )
    }
}

fn installed_by_mise(executable: &Path) -> bool {
    executable.components().any(|component| component.as_os_str() == "github-basecamp-writebook-cli")
}

fn upgrade_with_mise() -> Result<()> {
    let status = Command::new("mise")
        .args(["upgrade", MISE_TOOL])
        .status()
        .context("could not run mise. Is it on your PATH?")?;

    if status.success() {
        Ok(())
    } else {
        bail!("`mise upgrade {MISE_TOOL}` failed")
    }
}
