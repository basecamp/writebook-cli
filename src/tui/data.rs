//! What the TUI shows, fetched from the instances in the inventory, and the
//! changes it makes. Everything here blocks, so the TUI runs it on a thread.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{self, Part, PartData};
use crate::creds::{self, Access};
use crate::inventory::{self, Inventory};
use crate::sync;

pub struct Instance {
    pub alias: String,
    pub url: String,
    pub status: String,
}

#[derive(Deserialize, Clone)]
pub struct BookCard {
    pub id: u64,
    pub title: String,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub theme: Option<String>,
    pub cover: Option<File>,
    #[serde(default)]
    pub editable: bool,
}

#[derive(Deserialize, Clone)]
pub struct File {
    pub url: String,
}

pub struct Book {
    pub id: u64,
    pub title: String,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub theme: Option<String>,
    pub cover: Option<String>,
    pub slug: String,
    pub published: bool,
    pub editable: bool,
    pub leaves: Vec<Leaf>,
}

#[derive(Deserialize, Clone)]
pub struct Leaf {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub word_count: Option<u64>,
    pub preview: Option<String>,
    pub image: Option<File>,
}

impl Leaf {
    pub fn is_section(&self) -> bool {
        self.kind == "Section"
    }

    pub fn is_picture(&self) -> bool {
        self.kind == "Picture"
    }
}

pub struct Publication {
    pub published: bool,
    pub slug: String,
    pub url: String,
}

pub enum NewLeaf {
    Page { title: String },
    Section { title: String },
    Picture { caption: String, path: String },
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

        instances.push(Instance { alias: alias.clone(), url: instance.url.clone(), status: status.to_string() });
    }

    Ok(instances)
}

pub fn books(host: &str) -> Result<Vec<BookCard>> {
    let mut client = client(host)?;
    let can_write = creds::can_write(host)?;
    let mut books: Vec<BookCard> = serde_json::from_value(client.get("/books.json")?).context("could not understand the list of books")?;

    for book in &mut books {
        book.editable &= can_write;
    }

    Ok(books)
}

pub fn book(host: &str, book_id: u64) -> Result<Book> {
    let mut client = client(host)?;
    let manifest = client.get(&format!("/books/{book_id}/manifest.json"))?;
    let leaves: Vec<Leaf> = serde_json::from_value(manifest["leaves"].clone()).context("could not understand the book's pages")?;
    let book = &manifest["book"];

    Ok(Book {
        id: book_id,
        title: text(book, "title").unwrap_or_default(),
        subtitle: text(book, "subtitle"),
        author: text(book, "author"),
        theme: text(book, "theme"),
        cover: book["cover"]["url"].as_str().map(str::to_string),
        slug: text(book, "slug").unwrap_or_default(),
        published: book["published"].as_bool().unwrap_or(false),
        editable: book["editable"].as_bool().unwrap_or(false) && creds::can_write(host)?,
        leaves,
    })
}

pub fn image(host: &str, path: &str) -> Result<image::DynamicImage> {
    let bytes = client(host)?.download(path)?;
    image::load_from_memory(&bytes).context("could not read the image")
}

pub fn add(host: &str, book_id: u64, leaf: &NewLeaf) -> Result<()> {
    let mut client = client(host)?;

    match leaf {
        NewLeaf::Page { title } => {
            client.post(&format!("/books/{book_id}/pages.json"), &json!({ "leaf": { "title": title }, "page": { "body": "" } }))?;
        }
        NewLeaf::Section { title } => {
            client.post(&format!("/books/{book_id}/sections.json"), &json!({ "leaf": { "title": title }, "section": { "body": "" } }))?;
        }
        NewLeaf::Picture { caption, path } => {
            let path = shellexpand(path);
            let bytes = std::fs::read(&path).with_context(|| format!("could not read {path}"))?;
            let filename = std::path::Path::new(&path).file_name().and_then(|it| it.to_str()).unwrap_or("picture");

            client.post_multipart(
                &format!("/books/{book_id}/pictures.json"),
                &[
                    Part { name: "leaf[title]", filename: None, data: PartData::Text(caption) },
                    Part { name: "picture[caption]", filename: None, data: PartData::Text(caption) },
                    Part { name: "picture[image]", filename: Some(filename), data: PartData::Bytes(&bytes) },
                ],
            )?;
        }
    }

    Ok(())
}

pub fn delete(host: &str, book_id: u64, leaf: &Leaf) -> Result<()> {
    client(host)?.delete(&sync::leaf_path(book_id, &leaf.kind, leaf.id))?;
    Ok(())
}

/// Put the book's leaves in this order.
pub fn arrange(host: &str, book_id: u64, order: &[u64]) -> Result<()> {
    client(host)?.post(&format!("/books/{book_id}/leaves/moves.json"), &json!({ "id": order, "position": 0 }))?;
    Ok(())
}

