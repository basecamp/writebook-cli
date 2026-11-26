use anyhow::Result;

use super::token;

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
