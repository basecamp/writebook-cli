use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn write_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = temporary_sibling(path)?;
    fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?.write_all(contents)?;
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
/// others, not even for a moment.
#[cfg(unix)]
fn create_private(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    Ok(fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?)
}

/// Windows has no file mode. A new file gets the ACL of its folder.
#[cfg(windows)]
fn create_private(path: &Path) -> Result<fs::File> {
    Ok(fs::OpenOptions::new().write(true).create_new(true).open(path)?)
}

/// A directory only its owner can enter. One that's already there is made so.
#[cfg(unix)]
pub fn create_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    fs::DirBuilder::new().recursive(true).mode(0o700).create(path).with_context(|| format!("could not create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).with_context(|| format!("could not restrict {}", path.display()))
}

#[cfg(windows)]
pub fn create_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("could not create {}", path.display()))
}

pub fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

/// A name nobody else has picked, next to the file it will replace. It's
/// created with create_new, so a file or symlink someone planted under that
/// name makes the write fail rather than go through it.
fn temporary_sibling(path: &Path) -> Result<std::path::PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().context("path has no parent directory")?;
    let name = path.file_name().context("path has no file name")?.to_string_lossy();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|it| it.subsec_nanos()).unwrap_or_default();
    let unique = format!(".{name}.{}-{nanos}-{}.tmp", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));

    fs::create_dir_all(parent)?;
    Ok(parent.join(unique))
}