pub fn publish(host: &str, book_id: u64, published: bool, slug: &str) -> Result<Publication> {
    let response = client(host)?.patch(&format!("/books/{book_id}/publication.json"), &json!({ "book": { "published": published, "slug": slug } }))?;

    match (response["published"].as_bool(), text(&response, "slug"), text(&response, "url")) {
        (Some(published), Some(slug), Some(url)) => Ok(Publication { published, slug, url }),
        _ => bail!("{host} didn't say how the book is published now; it may need a newer Writebook"),
    }
}

fn client(host: &str) -> Result<api::Client> {
    Ok(api::Client::with_token(host, token(host)?))
}

fn token(host: &str) -> Result<creds::Token> {
    match creds::signed_in_token(host)? {
        Some(token) => Ok(token),
        None => bail!(
            "you're not signed in to {host}. Sign in with `{}`",
            inventory::sign_in_command(host, &inventory::profile_for(host)?)
        ),
    }
}

fn text(value: &Value, field: &str) -> Option<String> {
    value[field].as_str().map(str::to_string).filter(|it| !it.is_empty())
}

fn shellexpand(path: &str) -> String {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path.to_string(),
    }
}

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
    pub administrator: bool,
    #[serde(default)]
    pub current: bool,
}

impl Member {
    /// Administrators can always edit, and nobody takes their own access away.
    pub fn fixed(&self) -> bool {
        self.administrator || self.current
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
    let members: Vec<Member> = serde_json::from_value(value["users"].clone()).context("could not understand the book's readers")?;

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
            let path = shellexpand(path);
            let bytes = std::fs::read(&path).with_context(|| format!("could not read {path}"))?;
            let filename = std::path::Path::new(&path).file_name().and_then(|it| it.to_str()).unwrap_or("cover").to_string();
            Some((filename, bytes))
        }
        None => None,
    };
    let ids = |roles: &[Role]| -> Vec<String> {
        form.members.iter().filter(|it| roles.contains(&it.role)).map(|it| it.id.to_string()).collect()
    };
    let (editors, readers) = (ids(&[Role::Editor]), ids(&[Role::Editor, Role::Reader]));

    let mut parts = vec![
        Part { name: "book[title]", filename: None, data: PartData::Text(&form.title) },
        Part { name: "book[subtitle]", filename: None, data: PartData::Text(&form.subtitle) },
        Part { name: "book[author]", filename: None, data: PartData::Text(&form.author) },
        Part { name: "book[theme]", filename: None, data: PartData::Text(&form.theme) },
        Part { name: "book[everyone_access]", filename: None, data: PartData::Text(&everyone) },
    ];
    parts.extend(editors.iter().map(|id| Part { name: "editor_ids[]", filename: None, data: PartData::Text(id) }));
    parts.extend(readers.iter().map(|id| Part { name: "reader_ids[]", filename: None, data: PartData::Text(id) }));

    if let Some((filename, bytes)) = &cover {
        parts.push(Part { name: "book[cover]", filename: Some(filename), data: PartData::Bytes(bytes) });
    } else if form.remove_cover {
        parts.push(Part { name: "remove_cover", filename: None, data: PartData::Text("true") });
    }

    let mut client = client(host)?;

    match form.id {
        Some(id) => {
            client.patch_multipart(&format!("/books/{id}.json"), &parts)?;
            Ok(id)
        }
        None => {
            let response = client.post_multipart("/books.json", &parts)?;
            response["id"].as_u64().context("Writebook's answer had no id for the new book")
        }
    }
}

pub fn delete_book(host: &str, book_id: u64) -> Result<()> {
    client(host)?.delete(&format!("/books/{book_id}.json"))?;
    Ok(())
}

/// A page that matched a search, with Writebook's excerpt around the match
/// as HTML, the matched words in <mark> tags.
#[derive(Clone)]
pub struct Found {
    pub book: String,
    pub title: String,
    pub snippet: String,
    pub book_id: Option<u64>,
    pub leaf_id: Option<u64>,
}

/// Plain text as HTML, so a snippet without marks reads like one with them.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Search one book, or every book on the instance.
pub fn search(host: &str, book_id: Option<u64>, query: &str) -> Result<Vec<Found>> {
    let web = crate::web::Web::new(host, Some(token(host)?.value));

    Ok(crate::read::find(&web, book_id, query)?
        .into_iter()
        .map(|hit| {
            let target = crate::web::parse_target(&hit.url).ok();
            Found {
                book: hit.book,
                title: hit.title,
                snippet: hit.marked_snippet.unwrap_or_else(|| escaped(&hit.snippet)),
                book_id: target.as_ref().and_then(|it| it.book_id),
                leaf_id: target.and_then(|it| it.leaf_id),
            }
        })
        .collect())
}

/// One page, section or picture, as the page screen shows it.
#[derive(Clone)]
pub struct LeafContent {
    pub title: String,
    pub body: String,
    pub image: Option<String>,
    pub fingerprint: String,
    /// Where files uploaded into a page's body belong.
    pub record_gid: Option<String>,
}

