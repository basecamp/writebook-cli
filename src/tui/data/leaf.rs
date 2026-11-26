use anyhow::{Context, Result, bail};
use serde_json::json;

use super::{Leaf, client, text};
use crate::api::{self, Part, PartData};
use crate::sync;

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
    let name = path
        .file_name()
        .and_then(|it| it.to_str())
        .unwrap_or("file")
        .to_string();

    let response = client(host)?.post_multipart(
        "/uploads",
        &[
            Part {
                name: "record_gid",
                filename: None,
                data: PartData::Text(record_gid),
            },
            Part {
                name: "attribute_name",
                filename: None,
                data: PartData::Text("body"),
            },
            Part {
                name: "file",
                filename: Some(&name),
                data: PartData::Bytes(&bytes),
            },
        ],
    )?;
    let url = response["fileUrl"]
        .as_str()
        .context("Writebook's answer had no address for the file")?;
    let image = image::ImageFormat::from_path(path).is_ok();

    Ok(file_markdown(&name, url, image))
}

fn file_markdown(name: &str, url: &str, image: bool) -> String {
    let label = name.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]");

    if image {
        format!("![{label}]({url})")
    } else {
        format!("[{label}]({url})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_brackets_and_backslashes_in_an_uploaded_file_label() {
        assert_eq!(file_markdown("notes].pdf", "/u/1", false), "[notes\\].pdf](/u/1)");
        assert_eq!(file_markdown("a\\[b].png", "/u/2", true), "![a\\\\\\[b\\].png](/u/2)");
    }
}
