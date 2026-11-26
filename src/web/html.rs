#[derive(Debug, PartialEq)]
pub struct BookLink {
    pub id: u64,
    pub title: String,
    pub author: String,
}

/// The books on a Writebook library page (`/`). Each one renders as a
/// `library__book` figure with a lazy bookmark frame carrying its id.
pub fn parse_library(html: &str) -> Vec<BookLink> {
    html.split("<figure class=\"library__book")
        .skip(1)
        .filter_map(|chunk| {
            let id = between(chunk, "id=\"bookmark_book_", "\"")?.parse().ok()?;
            let title = text_of(between(chunk, "<strong>", "</strong>").unwrap_or_default());
            let after_title = chunk.split_once("</strong>").map(|(_, it)| it).unwrap_or_default();
            let author =
                text_of(between(after_title, "<span class=\"overflow-line-clamp\">", "</span>").unwrap_or_default());

            Some(BookLink { id, title, author })
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct SearchHit {
    pub title: String,
    pub path: String,
    pub snippet: String,
}

/// The hits on a book's search results page. Each hit renders twice, once
/// for reading mode and once for edit mode; keep the reading-mode link.
pub fn parse_search_results(html: &str) -> Vec<SearchHit> {
    html.split("<a ")
        .skip(1)
        .filter_map(|chunk| {
            let (tag, rest) = chunk.split_once('>')?;

            if tag.contains("search__result") && tag.contains("hide_from_edit_mode") {
                let href = unescape(between(tag, "href=\"", "\"")?);
                let path = href.split(['?', '#']).next().unwrap_or_default().to_string();
                let content = match rest.split_once("</a>") {
                    Some((inside, _)) => inside,
                    None => rest,
                };

                let title = text_of(between(content, "<strong>", "</strong>").unwrap_or_default());
                let title = title.trim_end_matches(':').trim().to_string();
                let after_title = match content.split_once("</strong>") {
                    Some((_, after)) => after,
                    None => content,
                };
                let snippet = text_of(after_title);

                Some(SearchHit { title, path, snippet })
            } else {
                None
            }
        })
        .collect()
}

/// The CSRF token Rails puts in every page's head.
pub fn csrf_token(html: &str) -> Option<String> {
    let tag = html.split("<meta ").find(|it| it.starts_with("name=\"csrf-token\""))?;
    between(tag, "content=\"", "\"").map(unescape)
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(start)?;
    rest.split_once(end).map(|(it, _)| it)
}

fn text_of(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;

    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }

    unescape(&text).split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn unescape(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];

        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|decimal| decimal.parse().ok()))
                    .and_then(char::from_u32),
            };
            character.map(|it| (it, end))
        });

        match decoded {
            Some((character, end)) => {
                result.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                result.push('&');
                rest = &rest[1..];
            }
        }
    }

    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_library() {
        let html = r#"
          <div class="library">
            <figure class="library__book theme--blue">
              <turbo-frame id="bookmark_book_4" src="/books/4/bookmark" target="_top"></turbo-frame>
              <h2><strong>Ops &amp; On-call</strong>
                <span class="overflow-line-clamp">37signals</span></h2>
            </figure>
            <figure class="library__book position-relative"><div class="library__book--empty center"></div></figure>
          </div>"#;

        assert_eq!(
            parse_library(html),
            vec![BookLink {
                id: 4,
                title: "Ops & On-call".to_string(),
                author: "37signals".to_string()
            }]
        );
    }

    #[test]
    fn parses_search_results() {
        let html = r#"
          <a class="search__result hide_from_reading_mode txt-ink" data-turbo-frame="_top" href="/books/4/pages/12/edit?search=deploy">
            <strong><mark>Deploy</mark>s:</strong> how to <mark>deploy</mark> &amp; roll back</a>
          <a class="search__result hide_from_edit_mode txt-ink" data-turbo-frame="_top" href="/4/handbook/12/deploys?search=deploy">
            <strong><mark>Deploy</mark>s:</strong> how to <mark>deploy</mark> &amp; roll back</a>"#;

        assert_eq!(
            parse_search_results(html),
            vec![SearchHit {
                title: "Deploys".to_string(),
                path: "/4/handbook/12/deploys".to_string(),
                snippet: "how to deploy & roll back".to_string(),
            }]
        );
    }

    #[test]
    fn finds_the_csrf_token() {
        let html =
            r#"<meta name="csrf-param" content="authenticity_token" /><meta name="csrf-token" content="abc+/=" />"#;
        assert_eq!(csrf_token(html), Some("abc+/=".to_string()));
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(
            unescape("a &amp; b &lt;c&gt; &#39;d&#x27; &bogus; & e"),
            "a & b <c> 'd' &bogus; & e"
        );
    }
}
