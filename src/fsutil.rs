use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = temporary_sibling(path)?;
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Like write_json_privately, for sign-ins and other secrets.
pub fn write_secret_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_private_location(path)?;
    write_json_privately(path, value)
}

#[cfg(unix)]
fn ensure_private_location(_path: &Path) -> Result<()> {
    Ok(())
}

/// A folder in the user's profile only lets that user read it, and the files
/// in it inherit that. Anywhere else, such as a shared XDG_CONFIG_HOME, other
/// accounts might read the secret, so it isn't written there. The folder is
/// checked by its resolved path, so `..` and junctions can't lead out of the
/// profile.
#[cfg(windows)]
fn ensure_private_location(path: &Path) -> Result<()> {
    let parent = path.parent().context("path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let folder = fs::canonicalize(parent)?;
    let home = fs::canonicalize(dirs::home_dir().context("could not determine the home directory")?)?;

    if folder.starts_with(&home) {
        Ok(())
    } else {
        anyhow::bail!("won't save sign-ins in {}: on Windows, writebook keeps them inside your user profile ({})", folder.display(), home.display())
    }
}

pub fn write_json_privately<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let payload = serde_json::to_string_pretty(value)?;
    let temporary = temporary_sibling(path)?;
    create_private(&temporary)?.write_all(payload.as_bytes())?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Created readable only by its owner, so the contents are never readable by
/// others, not even for a moment. A leftover file from an earlier run might
/// have other permissions, so it goes first.
#[cfg(unix)]
fn create_private(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    remove_if_present(path)?;
    Ok(fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?)
}

/// Windows has no file mode. A new file gets the ACL of its folder.
#[cfg(windows)]
fn create_private(path: &Path) -> Result<fs::File> {
    remove_if_present(path)?;
    Ok(fs::OpenOptions::new().write(true).create_new(true).open(path)?)
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
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
