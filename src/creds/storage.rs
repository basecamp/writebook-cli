//! Where sign-ins are kept: the system keyring (macOS Keychain, or the Secret
//! Service on Linux) when there is one, otherwise credentials.json in the
//! config dir, readable only by you. WRITEBOOK_NO_KEYRING skips the keyring.
//!
//! Each sign-in is stored under "<site> <profile>", like
//! "https://books.example.com default".

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::{Credentials, config_dir};
use crate::fsutil;

const SERVICE: &str = "writebook";
const AVAILABILITY_PROBE: &str = "writebook::availability";

static BACKEND: OnceLock<Backend> = OnceLock::new();

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Keyring,
    File,
}

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default)]
    sign_ins: BTreeMap<String, Credentials>,
}

pub fn load(host: &str, profile: &str) -> Result<Option<Credentials>> {
    let key = key(host, profile);

    match backend() {
        Backend::Keyring => match entry(&key)?.get_password() {
            Ok(json) => Ok(Some(serde_json::from_str(&json).with_context(|| format!("could not parse the keyring entry for {key}"))?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).with_context(|| format!("could not read {key} from the system keyring")),
        },
        Backend::File => Ok(load_file()?.sign_ins.remove(&key)),
    }
}

pub fn save(host: &str, profile: &str, credentials: &Credentials) -> Result<()> {
    let key = key(host, profile);

    match backend() {
        Backend::Keyring => entry(&key)?
            .set_password(&serde_json::to_string(credentials)?)
            .with_context(|| format!("could not save {key} to the system keyring")),
        Backend::File => {
            let mut file = load_file()?;
            file.sign_ins.insert(key, credentials.clone());
            fsutil::write_json_privately(&file_path(), &file)
        }
    }
}

pub fn delete(host: &str, profile: &str) -> Result<()> {
    let key = key(host, profile);

    match backend() {
        Backend::Keyring => match entry(&key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).with_context(|| format!("could not remove {key} from the system keyring")),
        },
        Backend::File => {
            let mut file = load_file()?;

            if file.sign_ins.remove(&key).is_some() {
                fsutil::write_json_privately(&file_path(), &file)?;
            }

            Ok(())
        }
    }
}

fn key(host: &str, profile: &str) -> String {
    format!("{host} {profile}")
}

fn backend() -> Backend {
    *BACKEND.get_or_init(|| {
        if std::env::var_os("WRITEBOOK_NO_KEYRING").is_some() {
            Backend::File
        } else if keyring_works() {
            Backend::Keyring
        } else {
            eprintln!("warning: no system keyring, so sign-ins are kept in {}", file_path().display());
            Backend::File
        }
    })
}

fn keyring_works() -> bool {
    matches!(entry(AVAILABILITY_PROBE).map(|it| it.get_password()), Ok(Ok(_) | Err(keyring::Error::NoEntry)))
}

fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).with_context(|| format!("could not open the system keyring for {key}"))
}

fn load_file() -> Result<CredentialsFile> {
    let path = file_path();

    if path.exists() {
        let contents = fsutil::read_to_string(&path)?;
        serde_json::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
    } else {
        Ok(CredentialsFile::default())
    }
}

fn file_path() -> PathBuf {
    config_dir().join("credentials.json")
}
