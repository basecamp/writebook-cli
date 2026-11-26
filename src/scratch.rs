//! Folders for tests, deleted when they go out of scope.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempDir(PathBuf);

impl TempDir {
    /// Tests run side by side, so the counter keeps two from getting the same one.
    pub fn new() -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("writebook-unit-test-{}-{count}", std::process::id()));
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("could not clear {}: {error}", dir.display()),
        }
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("could not remove {}: {error}", self.0.display());
        }
    }
}
