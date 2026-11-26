//! Where sign-ins are kept: the system keyring (macOS Keychain, the Windows
//! Credential Manager, or the Secret Service on Linux) when there is one,
//! otherwise credentials.json in the config dir, readable only by you.
//! WRITEBOOK_NO_KEYRING skips the keyring.
//!
//! Each sign-in is stored under "<site> <profile>", like
//! "https://books.example.com default".
//!
//! credentials.json also lists which sign-ins are in the keyring, so a
//! command that can't reach the keyring (say, over SSH) says so instead of
//! acting signed out, and signing out leaves none behind there.
//!
//! A sign-in in the file is always the newest: it's saved there only while
//! the keyring is out of reach, and it moves into the keyring, replacing
//! what was there, the first time the keyring is there to take it.
//!
//! Every change happens under sign-ins.lock, so commands running at the same
//! time can't undo each other's changes.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use super::Credentials;
use crate::{config, fsutil, inventory};

const SERVICE: &str = "writebook";
const AVAILABILITY_PROBE: &str = "writebook::availability";

static KEYRING_REACHABLE: OnceLock<bool> = OnceLock::new();

thread_local! {
    static LOCKED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default)]
    sign_ins: BTreeMap<String, Credentials>,
    #[serde(default)]
    in_keyring: BTreeSet<String>,
}

pub fn load(host: &str, profile: &str) -> Result<Option<Credentials>> {
    locked(|| {
        let key = key(host, profile);
        let mut file = load_file()?;

        if let Some(credentials) = file.sign_ins.get(&key).cloned() {
            if keyring_reachable() {
                set_in_keyring(&key, &credentials)?;
                file.sign_ins.remove(&key);
                file.in_keyring.insert(key);
                save_file(&file)?;
            }

            Ok(Some(credentials))
        } else if keyring_reachable() {
            let found = get_from_keyring(&key)?;

            if found.is_some() != file.in_keyring.contains(&key) {
                if found.is_some() {
                    file.in_keyring.insert(key);
                } else {
                    file.in_keyring.remove(&key);
                }
                save_file(&file)?;
            }

            Ok(found)
        } else if file.in_keyring.contains(&key) {
            Err(out_of_reach(host, profile))
        } else {
            Ok(None)
        }
    })
}

pub fn save(host: &str, profile: &str, credentials: &Credentials) -> Result<()> {
    locked(|| {
        let key = key(host, profile);
        let mut file = load_file()?;

        if keyring_reachable() {
            set_in_keyring(&key, credentials)?;
            file.sign_ins.remove(&key);
            file.in_keyring.insert(key);
        } else {
            if std::env::var_os("WRITEBOOK_NO_KEYRING").is_none() {
                eprintln!(
                    "warning: no system keyring, so the sign-in is kept in {}",
                    file_path()?.display()
                );
            }
            file.sign_ins.insert(key, credentials.clone());
        }

        save_file(&file)
    })
}

/// Fails, changing nothing, when the sign-in is in a keyring this command
/// can't reach, so it isn't left behind there.
pub fn delete(host: &str, profile: &str) -> Result<()> {
    locked(|| {
        let key = key(host, profile);
        let mut file = load_file()?;

        if keyring_reachable() {
            match entry(&key)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("could not remove {key} from the system keyring"));
                }
            }
        } else if file.in_keyring.contains(&key) {
            return Err(out_of_reach(host, profile));
        }

        file.sign_ins.remove(&key);
        file.in_keyring.remove(&key);
        save_file(&file)
    })
}

/// Run with sign-ins.lock held. Nested calls on the same thread share it.
pub fn locked<T>(work: impl FnOnce() -> Result<T>) -> Result<T> {
    if LOCKED.get() {
        return work();
    }

    let lock = config::lock_file("sign-ins.lock")?;
    lock.lock().context("could not lock the sign-ins")?;
    LOCKED.set(true);
    let result = work();
    LOCKED.set(false);
    result
}

fn out_of_reach(host: &str, profile: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "the sign-in to {host} is in the system keyring, which this command can't use. \
         Run it where the keyring works, sign in again here with `{}`, or set WRITEBOOK_TOKEN",
        inventory::sign_in_command(host, profile)
    )
}

fn get_from_keyring(key: &str) -> Result<Option<Credentials>> {
    match entry(key)?.get_password() {
        Ok(json) => {
            Ok(Some(serde_json::from_str(&json).with_context(|| {
                format!("could not parse the keyring entry for {key}")
            })?))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {key} from the system keyring")),
    }
}

fn set_in_keyring(key: &str, credentials: &Credentials) -> Result<()> {
    entry(key)?
        .set_password(&serde_json::to_string(credentials)?)
        .with_context(|| format!("could not save {key} to the system keyring"))
}

fn key(host: &str, profile: &str) -> String {
    format!("{host} {profile}")
}

fn keyring_reachable() -> bool {
    *KEYRING_REACHABLE.get_or_init(|| std::env::var_os("WRITEBOOK_NO_KEYRING").is_none() && keyring_works())
}

fn keyring_works() -> bool {
    matches!(
        entry(AVAILABILITY_PROBE).map(|it| it.get_password()),
        Ok(Ok(_) | Err(keyring::Error::NoEntry))
    )
}

fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).with_context(|| format!("could not open the system keyring for {key}"))
}

fn load_file() -> Result<CredentialsFile> {
    let path = file_path()?;

    if path.exists() {
        let contents = fsutil::read_to_string(&path)?;
        serde_json::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
    } else {
        Ok(CredentialsFile::default())
    }
}

fn save_file(file: &CredentialsFile) -> Result<()> {
    fsutil::write_json_privately(&file_path()?, file)
}

fn file_path() -> Result<PathBuf> {
    Ok(config::dir()?.join("credentials.json"))
}
