//! Windows has no file mode, so the 0700 and 0600 of Unix become an access
//! list that names only the signed-in user. It's protected, so nothing the
//! folders above allow carries over, and files created inside inherit it.
//! icacls sets it, so the CLI needs no unsafe calls into the Win32 API.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// In three steps, since icacls given them at once leaves entries behind:
/// drop any added ones, grant the user, then drop the inherited ones. A path
/// already restricted is left as it is: rewriting its list briefly locks out
/// another command reading a file inside it.
pub fn restrict(path: &Path) -> Result<()> {
    let user = current_user()?;
    let (inheritance, ace_inheritance) = if path.is_dir() { ("(OI)(CI)", "OICI") } else { ("", "") };

    if !is_protected_and_only(path, &format!("A;{ace_inheritance};FA;;;{user}"))? {
        icacls(path, &["/reset"])?;
        icacls(path, &["/grant:r", &format!("*{user}:{inheritance}F")])?;
        icacls(path, &["/inheritance:r"])?;
    }
    Ok(())
}

fn is_protected_and_only(path: &Path, entry: &str) -> Result<bool> {
    static SAVES: AtomicUsize = AtomicUsize::new(0);
    let saved = std::env::temp_dir().join(format!(
        "writebook-acl-{}-{}",
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed)
    ));
    icacls(path, &["/save", &saved.display().to_string()])?;
    let bytes = std::fs::read(&saved).with_context(|| format!("could not read {}", saved.display()));
    super::remove_if_present(&saved);

    let units: Vec<u16> = bytes?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let descriptor = String::from_utf16_lossy(&units);

    match descriptor
        .lines()
        .nth(1)
        .and_then(|line| line.trim().strip_prefix("D:"))
    {
        Some(list) => {
            let flags = &list[..list.find('(').unwrap_or(list.len())];
            let entries = list[flags.len()..].split("S:").next().unwrap_or_default();
            Ok(flags.contains('P') && entries == format!("({entry})"))
        }
        None => Ok(false),
    }
}

fn icacls(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("icacls")
        .arg(path)
        .args(args)
        .output()
        .context("could not run icacls")?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if output.status.success() {
        Ok(said)
    } else {
        bail!("could not restrict {} to the current user: {said}", path.display())
    }
}

fn current_user() -> Result<&'static str> {
    static USER: OnceLock<String> = OnceLock::new();

    match USER.get() {
        Some(user) => Ok(user),
        None => {
            let output = Command::new("whoami")
                .args(["/user", "/fo", "csv", "/nh"])
                .output()
                .context("could not run whoami")?;
            let line = String::from_utf8_lossy(&output.stdout);
            let sid = line.trim().rsplit(',').next().unwrap_or_default().trim_matches('"');

            if output.status.success() && sid.starts_with("S-") {
                Ok(USER.get_or_init(|| sid.to_string()))
            } else {
                bail!("could not look up the current user")
            }
        }
    }
}
