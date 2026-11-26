use anyhow::{Context, Result, bail};

use crate::checkout::{Checkout, Remote};

pub fn add(name: &str, url: &str) -> Result<()> {
    let checkout = Checkout::find()?;
    let (host, book_id) = parse_book_url(url)?;

    let mut config = checkout.config()?;

    if config.remotes.contains_key(name) {
        bail!("remote '{name}' already exists — remove it first with `writebook remote remove {name}`");
    }

    config.remotes.insert(
        name.to_string(),
        Remote {
            url: host.clone(),
            book_id,
        },
    );
    checkout.save_config(&config)?;

    println!("Added remote '{name}' for book {book_id} on {host}.");
    Ok(())
}

pub fn list() -> Result<()> {
    let checkout = Checkout::find()?;
    let config = checkout.config()?;

    if config.remotes.is_empty() {
        println!("No remotes. Add one with `writebook remote add origin <book-url>`.");
    } else {
        for (name, remote) in &config.remotes {
            println!("{name}\t{}/{}", remote.url, remote.book_id);
        }
    }

    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    let checkout = Checkout::find()?;
    let mut config = checkout.config()?;

    if config.remotes.remove(name).is_none() {
        bail!("no remote named '{name}' — see `writebook remote list`");
    }

    checkout.save_config(&config)?;
    println!("Removed remote '{name}'.");
    Ok(())
}

/// Split a book URL like https://books.example.com/4/some-book into the site
/// host and the book id.
pub fn parse_book_url(url: &str) -> Result<(String, u64)> {
    let target = crate::web::parse_target(url)?;
    let book_id = target
        .book_id
        .with_context(|| format!("'{url}' has no book id — expected something like https://site/4/slug"))?;

    Ok((target.host, book_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_book_urls() {
        assert_eq!(
            parse_book_url("https://books.example.com/4/some-book").unwrap(),
            ("https://books.example.com".to_string(), 4)
        );
        assert_eq!(
            parse_book_url("http://localhost:3000/12").unwrap(),
            ("http://localhost:3000".to_string(), 12)
        );
        assert!(parse_book_url("https://books.example.com").is_err());
        assert!(parse_book_url("https://books.example.com/not-a-book").is_err());
        assert!(parse_book_url("books.example.com/4").is_err());
        assert_eq!(
            parse_book_url("https://Books.Example.com/4").unwrap().0,
            "https://books.example.com"
        );
        assert!(parse_book_url("https://me@books.example.com/4").is_err());
    }
}
