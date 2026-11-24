//! `writebook instance add/list/remove/set-default`: manage the inventory.
//! Adding an instance signs in to it, and removing one signs out.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::creds;
use crate::output::{self, Format};
use crate::inventory::{DEFAULT_PROFILE, Instance, Inventory, host_of};

/// Sign in to an instance and keep it in the inventory. Adding one that's
/// already there signs in to it again, or adds the profile when it's new.
pub fn add(target: &str, alias: Option<&str>, profile: Option<&str>, code: Option<&str>) -> Result<()> {
    let mut inventory = Inventory::load()?;

    match inventory.lookup(target).map(|(alias, instance)| (alias.clone(), instance.clone())) {
        Some((existing, instance)) => {
            if let Some(alias) = alias.filter(|it| *it != existing) {
                bail!("{} is already in the inventory as '{existing}', not '{alias}'", instance.url);
            }

            let profile = profile.unwrap_or(&instance.default_profile);

            if creds::sign_in(&instance.url, profile, code)? {
                inventory.add_profile(&existing, profile);
                inventory.save()?;
                println!("Signed in to {existing} ({}) as {profile}.", instance.url);
            }
        }
        None => {
            let host = host_of(target)?;
            let alias = alias.map(str::to_string).unwrap_or_else(|| host.split("://").nth(1).unwrap_or(&host).to_string());
            let profile = profile.unwrap_or(DEFAULT_PROFILE);

            if alias.contains("://") {
                bail!("'{alias}' looks like a URL; an alias is a short name, like work");
            }

            if inventory.instances.contains_key(&alias) {
                bail!("there already is an instance called '{alias}'. Pick another with --alias");
            }

            if creds::sign_in(&host, profile, code)? {
                inventory.instances.insert(
                    alias.clone(),
                    Instance { url: host.clone(), default_profile: profile.to_string(), profiles: vec![profile.to_string()] },
                );
                inventory.default.get_or_insert_with(|| alias.clone());
                inventory.save()?;
                println!("Added {alias} ({host}), signed in as {profile}.");
            }
        }
    }

    Ok(())
}

#[derive(Serialize)]
struct Row {
    alias: String,
    url: String,
    default: bool,
    profile: String,
    signed_in: bool,
}

impl Row {
    fn alias_cell(&self) -> String {
        if self.default {
            format!("{} (default)", self.alias)
        } else {
            self.alias.clone()
        }
    }

    fn status_cell(&self) -> String {
        if self.signed_in {
            format!("Signed in as {}", self.profile)
        } else {
            "Signed out".to_string()
        }
    }
}

/// A table of instances and whether you're signed in to each, as its
/// default profile.
pub fn list(format: Format) -> Result<()> {
    let inventory = Inventory::load()?;
    let mut rows = Vec::new();

    for (alias, instance) in &inventory.instances {
        rows.push(Row {
            alias: alias.clone(),
            url: instance.url.clone(),
            default: inventory.default.as_ref() == Some(alias),
            profile: instance.default_profile.clone(),
            signed_in: creds::signed_in(&instance.url, &instance.default_profile)?,
        });
    }

    match format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        _ if rows.is_empty() => println!("No instances. Add one with `writebook instance add <url>`."),
        Format::Table => print_table(&rows),
        Format::Markdown => print_markdown(&rows),
    }

    Ok(())
}

fn print_table(rows: &[Row]) {
    let mut cells = vec![["Alias".to_string(), "URL".to_string(), "Status".to_string()]];
    cells.extend(rows.iter().map(|row| [row.alias_cell(), row.url.clone(), row.status_cell()]));
    output::print_table(&cells);
}

fn print_markdown(rows: &[Row]) {
    for row in rows {
        println!("- {}: {} ({})", row.alias_cell(), row.url, row.status_cell());
    }
}

pub fn remove(instance: &str) -> Result<()> {
    let mut inventory = Inventory::load()?;
    let (alias, removed) = inventory.find(Some(instance))?;
    let (alias, removed) = (alias.clone(), removed.clone());

    for profile in &removed.profiles {
        creds::forget(&removed.url, profile)?;
    }

    inventory.instances.remove(&alias);

    if inventory.default.as_ref() == Some(&alias) {
        inventory.default = None;
    }

    inventory.save()?;

    println!("Removed {alias} ({}) and signed out of it.", removed.url);
    Ok(())
}

pub fn set_default(instance: &str) -> Result<()> {
    let mut inventory = Inventory::load()?;
    let alias = inventory.find(Some(instance))?.0.clone();

    inventory.default = Some(alias.clone());
    inventory.save()?;

    println!("{alias} is now the default instance.");
    Ok(())
}
