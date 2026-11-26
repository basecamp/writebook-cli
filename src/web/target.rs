use anyhow::{Context, Result, bail};

/// What a URL points at: a whole book, or one leaf (page, section, picture)
/// in it. Accepts reading links (/4/handbook/12/on-call), editing links
/// (/books/4/pages/12/edit), `.md` links, and ignores ?query and #anchor.
#[derive(Debug, PartialEq)]
pub struct Target {
    pub host: String,
    pub book_id: Option<u64>,
    pub leaf_id: Option<u64>,
    pub kind: Option<String>,
    /// Reading path without `.md`, e.g. /4/handbook/12/on-call
    pub path: String,
}

pub fn parse_target(url: &str) -> Result<Target> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else {
        bail!("'{url}' is not an http(s) URL");
    };

    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(split);

    if authority.contains('@') {
        bail!("'{url}' has a user name or password in it; give the site's plain address");
    }

    let host = format!("{scheme}://{}", authority.to_lowercase());

    let path = path.split(['#', '?']).next().unwrap_or_default();
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".md").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|it| !it.is_empty()).collect();

    let number = |segment: Option<&&str>| segment.and_then(|it| it.parse::<u64>().ok());

    match segments.as_slice() {
        [] => Ok(Target {
            host,
            book_id: None,
            leaf_id: None,
            kind: None,
            path: String::new(),
        }),
        ["books", book, rest @ ..] => {
            let book_id = number(Some(book)).with_context(|| format!("'{url}' has no book id"))?;

            match rest {
                [kind @ ("pages" | "sections" | "pictures"), leaf, ..] => {
                    let leaf_id = number(Some(leaf)).with_context(|| format!("'{url}' has no page id"))?;
                    let kind = kind.trim_end_matches('s');
                    let kind = format!("{}{}", kind[..1].to_uppercase(), &kind[1..]);

                    Ok(Target {
                        host,
                        book_id: Some(book_id),
                        leaf_id: Some(leaf_id),
                        kind: Some(kind),
                        path: format!("/{book_id}/book/{leaf_id}/page"),
                    })
                }
                _ => Ok(Target {
                    host,
                    book_id: Some(book_id),
                    leaf_id: None,
                    kind: None,
                    path: format!("/{book_id}/book"),
                }),
            }
        }
        [book, rest @ ..] => {
            let book_id = number(Some(book)).with_context(|| {
                format!("'{url}' has no book id — expected something like https://site/4/some-book")
            })?;
            let book_slug = rest.first().copied().unwrap_or("book");

            match rest {
                [_, leaf, leaf_rest @ ..] => {
                    let leaf_id = number(Some(leaf)).with_context(|| format!("'{url}' has no page id"))?;
                    let leaf_slug = leaf_rest.first().copied().unwrap_or("page");

                    Ok(Target {
                        host,
                        book_id: Some(book_id),
                        leaf_id: Some(leaf_id),
                        kind: None,
                        path: format!("/{book_id}/{book_slug}/{leaf_id}/{leaf_slug}"),
                    })
                }
                _ => Ok(Target {
                    host,
                    book_id: Some(book_id),
                    leaf_id: None,
                    kind: None,
                    path: format!("/{book_id}/{book_slug}"),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(host: &str, book: Option<u64>, leaf: Option<u64>, kind: Option<&str>, path: &str) -> Target {
        Target {
            host: host.to_string(),
            book_id: book,
            leaf_id: leaf,
            kind: kind.map(str::to_string),
            path: path.to_string(),
        }
    }

    #[test]
    fn parses_reading_links() {
        assert_eq!(
            parse_target("https://books.example.com/4/handbook/12/on-call").unwrap(),
            target(
                "https://books.example.com",
                Some(4),
                Some(12),
                None,
                "/4/handbook/12/on-call"
            )
        );
        assert_eq!(
            parse_target("https://books.example.com/4/handbook/12/on-call.md?search=x#part").unwrap(),
            target(
                "https://books.example.com",
                Some(4),
                Some(12),
                None,
                "/4/handbook/12/on-call"
            )
        );
        assert_eq!(
            parse_target("http://localhost:3000/4/handbook/").unwrap(),
            target("http://localhost:3000", Some(4), None, None, "/4/handbook")
        );
        assert_eq!(
            parse_target("https://books.example.com/4").unwrap(),
            target("https://books.example.com", Some(4), None, None, "/4/book")
        );
    }

    #[test]
    fn parses_editing_links() {
        assert_eq!(
            parse_target("https://books.example.com/books/4/pages/12/edit").unwrap(),
            target(
                "https://books.example.com",
                Some(4),
                Some(12),
                Some("Page"),
                "/4/book/12/page"
            )
        );
        assert_eq!(
            parse_target("https://books.example.com/books/4/sections/7").unwrap(),
            target(
                "https://books.example.com",
                Some(4),
                Some(7),
                Some("Section"),
                "/4/book/7/page"
            )
        );
        assert_eq!(
            parse_target("https://books.example.com/books/4/edit").unwrap(),
            target("https://books.example.com", Some(4), None, None, "/4/book")
        );
    }

    #[test]
    fn parses_site_links() {
        assert_eq!(
            parse_target("https://books.example.com").unwrap(),
            target("https://books.example.com", None, None, None, "")
        );
        assert_eq!(
            parse_target("https://books.example.com/").unwrap(),
            target("https://books.example.com", None, None, None, "")
        );
    }

    #[test]
    fn keeps_the_site_apart_from_query_and_anchor() {
        assert_eq!(
            parse_target("https://Books.Example.com?x=1").unwrap(),
            target("https://books.example.com", None, None, None, "")
        );
        assert_eq!(
            parse_target("https://books.example.com#a").unwrap(),
            target("https://books.example.com", None, None, None, "")
        );
    }

    #[test]
    fn rejects_bad_links() {
        assert!(parse_target("https://books.example.com@evil.example/4/x").is_err());
        assert!(parse_target("books.example.com/4/x").is_err());
        assert!(parse_target("https://books.example.com/users/4").is_err());
        assert!(parse_target("https://books.example.com/4/x/nope").is_err());
    }
}
