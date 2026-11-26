use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::{client, text};

/// The people with access to an instance, and the link that invites more.
pub struct People {
    pub join_url: Option<String>,
    pub can_administer: bool,
    pub people: Vec<Person>,
}

#[derive(Deserialize, Clone)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub current: bool,
}

impl Person {
    pub fn administrator(&self) -> bool {
        self.role == "administrator"
    }
}

pub fn people(host: &str) -> Result<People> {
    let value = client(host)?.get("/users.json")?;
    let people = serde_json::from_value(value["users"].clone()).context("could not understand the list of people")?;

    Ok(People {
        join_url: text(&value, "join_url"),
        can_administer: value["can_administer"].as_bool().unwrap_or(false),
        people,
    })
}

/// Make a new invite link, which stops the old one working.
pub fn reset_invite(host: &str) -> Result<String> {
    let value = client(host)?.post("/account/join_code.json", &json!({}))?;
    text(&value, "join_url").context("Writebook's answer had no invite link")
}

pub fn set_role(host: &str, person_id: u64, administrator: bool) -> Result<()> {
    let role = if administrator { "administrator" } else { "member" };
    client(host)?.patch(
        &format!("/users/{person_id}.json"),
        &json!({ "user": { "role": role } }),
    )?;
    Ok(())
}

pub fn remove_person(host: &str, person_id: u64) -> Result<()> {
    client(host)?.delete(&format!("/users/{person_id}.json"))?;
    Ok(())
}
