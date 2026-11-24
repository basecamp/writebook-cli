//! `writebook profile add/list/remove/set-default`: the sign-ins kept for an
//! instance, like a personal account and an editor account on the same site.
//! Adding a profile signs in as it, and removing one signs it out.

use anyhow::{Result, bail};

use crate::creds;
use crate::inventory::Inventory;

/// Sign in to an instance as a profile and keep it. Adding one that's
/// already there signs in as it again.
pub fn add(name: &str, instance: Option<&str>, code: Option<&str>) -> Result<()> {
    let inventory = Inventory::load()?;
    let (alias, found) = inventory.find(instance)?;
    let (alias, url) = (alias.clone(), found.url.clone());

    if creds::sign_in(&url, name, code)? {
        Inventory::update(|inventory| {
            inventory.add_profile(&alias, name)?;
            Ok(())
        })?;
        println!("Signed in to {alias} ({url}) as {name}.");
    }

    Ok(())
}

pub fn list(instance: Option<&str>) -> Result<()> {
    let inventory = Inventory::load()?;
    let (_, found) = inventory.find(instance)?;

    for profile in &found.profiles {
        if *profile == found.default_profile {
            println!("* {profile}");
        } else {
            println!("  {profile}");
        }
    }

    Ok(())
}

pub fn remove(name: &str, instance: Option<&str>) -> Result<()> {
    let alias = Inventory::update(|inventory| {
        let (alias, found) = inventory.find_mut(instance)?;

        if !found.profiles.iter().any(|it| it == name) {
            bail!("instance '{alias}' has no profile '{name}'");
        }

        if found.default_profile == name {
            bail!("'{name}' is the default profile of '{alias}'. Make another one the default with `writebook profile set-default` first");
        }

        creds::forget(&found.url, name)?;
        found.profiles.retain(|it| it != name);
        Ok(alias)
    })?;

    println!("Removed profile {name} from {alias} and signed it out.");
    Ok(())
}

pub fn set_default(name: &str, instance: Option<&str>) -> Result<()> {
    let alias = Inventory::update(|inventory| {
        let (alias, found) = inventory.find_mut(instance)?;

        if !found.profiles.iter().any(|it| it == name) {
            bail!("instance '{alias}' has no profile '{name}'. Add it with `writebook profile add {name} --instance {alias}`");
        }

        found.default_profile = name.to_string();
        Ok(alias)
    })?;

    println!("{name} is now the default profile of {alias}.");
    Ok(())
}
