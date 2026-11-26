use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
mod owner_only;

/// Written to a new file next to `path` and renamed over it. A file or symlink
/// planted where the new file goes makes the write fail rather than go through it.
pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = temporary_sibling(path)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?
        .write_all(contents)?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Like `write_atomically`, with the file readable only by its owner from the
/// moment it exists, so its contents are never readable by others.
pub fn write_json_privately<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let payload = serde_json::to_string_pretty(value)?;
    let temporary = temporary_sibling(path)?;
    create_private(&temporary)?.write_all(payload.as_bytes())?;
    fs::rename(&temporary, path).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn create_private(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    Ok(fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}

#[cfg(windows)]
fn create_private(path: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    owner_only::restrict(path)?;
    Ok(file)
}

/// A directory only its owner can enter. One that's already there is made so.
#[cfg(unix)]
pub fn create_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .with_context(|| format!("could not create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("could not restrict {}", path.display()))
}

#[cfg(windows)]
pub fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("could not create {}", path.display()))?;
    owner_only::restrict(path)
}

pub fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

/// A file that isn't there reads as empty.
pub fn read_to_string_or_empty(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

/// Best-effort cleanup: a file that's already gone is fine, and any other
/// failure is a warning, not an error.
pub fn remove_if_present(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("warning: could not remove {}: {error}", path.display()),
    }
}

fn temporary_sibling(path: &Path) -> Result<std::path::PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().context("path has no parent directory")?;
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since_epoch) => since_epoch.subsec_nanos(),
        Err(_before_epoch) => 0,
    };
    let unique = format!(
        ".writebook-{}-{nanos}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );

    fs::create_dir_all(parent)?;
    Ok(parent.join(unique))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::TempDir;

    #[test]
    fn writes_a_file_with_the_longest_name_allowed() {
        let dir = TempDir::new();
        let path = dir.join("a".repeat(255));

        write_atomically(&path, b"picture").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"picture");
    }
}
