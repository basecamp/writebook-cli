//! index.md is the whole book, minus the prose: front matter carries the
//! book's metadata, and the lines below are the table of contents — one line
//! per leaf, and line order is leaf order. Sections are headings (linked to a
//! file when they have body text of their own), pages are links, and pictures
//! are images whose alt text is the caption. A section can end with a
//! `{theme=...}` attribute, and the front matter can name a cover image.
//!
//! The front matter sits between two lines of three dashes and has the keys
//! `title`, `subtitle`, `author`, `theme` and, optionally, `cover`, whose
//! value is an image path like `assets/cover.jpg`. The entries below it look
//! like this:
//!
//! - `# Part One {theme=blue}`, a section without a file
//! - `# [Part Two](part-two.md)`, a section with a file
//! - `[Getting Started](getting-started.md)`, a page
//! - `![A sunset over the bay](assets/sunset.jpg)`, a picture

use anyhow::{Context, Result, bail};

pub struct Document {
    pub meta: BookMeta,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct BookMeta {
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub theme: String,
    /// The cover image in assets/, empty for no cover. None when index.md
    /// has no `cover:` line (checkouts made before it existed), which leaves
    /// the cover to the file tracked in state.
    pub cover: Option<String>,
}

impl BookMeta {
    pub fn without_cover(&self) -> BookMeta {
        BookMeta {
            cover: None,
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Section {
        title: String,
        filename: Option<String>,
        theme: Option<String>,
    },
    Page {
        title: String,
        filename: String,
    },
    Picture {
        caption: String,
        filename: String,
    },
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

    Ok(Document {
        meta: parse_meta(meta_text),
        entries,
    })
}

fn split_front_matter(text: &str) -> Result<(&str, &str)> {
    let after_opening = text
        .strip_prefix("---\n")
        .context("index.md must start with a --- front matter block carrying the book's title, author, and theme")?;

    let closing = after_opening
        .find("\n---")
        .context("index.md front matter is missing its closing ---")?;

    let contents = after_opening[closing + 4..]
        .strip_prefix('\n')
        .unwrap_or(&after_opening[closing + 4..]);
    Ok((&after_opening[..closing], contents))
}

fn parse_line(line: &str) -> Option<Entry> {
    if let Some(heading) = line.strip_prefix('#') {
        let (heading, theme) = split_theme(heading.trim_start_matches('#').trim());

        if let Some(rest) = heading.strip_prefix('[') {
            let (title, filename) = split_link(rest)?;
            Some(Entry::Section {
                title,
                filename: Some(filename),
                theme,
            })
        } else {
            Some(Entry::Section {
                title: heading.to_string(),
                filename: None,
                theme,
            })
        }
    } else if let Some(rest) = line.strip_prefix("![") {
        let (caption, filename) = split_link(rest)?;
        Some(Entry::Picture { caption, filename })
    } else if let Some(rest) = line.strip_prefix('[') {
        let (title, filename) = split_link(rest)?;
        Some(Entry::Page { title, filename })
    } else {
        None
    }
}

fn split_theme(heading: &str) -> (&str, Option<String>) {
    match heading
        .strip_suffix('}')
        .and_then(|attribute| attribute.rsplit_once("{theme="))
    {
        Some((rest, theme)) => (rest.trim_end(), Some(theme.trim().to_string())),
        None => (heading, None),
    }
}

fn split_link(rest: &str) -> Option<(String, String)> {
    let closing = rest.rfind("](")?;
    let target = &rest[closing + 2..];
    let target = target.strip_suffix(')')?;

    if target.is_empty() {
        None
    } else {
        Some((rest[..closing].to_string(), target.to_string()))
    }
}

fn parse_meta(text: &str) -> BookMeta {
    let mut meta = BookMeta::default();

    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().to_string();

            match key.trim() {
                "title" => meta.title = value,
                "subtitle" => meta.subtitle = value,
                "author" => meta.author = value,
                "theme" => meta.theme = value,
                "cover" => meta.cover = Some(value),
                _ => {}
            }
        }
    }

    meta
}

pub fn serialize(meta: &BookMeta, entries: &[Entry]) -> String {
    let mut text = serialize_meta(meta);
    text.push('\n');
    text.push_str(&serialize_entries(entries));
    text
}

fn serialize_meta(meta: &BookMeta) -> String {
    let mut text = format!(
        "---\ntitle: {}\nsubtitle: {}\nauthor: {}\ntheme: {}\n",
        meta.title, meta.subtitle, meta.author, meta.theme
    );

    if let Some(cover) = &meta.cover {
        text.push_str(&format!("cover: {cover}\n"));
    }

    text.push_str("---\n");
    text
}

fn serialize_entries(entries: &[Entry]) -> String {
    let mut text = String::new();

    for entry in entries {
        match entry {
            Entry::Section { title, filename, theme } => {
                if !text.is_empty() {
                    text.push('\n');
                }

                match filename {
                    Some(filename) => text.push_str(&format!("# [{title}]({filename})")),
                    None => text.push_str(&format!("# {title}")),
                }

                if let Some(theme) = theme {
                    text.push_str(&format!(" {{theme={theme}}}"));
                }

                text.push('\n');
            }
            Entry::Page { title, filename } => text.push_str(&format!("[{title}]({filename})\n")),
            Entry::Picture { caption, filename } => text.push_str(&format!("![{caption}]({filename})\n")),
        }
    }

    text
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
                Entry::Section {
                    title: "Part One".into(),
                    filename: None,
                    theme: None
                },
                Entry::Section {
                    title: "Part Two".into(),
                    filename: Some("part-two.md".into()),
                    theme: None
                },
                Entry::Page {
                    title: "Getting Started".into(),
                    filename: "getting-started.md".into()
                },
                Entry::Picture {
                    caption: "A sunset".into(),
                    filename: "assets/sunset.jpg".into()
                },
            ]
        );
    }

    #[test]
    fn ignores_blank_lines_and_keeps_brackets_in_titles() {
        let document = parse(&with_meta("\n[Arrays [and] slices](arrays.md)\n\n")).unwrap();

        assert_eq!(
            document.entries,
            vec![Entry::Page {
                title: "Arrays [and] slices".into(),
                filename: "arrays.md".into()
            }]
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
    fn reads_a_sections_theme() {
        let document = parse(&with_meta(
            "# Part One {theme=green}\n# [Part Two](part-two.md) {theme=blue}\n# Sets {and} maps\n",
        ))
        .unwrap();

        assert_eq!(
            document.entries,
            vec![
                Entry::Section {
                    title: "Part One".into(),
                    filename: None,
                    theme: Some("green".into())
                },
                Entry::Section {
                    title: "Part Two".into(),
                    filename: Some("part-two.md".into()),
                    theme: Some("blue".into())
                },
                Entry::Section {
                    title: "Sets {and} maps".into(),
                    filename: None,
                    theme: None
                },
            ]
        );
    }

    #[test]
    fn round_trips() {
        let text = with_meta(
            "# Part One {theme=green}\n[Getting Started](getting-started.md)\n![A sunset](assets/sunset.jpg)\n\n# [Part Two](part-two.md)\n",
        );
        let document = parse(&text).unwrap();
        assert_eq!(serialize(&document.meta, &document.entries), text);
    }
}
