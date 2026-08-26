use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

pub fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

fn temporary_sibling(path: &Path) -> Result<std::path::PathBuf> {
    let parent = path.parent().context("path has no parent directory")?;
    fs::create_dir_all(parent)?;
    Ok(path.with_extension("tmp"))
}
