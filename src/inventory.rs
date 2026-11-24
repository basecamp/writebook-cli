//! The Writebook instances this machine knows about, kept in
//! ~/.config/writebook/instances.toml. Each instance has a URL, an alias to
//! refer to it by, and the profiles (separate sign-ins) used with it. One
//! instance can be the default, used whenever a command isn't told which.
//!
//! An instance doesn't have to be in the inventory to be used: a URL works
//! anywhere an instance is asked for, signed in as the "default" profile.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::{creds, fsutil, web};

pub const DEFAULT_PROFILE: &str = "default";

static SELECTED_PROFILE: OnceLock<String> = OnceLock::new();

#[derive(Serialize, Deserialize, Default)]
pub struct Inventory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub instances: BTreeMap<String, Instance>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Instance {
    pub url: String,
    pub default_profile: String,
    pub profiles: Vec<String>,
}

impl Inventory {
    pub fn load() -> Result<Inventory> {
        let path = path();

        if path.exists() {
            let contents = fsutil::read_to_string(&path)?;
            toml::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
        } else {
            Ok(Inventory::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        fsutil::write_atomically(&path(), toml::to_string(self)?.as_bytes())
    }

    /// The alias and instance for --instance (an alias, a URL, or a bare host
    /// name), or the default instance when it wasn't given.
    pub fn find(&self, instance: Option<&str>) -> Result<(&String, &Instance)> {
        let name = match instance.or(self.default.as_deref()) {
            Some(name) => name,
            None => bail!("which instance? Pass --instance, or add one with `writebook instance add <url>`"),
        };

        self.lookup(name).with_context(|| format!("there is no instance '{name}'. See them with `writebook instance list`"))
    }

    pub fn find_mut(&mut self, instance: Option<&str>) -> Result<(String, &mut Instance)> {
        let alias = self.find(instance)?.0.clone();
        let found = self.instances.get_mut(&alias).expect("the instance was just found");
        Ok((alias, found))
    }

    pub fn add_profile(&mut self, alias: &str, profile: &str) {
        let instance = self.instances.get_mut(alias).expect("the instance is in the inventory");

        if !instance.profiles.iter().any(|it| it == profile) {
            instance.profiles.push(profile.to_string());
        }
    }

    pub fn lookup(&self, name: &str) -> Option<(&String, &Instance)> {
        self.instances.get_key_value(name).or_else(|| self.for_host(&host_of(name).ok()?))
    }

    fn for_host(&self, host: &str) -> Option<(&String, &Instance)> {
        self.instances.iter().find(|(_, instance)| instance.url == host)
    }
}

/// Use this profile instead of each instance's default one.
pub fn select_profile(profile: Option<&str>) {
    if let Some(profile) = profile.map(str::trim).filter(|it| !it.is_empty()) {
        let _ = SELECTED_PROFILE.set(profile.to_string());
    }
}

/// The site address for --instance. A URL needn't be in the inventory.
pub fn host(instance: Option<&str>) -> Result<String> {
    let inventory = Inventory::load()?;

    match instance {
        Some(name) if name.contains("://") && inventory.lookup(name).is_none() => host_of(name),
        _ => Ok(inventory.find(instance)?.1.url.clone()),
    }
}

/// The profile to sign in to this site as: the one picked with --profile, or
/// the instance's default.
pub fn profile_for(host: &str) -> Result<String> {
    let inventory = Inventory::load()?;
    let selected = SELECTED_PROFILE.get();

    match (inventory.for_host(host), selected) {
        (Some((alias, instance)), Some(profile)) => {
            if instance.profiles.contains(profile) {
                Ok(profile.clone())
            } else {
                bail!("instance '{alias}' has no profile '{profile}'. Add it with `writebook profile add {profile} --instance {alias}`")
            }
        }
        (Some((_, instance)), None) => Ok(instance.default_profile.clone()),
        (None, Some(profile)) => Ok(profile.clone()),
        (None, None) => Ok(DEFAULT_PROFILE.to_string()),
    }
}

/// The command that signs in to this site as this profile.
pub fn sign_in_command(host: &str, profile: &str) -> String {
    let inventory = Inventory::load().unwrap_or_default();

    let (instance, default_profile) = match inventory.for_host(host) {
        Some((alias, instance)) => (alias.as_str(), instance.default_profile.as_str()),
        None => (host, DEFAULT_PROFILE),
    };

    if profile == default_profile {
        format!("writebook instance add {instance}")
    } else {
        format!("writebook instance add {instance} --profile {profile}")
    }
}

/// The canonical site address in a URL, like https://books.example.com. A
/// bare host name is taken to be https.
pub fn host_of(url: &str) -> Result<String> {
    if url.contains("://") {
        Ok(web::parse_target(url)?.host)
    } else {
        Ok(web::parse_target(&format!("https://{url}"))?.host)
    }
}

fn path() -> PathBuf {
    creds::config_dir().join("instances.toml")
}
