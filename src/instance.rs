//! `writebook instance add/list/remove/set-default`: manage the inventory.
//! Adding an instance signs in to it, and removing one signs out.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::creds;
use crate::output::{self, Breadcrumb, Format};
use crate::inventory::{DEFAULT_PROFILE, Instance, Inventory, host_of};

/// Sign in to an instance and keep it in the inventory. Adding one that's
/// already there signs in to it again, or adds the profile when it's new.
pub fn add(target: &str, alias: Option<&str>, profile: Option<&str>, code: Option<&str>) -> Result<()> {
    let inventory = Inventory::load()?;

    match inventory.lookup(target).map(|(alias, instance)| (alias.clone(), instance.clone())) {
        Some((existing, instance)) => {
            if let Some(alias) = alias.filter(|it| *it != existing) {
                bail!("{} is already in the inventory as '{existing}', not '{alias}'", instance.url);
            }

            let profile = profile.unwrap_or(&instance.default_profile);

            if creds::sign_in(&instance.url, profile, code)? {
                record_sign_in(&existing, &instance.url, profile)?;
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

            ensure_alias_free(&inventory, &alias)?;

            if creds::sign_in(&host, profile, code)? {
                record_sign_in(&alias, &host, profile)?;
                println!("Added {alias} ({host}), signed in as {profile}.");
            }
        }
    }

    Ok(())
}

/// Add the instance and profile after signing in. Signing in can take
/// minutes, so this checks the latest inventory again.
fn record_sign_in(alias: &str, host: &str, profile: &str) -> Result<()> {
    Inventory::update(|inventory| {
        match inventory.instances.get_mut(alias) {
            Some(instance) if instance.url == host => {
                if !instance.profiles.iter().any(|it| it == profile) {
                    instance.profiles.push(profile.to_string());
                }
            }
            Some(_) => bail!("there already is an instance called '{alias}'. Pick another with --alias"),
            None => {
                if let Some((existing, _)) = inventory.instances.iter().find(|(_, instance)| instance.url == host) {
                    bail!("{host} is already in the inventory as '{existing}'");
                }

                inventory.instances.insert(
                    alias.to_string(),
                    Instance { url: host.to_string(), default_profile: profile.to_string(), profiles: vec![profile.to_string()] },
                );
                inventory.default.get_or_insert_with(|| alias.to_string());
            }
        }

        Ok(())
    })
}

fn ensure_alias_free(inventory: &Inventory, alias: &str) -> Result<()> {
    if inventory.instances.contains_key(alias) {
        bail!("there already is an instance called '{alias}'. Pick another with --alias");
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
        Format::Json { jq } => {
            let breadcrumbs = [
                Breadcrumb { action: "books", command: "writebook book list --instance <alias>".to_string(), description: "List an instance's books" },
                Breadcrumb { action: "add", command: "writebook instance add <url>".to_string(), description: "Sign in to another instance" },
                Breadcrumb {
                    action: "set_default",
                    command: "writebook instance set-default <alias>".to_string(),
                    description: "Use an instance when --instance isn't given",
                },
            ];
            output::print_json(&rows, &breadcrumbs, jq.as_deref())?;
        }
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
    let (alias, removed) = Inventory::update(|inventory| {
        let (alias, removed) = inventory.find(Some(instance)).map(|(alias, removed)| (alias.clone(), removed.clone()))?;

        for profile in &removed.profiles {
            creds::forget(&removed.url, profile)?;
        }

        inventory.instances.remove(&alias);

        if inventory.default.as_ref() == Some(&alias) {
            inventory.default = None;
        }

        Ok((alias, removed))
    })?;

    println!("Removed {alias} ({}) and signed out of it.", removed.url);
    Ok(())
}

pub fn set_default(instance: &str) -> Result<()> {
    let alias = Inventory::update(|inventory| {
        let alias = inventory.find(Some(instance))?.0.clone();
        inventory.default = Some(alias.clone());
        Ok(alias)
    })?;

    println!("{alias} is now the default instance.");
    Ok(())
}
