//! The tools `writebook mcp` serves, and what each one does.

use anyhow::{Context, Result, bail};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{Value, json};
use std::sync::Arc;

use super::Connection;
use crate::api::ApiError;
use crate::inventory::Inventory;
use crate::{creds, read, sync};

struct Spec {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    changes_books: bool,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "list_instances",
        description: "List the Writebook instances you added, which one is the default, and whether your sign-in to each can change books.",
        schema: || json!({ "type": "object", "properties": {} }),
        changes_books: false,
    },
    Spec {
        name: "list_books",
        description: "List the books you can read, with their ids, titles, authors and URLs.",
        schema: || json!({ "type": "object", "properties": {} }),
        changes_books: false,
    },
    Spec {
        name: "book_contents",
        description: "List a book's pages, sections and pictures, in order, with their ids and types.",
        schema: || json!({ "type": "object", "properties": { "book_id": book_id() }, "required": ["book_id"] }),
        changes_books: false,
    },
    Spec {
        name: "read_book",
        description: "Read a whole book as Markdown, every page, section and picture caption in order.",
        schema: || json!({ "type": "object", "properties": { "book_id": book_id() }, "required": ["book_id"] }),
        changes_books: false,
    },
    Spec {
        name: "read_page",
        description: "Read one page, section or picture as Markdown. A picture's Markdown is its caption. \
                      The fingerprint is what update_page needs to change it.",
        schema: || {
            json!({
                "type": "object",
                "properties": { "book_id": book_id(), "page_id": page_id() },
                "required": ["book_id", "page_id"],
            })
        },
        changes_books: false,
    },
    Spec {
        name: "search",
        description: "Search every book you can read, or one book. Returns up to 50 matches with snippets and URLs.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to look for" },
                    "book_id": { "type": "integer", "description": "Search only this book" },
                },
                "required": ["query"],
            })
        },
        changes_books: false,
    },
    Spec {
        name: "create_page",
        description: "Add a page to the end of a book.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "book_id": book_id(),
                    "title": { "type": "string" },
                    "markdown": { "type": "string", "description": "The page's body" },
                },
                "required": ["book_id", "title", "markdown"],
            })
        },
        changes_books: true,
    },
    Spec {
        name: "update_page",
        description: "Change a page's title, body, or both. Pass the fingerprint read_page gave: if the page changed since, \
                      nothing is changed, and the page has to be read again.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "book_id": book_id(),
                    "page_id": page_id(),
                    "fingerprint": { "type": "string", "description": "From read_page" },
                    "title": { "type": "string", "description": "The new title, if it changes" },
                    "markdown": { "type": "string", "description": "The whole new body, if it changes" },
                },
                "required": ["book_id", "page_id", "fingerprint"],
            })
        },
        changes_books: true,
    },
    Spec {
        name: "create_section",
        description: "Add a section, a title page that divides a book, to the end of a book.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "book_id": book_id(),
                    "title": { "type": "string" },
                    "markdown": { "type": "string", "description": "Text under the section's title, if any" },
                },
                "required": ["book_id", "title"],
            })
        },
        changes_books: true,
    },
];

fn book_id() -> Value {
    json!({ "type": "integer", "description": "From list_books or search" })
}

fn page_id() -> Value {
    json!({ "type": "integer", "description": "The page's, section's or picture's id, from book_contents" })
}

pub fn exists(name: &str) -> bool {
    SPECS.iter().any(|spec| spec.name == name)
}

/// The tools served. From the inventory, each takes the instance to use, and
/// list_instances lists them; a served instance is the only one there is.
pub fn list(read_only: bool, inventory: bool) -> Vec<Tool> {
    SPECS
        .iter()
        .filter(|spec| !(read_only && spec.changes_books))
        .filter(|spec| inventory || spec.name != "list_instances")
        .map(|spec| {
            let mut schema = (spec.schema)();

            if inventory && spec.name != "list_instances" {
                schema["properties"]["instance"] =
                    json!({ "type": "string", "description": "Alias or URL from list_instances (defaults to the default instance)" });
            }

            let Value::Object(schema) = schema else { unreachable!("every schema is an object") };
            let annotations = ToolAnnotations::new().read_only(!spec.changes_books).destructive(false).open_world(false);

            Tool::new(spec.name, spec.description, Arc::new(schema)).annotate(annotations)
        })
        .collect()
}

/// Run a tool. A served instance comes with the caller's connection; without
/// one, the tool reaches the instance it names.
pub fn call(name: &str, arguments: &JsonObject, served: Option<Connection>, read_only: bool) -> Result<Value> {
    let Some(spec) = SPECS.iter().find(|spec| spec.name == name).filter(|spec| served.is_none() || spec.name != "list_instances") else {
        bail!("there is no tool {name}");
    };

    if read_only && spec.changes_books {
        bail!("this server is read-only, so {name} isn't served");
    }

    if name == "list_instances" {
        return instances();
    }

    let connection = match served {
        Some(connection) => connection,
        None => Connection::to_instance(optional_text(arguments, "instance")?.as_deref())?,
    };

    if spec.changes_books {
        connection.ensure_can_write()?;
    }

    let connection = &connection;

    match name {
        "list_books" => Ok(json!(read::list_books(&connection.web()?)?)),
        "book_contents" => Ok(json!(read::book_contents(&connection.web()?, number(arguments, "book_id")?)?)),
        "read_book" => Ok(json!(read::whole_book(&connection.web()?, number(arguments, "book_id")?)?)),
        "read_page" => read_page(connection, number(arguments, "book_id")?, number(arguments, "page_id")?),
        "search" => search(connection, arguments),
        "create_page" => create_page(connection, arguments),
        "update_page" => update_page(connection, arguments),
        "create_section" => create_section(connection, arguments),
        _ => unreachable!("every tool is called"),
    }
}

