//! A checkout on disk: the working files (book.md, index.md, pages, assets/)
//! plus the .writebook directory holding remotes, sync state, and the base
//! snapshot that three-way merges use as their common ancestor.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::fsutil;

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub remotes: BTreeMap<String, Remote>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Remote {
    pub url: String,
    pub book_id: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct State {
    pub version: u32,
    pub book_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<CoverState>,
    #[serde(default)]
    pub leaves: Vec<LeafState>,
    /// Embedded page uploads living in assets/, keyed by their server slug
    /// (which is also their filename), valued by the sha256 of their bytes.
    #[serde(default)]
    pub uploads: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CoverState {
    pub filename: String,
    pub remote_checksum: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LeafState {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_remote_checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_sha256: Option<String>,
}

impl LeafState {
    /// The index.md line this leaf had at last sync. Imageless pictures have
    /// no line at all.
    pub fn entry(&self) -> Option<crate::index::Entry> {
        match self.kind.as_str() {
            "Page" => self
                .filename
                .clone()
                .map(|filename| crate::index::Entry::Page { title: self.title.clone(), filename }),
            "Section" => Some(crate::index::Entry::Section { title: self.title.clone(), filename: self.filename.clone() }),
            "Picture" => self
                .filename
                .clone()
                .map(|filename| crate::index::Entry::Picture { caption: self.title.clone(), filename }),
            _ => None,
        }
    }
}

pub struct Checkout {
    pub root: PathBuf,
}

impl Checkout {
    pub fn find() -> Result<Checkout> {
        let start = std::env::current_dir().context("could not determine the current directory")?;
        let mut directory = start.as_path();

        loop {
            if directory.join(".writebook").is_dir() {
                return Ok(Checkout { root: directory.to_path_buf() });
            }

            match directory.parent() {
                Some(parent) => directory = parent,
                None => bail!("not inside a writebook checkout — run `writebook clone <url>` first"),
            }
        }
    }

    pub fn create(root: &Path) -> Result<Checkout> {
        fs::create_dir_all(root.join(".writebook/base"))?;
        fs::create_dir_all(root.join("assets"))?;

        let checkout = Checkout { root: root.to_path_buf() };
        checkout.ignore_metadata_in_git()?;
        Ok(checkout)
    }

    pub fn config(&self) -> Result<Config> {
        read_json_or_default(&self.metadata_path("config.json"))
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        fsutil::write_json_privately(&self.metadata_path("config.json"), config)
    }

    pub fn state(&self) -> Result<State> {
        read_json_or_default(&self.metadata_path("state.json"))
    }

    pub fn save_state(&self, state: &State) -> Result<()> {
        fsutil::write_json_privately(&self.metadata_path("state.json"), state)
    }

    pub fn origin(&self) -> Result<Remote> {
        let config = self.config()?;

        config
            .remotes
            .get("origin")
            .cloned()
            .context("this checkout has no origin remote — add one with `writebook remote add origin <url>`")
    }

    pub fn working_path(&self, filename: &str) -> PathBuf {
        self.root.join(filename)
    }

    pub fn base_path(&self, filename: &str) -> PathBuf {
        self.root.join(".writebook/base").join(filename)
    }

    pub fn read_working(&self, filename: &str) -> Result<String> {
        fsutil::read_to_string(&self.working_path(filename))
    }

    pub fn read_base(&self, filename: &str) -> Result<String> {
        fsutil::read_to_string(&self.base_path(filename))
    }

    pub fn write_working(&self, filename: &str, contents: &str) -> Result<()> {
        fsutil::write_atomically(&self.working_path(filename), contents.as_bytes())
    }

    pub fn write_base(&self, filename: &str, contents: &str) -> Result<()> {
        fsutil::write_atomically(&self.base_path(filename), contents.as_bytes())
    }

    pub fn remove_base(&self, filename: &str) -> Result<()> {
        let path = self.base_path(filename);

        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("could not remove {}", path.display()))?;
        }

        Ok(())
    }

    fn metadata_path(&self, filename: &str) -> PathBuf {
        self.root.join(".writebook").join(filename)
    }

    fn ignore_metadata_in_git(&self) -> Result<()> {
        let path = self.root.join(".gitignore");

        let existing = if path.exists() {
            fsutil::read_to_string(&path)?
        } else {
            String::new()
        };

        if existing.lines().any(|line| line.trim() == ".writebook/" || line.trim() == ".writebook") {
            return Ok(());
        }

        let mut contents = existing;

        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }

        contents.push_str(".writebook/\n");
        fsutil::write_atomically(&path, contents.as_bytes())
    }
}

fn read_json_or_default<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    if path.exists() {
        let contents = fsutil::read_to_string(path)?;
        serde_json::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
    } else {
        Ok(T::default())
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct BookMeta {
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub theme: String,
}

pub fn parse_book_md(text: &str) -> BookMeta {
    let mut meta = BookMeta::default();

    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().to_string();

            match key.trim() {
                "title" => meta.title = value,
                "subtitle" => meta.subtitle = value,
                "author" => meta.author = value,
                "theme" => meta.theme = value,
                _ => {}
            }
        }
    }

    meta
}

pub fn serialize_book_md(meta: &BookMeta) -> String {
    format!(
        "---\ntitle: {}\nsubtitle: {}\nauthor: {}\ntheme: {}\n---\n",
        meta.title, meta.subtitle, meta.author, meta.theme
    )
}

pub fn slugify(title: &str) -> String {
    let mut slug = String::new();

    for character in title.to_lowercase().chars() {
        if character.is_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }

    slug.trim_matches('-').to_string()
}

/// A filename nothing else claims yet: the wanted name as-is, or with -2, -3,
/// ... spliced in before the extension.
pub fn unique_filename(wanted: &str, taken: &[String]) -> String {
    if !taken.iter().any(|it| it == wanted) {
        return wanted.to_string();
    }

    let (stem, extension) = match wanted.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (wanted.to_string(), String::new()),
    };

    let mut counter = 2;

    loop {
        let candidate = format!("{stem}-{counter}{extension}");

        if !taken.iter().any(|it| it == &candidate) {
            return candidate;
        }

        counter += 1;
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