pub fn leaf(host: &str, book_id: u64, leaf: &Leaf) -> Result<LeafContent> {
    let value = client(host)?.get(&sync::leaf_path(book_id, &leaf.kind, leaf.id))?;
    let body = match leaf.kind.as_str() {
        "Picture" => text(&value, "caption"),
        _ => value["body"].as_str().map(str::to_string),
    };

    Ok(LeafContent {
        title: value["title"].as_str().unwrap_or_default().to_string(),
        body: body.unwrap_or_default(),
        image: value["image"]["url"].as_str().map(str::to_string),
        fingerprint: value["fingerprint"].as_str().unwrap_or_default().to_string(),
        record_gid: text(&value, "record_gid"),
    })
}

/// Save a page's or section's title and text, unless it changed since it was
/// read. Returns the new fingerprint.
pub fn save_leaf(host: &str, book_id: u64, leaf: &Leaf, title: &str, body: &str, fingerprint: &str) -> Result<String> {
    let kind = if leaf.is_section() { "section" } else { "page" };
    let request = json!({ "leaf": { "title": title }, kind: { "body": body }, "base_fingerprint": fingerprint });

    match client(host)?.patch(&sync::leaf_path(book_id, &leaf.kind, leaf.id), &request) {
        Ok(response) => Ok(response["fingerprint"].as_str().unwrap_or_default().to_string()),
        Err(error) if error.downcast_ref::<api::ApiError>().is_some_and(|it| it.status == 409) => {
            bail!("someone changed this page while you were editing it, so it wasn't saved")
        }
        Err(error) => Err(error),
    }
}

/// Upload a file into a page, and return the Markdown that shows it: an
/// image for pictures, a link for anything else.
pub fn upload(host: &str, record_gid: &str, path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    let name = path.file_name().and_then(|it| it.to_str()).unwrap_or("file").to_string();

    let response = client(host)?.post_multipart(
        "/uploads",
        &[
            Part { name: "record_gid", filename: None, data: PartData::Text(record_gid) },
            Part { name: "attribute_name", filename: None, data: PartData::Text("body") },
            Part { name: "file", filename: Some(&name), data: PartData::Bytes(&bytes) },
        ],
    )?;
    let url = response["fileUrl"].as_str().context("Writebook's answer had no address for the file")?;
    let image = image::ImageFormat::from_path(path).is_ok();

    Ok(if image { format!("![{name}]({url})") } else { format!("[{name}]({url})") })
}

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

    Ok(People { join_url: text(&value, "join_url"), can_administer: value["can_administer"].as_bool().unwrap_or(false), people })
}

/// Make a new invite link, which stops the old one working.
pub fn reset_invite(host: &str) -> Result<String> {
    let value = client(host)?.post("/account/join_code.json", &json!({}))?;
    text(&value, "join_url").context("Writebook's answer had no invite link")
}

pub fn set_role(host: &str, person_id: u64, administrator: bool) -> Result<()> {
    let role = if administrator { "administrator" } else { "member" };
    client(host)?.patch(&format!("/users/{person_id}.json"), &json!({ "user": { "role": role } }))?;
    Ok(())
}

pub fn remove_person(host: &str, person_id: u64) -> Result<()> {
    client(host)?.delete(&format!("/users/{person_id}.json"))?;
    Ok(())
}

/// Your own name and email, as the profile screen edits them.
pub struct Profile {
    pub id: u64,
    pub name: String,
    pub email: String,
}

/// Your own profile on the instance: the person marked as you in its people.
pub fn profile(host: &str) -> Result<Profile> {
    let people = people(host)?;
    let me = people.people.iter().find(|it| it.current).context("Writebook didn't say which person is you")?;
    let value = client(host)?.get(&format!("/users/{}/profile/edit.json", me.id))?;

    Ok(Profile {
        id: me.id,
        name: text(&value, "name").unwrap_or_default(),
        email: text(&value, "email_address").unwrap_or_default(),
    })
}

/// Save your name, and your email and password when they change, which
/// Writebook only lets a CLI do with your current password.
pub fn save_profile(host: &str, id: u64, name: &str, email: &str, password: &str, current_password: &str) -> Result<()> {
    let mut user = json!({ "name": name, "email_address": email, "current_password": current_password });

    if !password.is_empty() {
        user["password"] = json!(password);
    }

    let request = json!({ "user": user });

    match client(host)?.patch(&format!("/users/{id}/profile.json"), &request) {
        Ok(_) => Ok(()),
        Err(error) => match error.downcast_ref::<api::ApiError>() {
            Some(api_error) if api_error.status == 422 => {
                let reasons: Vec<&str> = api_error.body["errors"].as_array().into_iter().flatten().filter_map(|it| it.as_str()).collect();
                if reasons.is_empty() { Err(error) } else { bail!("{}", reasons.join(". ")) }
            }
            _ => Err(error),
        },
    }
}