fn instances() -> Result<Value> {
    let inventory = Inventory::load()?;
    let mut instances = Vec::new();

    for (alias, instance) in &inventory.instances {
        instances.push(json!({
            "alias": alias,
            "url": instance.url,
            "default": inventory.default.as_ref() == Some(alias),
            "can_change_books": creds::can_write(&instance.url)?,
        }));
    }

    Ok(Value::Array(instances))
}

fn read_page(connection: &Connection, book_id: u64, page_id: u64) -> Result<Value> {
    let mut client = connection.client()?;
    let manifest = sync::fetch_manifest(&mut client, book_id)?;

    let Some(leaf) = manifest.leaves.iter().find(|leaf| leaf.id == page_id) else {
        bail!("book {book_id} has no page {page_id}. book_contents lists the ones it has");
    };

    let content = sync::fetch_leaf(&mut client, book_id, &leaf.kind, page_id)?;
    let markdown = match leaf.kind.as_str() {
        "Picture" => content.caption,
        _ => content.body,
    };

    Ok(json!({
        "id": page_id,
        "type": leaf.kind.to_lowercase(),
        "title": content.title,
        "markdown": markdown.unwrap_or_default(),
        "fingerprint": content.fingerprint,
    }))
}

fn search(connection: &Connection, arguments: &JsonObject) -> Result<Value> {
    let query = text(arguments, "query")?;

    if query.trim().is_empty() {
        bail!("nothing to search for");
    }

    let book_id = match arguments.get("book_id") {
        Some(_) => Some(number(arguments, "book_id")?),
        None => None,
    };

    Ok(json!(read::find(&connection.web()?, book_id, &query)?))
}

fn create_page(connection: &Connection, arguments: &JsonObject) -> Result<Value> {
    let book_id = number(arguments, "book_id")?;
    let body = json!({
        "leaf": { "title": text(arguments, "title")? },
        "page": { "body": text(arguments, "markdown")? },
    });

    let response = connection.client()?.post(&format!("/books/{book_id}/pages.json"), &body)?;
    Ok(created(&response))
}

fn update_page(connection: &Connection, arguments: &JsonObject) -> Result<Value> {
    let book_id = number(arguments, "book_id")?;
    let page_id = number(arguments, "page_id")?;
    let mut body = json!({ "base_fingerprint": text(arguments, "fingerprint")? });

    if arguments.contains_key("title") {
        body["leaf"] = json!({ "title": text(arguments, "title")? });
    }

    if arguments.contains_key("markdown") {
        body["page"] = json!({ "body": text(arguments, "markdown")? });
    }

    if body.get("leaf").is_none() && body.get("page").is_none() {
        bail!("say what changes: a title, markdown, or both");
    }

    match connection.client()?.patch(&sync::leaf_path(book_id, "Page", page_id), &body) {
        Ok(response) => Ok(created(&response)),
        Err(error) if error.downcast_ref::<ApiError>().is_some_and(|it| it.status == 409) => {
            bail!("page {page_id} changed since it was read, so nothing was changed. Read it again, then make the change to what it says now")
        }
        Err(error) => Err(error),
    }
}

fn create_section(connection: &Connection, arguments: &JsonObject) -> Result<Value> {
    let book_id = number(arguments, "book_id")?;
    let markdown = match arguments.get("markdown") {
        Some(_) => text(arguments, "markdown")?,
        None => String::new(),
    };
    let body = json!({ "leaf": { "title": text(arguments, "title")? }, "section": { "body": markdown } });

    let response = connection.client()?.post(&format!("/books/{book_id}/sections.json"), &body)?;
    Ok(created(&response))
}

/// What a change hands back: the leaf's id, and its fingerprint for the next change.
fn created(response: &Value) -> Value {
    json!({ "id": response["id"], "fingerprint": response["fingerprint"] })
}

fn number(arguments: &JsonObject, name: &str) -> Result<u64> {
    let value = arguments.get(name).with_context(|| format!("{name} is missing"))?;
    value.as_u64().or_else(|| value.as_str()?.parse().ok()).with_context(|| format!("{name} must be a whole number"))
}

fn optional_text(arguments: &JsonObject, name: &str) -> Result<Option<String>> {
    match arguments.get(name) {
        Some(_) => text(arguments, name).map(Some),
        None => Ok(None),
    }
}

fn text(arguments: &JsonObject, name: &str) -> Result<String> {
    let value = arguments.get(name).with_context(|| format!("{name} is missing"))?;
    value.as_str().map(str::to_string).with_context(|| format!("{name} must be text"))
}
