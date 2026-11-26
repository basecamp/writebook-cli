use anyhow::{Context, Result, bail};
use serde_json::json;

use super::{client, people, text};
use crate::api;

/// Your own name and email, as the profile screen edits them.
pub struct Profile {
    pub id: u64,
    pub name: String,
    pub email: String,
}

/// Your own profile on the instance: the person marked as you in its people.
pub fn profile(host: &str) -> Result<Profile> {
    let people = people(host)?;
    let me = people
        .people
        .iter()
        .find(|it| it.current)
        .context("Writebook didn't say which person is you")?;
    let value = client(host)?.get(&format!("/users/{}/profile/edit.json", me.id))?;

    Ok(Profile {
        id: me.id,
        name: text(&value, "name").unwrap_or_default(),
        email: text(&value, "email_address").unwrap_or_default(),
    })
}

/// Save your name, and your email and password when they change, which
/// Writebook only lets a CLI do with your current password.
pub fn save_profile(
    host: &str,
    id: u64,
    name: &str,
    email: &str,
    password: &str,
    current_password: &str,
) -> Result<()> {
    let mut user = json!({ "name": name, "email_address": email, "current_password": current_password });

    if !password.is_empty() {
        user["password"] = json!(password);
    }

    let request = json!({ "user": user });

    match client(host)?.patch(&format!("/users/{id}/profile.json"), &request) {
        Ok(_) => Ok(()),
        Err(error) => match error.downcast_ref::<api::ApiError>() {
            Some(api_error) if api_error.status == 422 => {
                let reasons: Vec<&str> = api_error.body["errors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|it| it.as_str())
                    .collect();
                if reasons.is_empty() {
                    Err(error)
                } else {
                    bail!("{}", reasons.join(". "))
                }
            }
            _ => Err(error),
        },
    }
}
