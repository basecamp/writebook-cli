use anyhow::{Context, Result};
use serde::Deserialize;

use super::{client, expand_home, text};
use crate::api::{Part, PartData};

/// Everything on a book's edit screen, which is also the new-book screen.
#[derive(Clone)]
pub struct BookForm {
    pub id: Option<u64>,
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub theme: String,
    pub everyone: bool,
    pub cover: Option<String>,
    pub new_cover: Option<String>,
    pub remove_cover: bool,
    pub members: Vec<Member>,
}

#[derive(Deserialize, Clone)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub role: Role,
    #[serde(default)]
    pub can_administer: bool,
    #[serde(default)]
    pub current: bool,
}

impl Member {
    /// Administrators can always edit, and nobody takes their own access away.
    pub fn fixed(&self) -> bool {
        self.can_administer || self.current
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    None,
    Reader,
    Editor,
}

pub fn book_form(host: &str, book_id: Option<u64>) -> Result<BookForm> {
    let path = match book_id {
        Some(id) => format!("/books/{id}/edit.json"),
        None => "/books/new.json".to_string(),
    };
    let value = client(host)?.get(&path)?;
    let book = &value["book"];
    let members: Vec<Member> =
        serde_json::from_value(value["users"].clone()).context("could not understand the book's readers")?;

    Ok(BookForm {
        id: book["id"].as_u64(),
        title: text(book, "title").unwrap_or_default(),
        subtitle: text(book, "subtitle").unwrap_or_default(),
        author: text(book, "author").unwrap_or_default(),
        theme: text(book, "theme").unwrap_or_else(|| "blue".to_string()),
        everyone: book["everyone_access"].as_bool().unwrap_or(false),
        cover: book["cover"]["url"].as_str().map(str::to_string),
        new_cover: None,
        remove_cover: false,
        members,
    })
}

/// Create the book or save the changes to it. Returns its id.
pub fn save_book(host: &str, form: &BookForm) -> Result<u64> {
    let everyone = form.everyone.to_string();
    let cover = match &form.new_cover {
        Some(path) => {
            let path = expand_home(path);
            let bytes = std::fs::read(&path).with_context(|| format!("could not read {path}"))?;
            let filename = std::path::Path::new(&path)
                .file_name()
                .and_then(|it| it.to_str())
                .unwrap_or("cover")
                .to_string();
            Some((filename, bytes))
        }
        None => None,
    };
    let ids = |roles: &[Role]| -> Vec<String> {
        form.members
            .iter()
            .filter(|it| roles.contains(&it.role))
            .map(|it| it.id.to_string())
            .collect()
    };
    let (editors, readers) = (ids(&[Role::Editor]), ids(&[Role::Editor, Role::Reader]));

    let mut parts = vec![
        Part {
            name: "book[title]",
            filename: None,
            data: PartData::Text(&form.title),
        },
        Part {
            name: "book[subtitle]",
            filename: None,
            data: PartData::Text(&form.subtitle),
        },
        Part {
            name: "book[author]",
            filename: None,
            data: PartData::Text(&form.author),
        },
        Part {
            name: "book[theme]",
            filename: None,
            data: PartData::Text(&form.theme),
        },
        Part {
            name: "book[everyone_access]",
            filename: None,
            data: PartData::Text(&everyone),
        },
    ];
    parts.extend(editors.iter().map(|id| Part {
        name: "editor_ids[]",
        filename: None,
        data: PartData::Text(id),
    }));
    parts.extend(readers.iter().map(|id| Part {
        name: "reader_ids[]",
        filename: None,
        data: PartData::Text(id),
    }));

    if let Some((filename, bytes)) = &cover {
        parts.push(Part {
            name: "book[cover]",
            filename: Some(filename),
            data: PartData::Bytes(bytes),
        });
    } else if form.remove_cover {
        parts.push(Part {
            name: "remove_cover",
            filename: None,
            data: PartData::Text("true"),
        });
    }

    let mut client = client(host)?;

    match form.id {
        Some(id) => {
            client.patch_multipart(&format!("/books/{id}.json"), &parts)?;
            Ok(id)
        }
        None => {
            let response = client.post_multipart("/books.json", &parts)?;
            response["id"]
                .as_u64()
                .context("Writebook's answer had no id for the new book")
        }
    }
}

pub fn delete_book(host: &str, book_id: u64) -> Result<()> {
    client(host)?.delete(&format!("/books/{book_id}.json"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_who_administers_the_instance_from_the_book_form() {
        let members: Vec<Member> = serde_json::from_value(json!([
            { "id": 1, "name": "Ann", "role": "editor", "can_administer": true, "current": false },
            { "id": 2, "name": "Bob", "role": "reader" }
        ]))
        .unwrap();

        assert!(members[0].can_administer);
        assert!(members[0].fixed());
        assert!(!members[1].can_administer);
        assert!(!members[1].fixed());
    }
}
