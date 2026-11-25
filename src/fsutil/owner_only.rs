//! Windows has no file mode, so the 0700 and 0600 of Unix become an access
//! list that names only the signed-in user. It's protected, so nothing the
//! folders above allow carries over, and files created inside inherit it.
//! icacls sets it, so the CLI needs no unsafe calls into the Win32 API.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

pub fn restrict(path: &Path) -> Result<()> {
    let inheritance = if path.is_dir() { "(OI)(CI)" } else { "" };
    let output = Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("*{}:{inheritance}F", current_user()?))
        .output()
        .context("could not run icacls")?;

    if output.status.success() {
        Ok(())
    } else {
        bail!("could not restrict {} to the current user: {}", path.display(), String::from_utf8_lossy(&output.stderr).trim())
    }
}

/// The signed-in user's SID, as in S-1-5-21-…, from whoami's
/// `"name","sid"` line.
fn current_user() -> Result<&'static str> {
    static USER: OnceLock<String> = OnceLock::new();

    if let Some(user) = USER.get() {
        return Ok(user);
    }

    let output = Command::new("whoami").args(["/user", "/fo", "csv", "/nh"]).output().context("could not run whoami")?;
    let line = String::from_utf8_lossy(&output.stdout);
    let sid = line.trim().rsplit(',').next().unwrap_or_default().trim_matches('"');

    if output.status.success() && sid.starts_with("S-") {
        Ok(USER.get_or_init(|| sid.to_string()))
    } else {
        bail!("could not look up the current user")
    }
}
