use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::Path;

pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = temporary_sibling(path)?;
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

pub fn write_json_privately<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let payload = serde_json::to_string_pretty(value)?;
    let temporary = temporary_sibling(path)?;
    fs::write(&temporary, payload)?;
    make_private(&temporary)?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn make_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    Ok(fs::set_permissions(path, fs::Permissions::from_mode(0o600))?)
}

/// Windows gives files in a user's profile an ACL that only that user can
/// read. Anywhere else, such as a shared XDG_CONFIG_HOME, the file could end
/// up readable by other accounts, so it isn't written there.
#[cfg(windows)]
fn make_private(path: &Path) -> Result<()> {
    let home = dirs::home_dir().context("could not determine the home directory")?;

    if path.starts_with(&home) {
        Ok(())
    } else {
        fs::remove_file(path)?;
        anyhow::bail!("won't write {}: on Windows, writebook keeps sign-ins only inside your user profile ({})", path.display(), home.display())
    }
}

pub fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

fn temporary_sibling(path: &Path) -> Result<std::path::PathBuf> {
    let parent = path.parent().context("path has no parent directory")?;
    fs::create_dir_all(parent)?;
    Ok(path.with_extension("tmp"))
}
