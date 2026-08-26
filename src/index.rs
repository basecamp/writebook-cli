//! index.md is the whole book, minus the prose: front matter carries the
//! book's metadata, and the lines below are the table of contents — one line
//! per leaf, and line order is leaf order. Sections are headings (linked to a
//! file when they have body text of their own), pages are links, and pictures
//! are images whose alt text is the caption:
//!
//!   ---
//!   title: The Field Guide
//!   subtitle:
//!   author: Stanko
//!   theme: green
//!   ---
//!
//!   # Part One
//!   # [Part Two](part-two.md)
//!   [Getting Started](getting-started.md)
//!   ![A sunset over the bay](assets/sunset.jpg)

use anyhow::{Context, Result, bail};

use crate::checkout::BookMeta;

pub struct Document {
    pub meta: BookMeta,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Section { title: String, filename: Option<String> },
    Page { title: String, filename: String },
    Picture { caption: String, filename: String },
}

impl Entry {
    pub fn title(&self) -> &str {
        match self {
            Entry::Section { title, .. } => title,
            Entry::Page { title, .. } => title,
            Entry::Picture { caption, .. } => caption,
        }
    }

    pub fn filename(&self) -> Option<&str> {
        match self {
            Entry::Section { filename, .. } => filename.as_deref(),
            Entry::Page { filename, .. } => Some(filename),
            Entry::Picture { filename, .. } => Some(filename),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Entry::Section { .. } => "Section",
            Entry::Page { .. } => "Page",
            Entry::Picture { .. } => "Picture",
        }
    }
}

pub fn parse(text: &str) -> Result<Document> {
    let (meta_text, contents) = split_front_matter(text)?;

    let mut entries = Vec::new();

    for (number, line) in contents.lines().enumerate() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        match parse_line(line) {
            Some(entry) => entries.push(entry),
            None => bail!(
                "index.md line {}: expected a section heading, a [page](file.md) link, or an ![image](assets/...) — got: {}",
                number + 1,
                line
            ),
        }
    }

    Ok(Document { meta: crate::checkout::parse_book_md(meta_text), entries })
}

pub fn serialize(meta: &BookMeta, entries: &[Entry]) -> String {
    let mut text = crate::checkout::serialize_book_md(meta);
    text.push('\n');
    text.push_str(&serialize_entries(entries));
    text
}

fn split_front_matter(text: &str) -> Result<(&str, &str)> {
    let after_opening = text
        .strip_prefix("---\n")
        .context("index.md must start with a --- front matter block carrying the book's title, author, and theme")?;

    let closing = after_opening
        .find("\n---")
        .context("index.md front matter is missing its closing ---")?;

    let contents = after_opening[closing + 4..].strip_prefix('\n').unwrap_or(&after_opening[closing + 4..]);
    Ok((&after_opening[..closing], contents))
}

fn serialize_entries(entries: &[Entry]) -> String {
    let mut text = String::new();

    for entry in entries {
        match entry {
            Entry::Section { title, filename } => {
                if !text.is_empty() {
                    text.push('\n');
                }

                match filename {
                    Some(filename) => text.push_str(&format!("# [{title}]({filename})\n")),
                    None => text.push_str(&format!("# {title}\n")),
                }
            }
            Entry::Page { title, filename } => text.push_str(&format!("[{title}]({filename})\n")),
            Entry::Picture { caption, filename } => text.push_str(&format!("![{caption}]({filename})\n")),
        }
    }

    text
}

fn parse_line(line: &str) -> Option<Entry> {
    if let Some(heading) = line.strip_prefix('#') {
        let heading = heading.trim_start_matches('#').trim();

        if let Some(rest) = heading.strip_prefix('[') {
            let (title, filename) = split_link(rest)?;
            return Some(Entry::Section { title, filename: Some(filename) });
        }

        return Some(Entry::Section { title: heading.to_string(), filename: None });
    }

    if let Some(rest) = line.strip_prefix("![") {
        let (caption, filename) = split_link(rest)?;
        return Some(Entry::Picture { caption, filename });
    }

    if let Some(rest) = line.strip_prefix('[') {
        let (title, filename) = split_link(rest)?;
        return Some(Entry::Page { title, filename });
    }

    None
}

fn split_link(rest: &str) -> Option<(String, String)> {
    let closing = rest.rfind("](")?;
    let target = &rest[closing + 2..];
    let target = target.strip_suffix(')')?;

    if target.is_empty() {
        return None;
    }

    Some((rest[..closing].to_string(), target.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONT_MATTER: &str = "---\ntitle: Book\nsubtitle: \nauthor: Jane\ntheme: blue\n---\n\n";

    fn with_meta(contents: &str) -> String {
        format!("{FRONT_MATTER}{contents}")
    }

    #[test]
    fn parses_front_matter_and_the_three_entry_kinds() {
        let document = parse(&with_meta(
            "# Part One\n# [Part Two](part-two.md)\n[Getting Started](getting-started.md)\n![A sunset](assets/sunset.jpg)\n",
        ))
        .unwrap();

        assert_eq!(document.meta.title, "Book");
        assert_eq!(document.meta.author, "Jane");
        assert_eq!(
            document.entries,
            vec![
                Entry::Section { title: "Part One".into(), filename: None },
                Entry::Section { title: "Part Two".into(), filename: Some("part-two.md".into()) },
                Entry::Page { title: "Getting Started".into(), filename: "getting-started.md".into() },
                Entry::Picture { caption: "A sunset".into(), filename: "assets/sunset.jpg".into() },
            ]
        );
    }

    #[test]
    fn ignores_blank_lines_and_keeps_brackets_in_titles() {
        let document = parse(&with_meta("\n[Arrays [and] slices](arrays.md)\n\n")).unwrap();

        assert_eq!(
            document.entries,
            vec![Entry::Page { title: "Arrays [and] slices".into(), filename: "arrays.md".into() }]
        );
    }

    #[test]
    fn rejects_lines_it_cannot_understand() {
        assert!(parse(&with_meta("just some prose\n")).is_err());
        assert!(parse(&with_meta("[no target]()\n")).is_err());
    }

    #[test]
    fn requires_front_matter() {
        assert!(parse("[Getting Started](getting-started.md)\n").is_err());
        assert!(parse("---\ntitle: Book\nno closing fence\n").is_err());
    }

    #[test]
    fn round_trips() {
        let text = with_meta("# Part One\n[Getting Started](getting-started.md)\n![A sunset](assets/sunset.jpg)\n\n# [Part Two](part-two.md)\n");
        let document = parse(&text).unwrap();
        assert_eq!(serialize(&document.meta, &document.entries), text);
    }
}
