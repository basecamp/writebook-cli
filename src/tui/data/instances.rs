use anyhow::Result;

use crate::creds::{self, Access};
use crate::inventory::Inventory;

pub struct Instance {
    pub alias: String,
    pub url: String,
    pub status: String,
}

pub fn instances() -> Result<Vec<Instance>> {
    let inventory = Inventory::load()?;
    let mut instances = Vec::new();

    for (alias, instance) in &inventory.instances {
        let status = match creds::stored_access(&instance.url, &instance.default_profile)? {
            Some(Access::Full) => "signed in",
            Some(Access::ReadOnly) => "signed in, read-only",
            None => "not signed in",
        };

        instances.push(Instance {
            alias: alias.clone(),
            url: instance.url.clone(),
            status: status.to_string(),
        });
    }

    Ok(instances)
}
