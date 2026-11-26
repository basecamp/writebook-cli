use anyhow::{Result, bail};
use serde::Serialize;

use super::{cli_api_json, connect, not_readable, unexpected};
use crate::checkout;
use crate::leaf_kind::LeafKind;
use crate::output::{self, Breadcrumb, Format};
use crate::sync::{self, Manifest};
use crate::web::{self, Target, Web};

#[derive(Serialize)]
pub struct Document {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    url: String,
    markdown: String,
    source: Source,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Source {
    Api,
    Export,
}

pub fn read(url: &str, format: Format) -> Result<()> {
    let target = web::parse_target(url)?;

    let book_id = match target.book_id {
        Some(book_id) => book_id,
        None => {
            bail!("'{url}' is a site, not a book or page — run `writebook book list --instance {url}` to see its books")
        }
    };

    let web = connect(&target.host)?;

    let document = match from_api(&web, book_id, target.leaf_id)? {
        Some(document) => document,
        None => from_export(&web, &target)?,
    };

    match format {
        Format::Json { jq } => {
            let breadcrumbs = [
                Breadcrumb {
                    action: "contents",
                    command: format!("writebook book contents {}", document.url),
                    description: "List the book's pages, sections and pictures",
                },
                Breadcrumb {
                    action: "search",
                    command: format!("writebook search \"<query>\" --book {}", document.url),
                    description: "Search the book",
                },
            ];
            output::print_json(&document, &breadcrumbs, jq.as_deref())?;
        }
        _ => output::print_text(&as_markdown(&document)),
    }

    Ok(())
}

/// A whole book as Markdown, through the CLI API.
pub fn whole_book(web: &Web, book_id: u64) -> Result<Document> {
    match from_api(web, book_id, None)? {
        Some(document) => Ok(document),
        None => bail!("{} has no book {book_id}, or it doesn't have the CLI API", web.host),
    }
}

fn from_api(web: &Web, book_id: u64, leaf_id: Option<u64>) -> Result<Option<Document>> {
    if !web.signed_in() {
        return Ok(None);
    }

    let manifest = match cli_api_json(web, &format!("/books/{book_id}/manifest.json"))? {
        Some(json) => Manifest::parse(&json).ok(),
        None => None,
    };

    match manifest {
        Some(manifest) => Ok(Some(document_from_manifest(web, book_id, leaf_id, &manifest)?)),
        None => Ok(None),
    }
}

fn document_from_manifest(web: &Web, book_id: u64, leaf_id: Option<u64>, manifest: &Manifest) -> Result<Document> {
    let book_url = web.url(&format!("/{book_id}/{}", manifest.book.slug));

    match leaf_id {
        Some(leaf_id) => {
            let leaf = match manifest.leaves.iter().find(|leaf| leaf.id == leaf_id) {
                Some(leaf) => leaf,
                None => bail!("{} has no page {leaf_id} — it may have been deleted", book_url),
            };

            let markdown = leaf_markdown(web, book_id, &leaf.kind, leaf.id)?;
            let slug = checkout::slugify(&leaf.title);
            let slug = if slug.is_empty() { "page".to_string() } else { slug };

            Ok(Document {
                title: leaf.title.clone(),
                author: None,
                url: format!("{book_url}/{leaf_id}/{slug}"),
                markdown,
                source: Source::Api,
            })
        }
        None => {
            // The same shape as Writebook's own book export: every leaf's
            // Markdown, in book order, a blank line apart.
            let mut parts = Vec::new();

            for leaf in &manifest.leaves {
                parts.push(leaf_markdown(web, book_id, &leaf.kind, leaf.id)?);
            }

            Ok(Document {
                title: manifest.book.title.clone(),
                author: manifest.book.author.clone(),
                url: book_url,
                markdown: parts.join("\n\n"),
                source: Source::Api,
            })
        }
    }
}

fn leaf_markdown(web: &Web, book_id: u64, kind: &LeafKind, leaf_id: u64) -> Result<String> {
    let path = sync::leaf_path(book_id, kind, leaf_id);
    let value = match cli_api_json(web, &path)? {
        Some(value) => value,
        None => bail!("{} could not be read", web.url(&path)),
    };
    let content = sync::parse_leaf_content(&value);

    Ok(match kind {
        LeafKind::Picture => content.caption.unwrap_or_default(),
        LeafKind::Page | LeafKind::Section | LeafKind::Unknown(_) => content.body.unwrap_or_default(),
    })
}

fn from_export(web: &Web, target: &Target) -> Result<Document> {
    let page = web.get(&format!("{}.md", target.path), "text/markdown")?;
    let what = if target.leaf_id.is_some() { "page" } else { "book" };

    if page.status == 404 || page.wants_sign_in() {
        bail!("{}", not_readable(web, what, &web.url(&target.path)));
    }

    if !page.ok() {
        bail!("{}", unexpected(web, &page, &format!("{}.md", target.path)));
    }

    let (fields, markdown) = split_front_matter(&page.body);
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };

    Ok(Document {
        title: field("title").unwrap_or_default(),
        author: field("author"),
        url: field("url").unwrap_or_else(|| web.url(&target.path)),
        markdown,
        source: Source::Export,
    })
}

/// Split Writebook's Markdown export into its front matter (title, url,
/// author) and body.
pub fn split_front_matter(text: &str) -> (Vec<(String, String)>, String) {
    let split = text.strip_prefix("---\n").and_then(|rest| {
        rest.split_once("\n---\n")
            .or_else(|| rest.split_once("\n---").filter(|(_, it)| it.is_empty()))
    });

    match split {
        Some((header, body)) => {
            let fields = header
                .lines()
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| {
                    let value = value.trim();
                    let value = value
                        .strip_prefix('"')
                        .and_then(|it| it.strip_suffix('"'))
                        .unwrap_or(value);
                    (key.trim().to_string(), web::unescape(value))
                })
                .collect();

            (fields, body.trim_start_matches('\n').to_string())
        }
        None => (Vec::new(), text.to_string()),
    }
}

fn as_markdown(document: &Document) -> String {
    let mut text = String::from("---\n");
    text.push_str(&format!("title: {}\n", quoted(&document.title)));

    if let Some(author) = &document.author {
        text.push_str(&format!("author: {}\n", quoted(author)));
    }

    text.push_str(&format!("url: {}\n", quoted(&document.url)));
    text.push_str("---\n\n");
    text.push_str(&document.markdown);

    if !text.ends_with('\n') {
        text.push('\n');
    }

    text
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("a string always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_front_matter() {
        let (fields, body) =
            split_front_matter("---\ntitle: \"On &amp; off call\"\nurl: \"http://x/4/b/12/p\"\n---\n\n# Hello\n");

        assert_eq!(
            fields,
            vec![
                ("title".to_string(), "On & off call".to_string()),
                ("url".to_string(), "http://x/4/b/12/p".to_string())
            ]
        );
        assert_eq!(body, "# Hello\n");
    }

    #[test]
    fn leaves_text_without_front_matter_alone() {
        assert_eq!(split_front_matter("# Hello"), (Vec::new(), "# Hello".to_string()));
    }
}
