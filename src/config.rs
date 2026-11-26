//! The config directory, where sign-ins and the instance list are kept.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use crate::fsutil;

/// The config directory, created readable by you alone if it isn't there yet.
pub fn dir() -> Result<PathBuf> {
    let dir = if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|it| !it.is_empty()) {
        PathBuf::from(dir).join("writebook")
    } else {
        dirs::home_dir()
            .context("could not determine the home directory")?
            .join(".config/writebook")
    };

    fsutil::create_private_dir(&dir)?;
    Ok(dir)
}

/// A lock file in the config directory, created if it isn't there yet.
/// Holding a lock on it keeps other commands out until it's dropped.
pub fn lock_file(name: &str) -> Result<File> {
    let path = dir()?.join(name);

    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("could not open {}", path.display()))
}
