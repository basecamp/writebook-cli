//! Three-way merges via diffy, plus the rewriting between the server's
//! /u/<slug> upload URLs and local assets/<slug> paths. Slugs are unique
//! server-generated tokens, so plain substring replacement is safe.

pub fn merge(base: &str, ours: &str, theirs: &str) -> Result<String, String> {
    diffy::merge(base, ours, theirs)
}

pub fn localize_upload_urls(body: &str) -> String {
    body.replace("](/u/", "](assets/")
}

pub fn globalize_upload_urls(body: &str) -> String {
    body.replace("](assets/", "](/u/")
}

/// Local asset paths referenced by a working page body, in order of appearance.
pub fn referenced_assets(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = body;

    while let Some(position) = rest.find("](assets/") {
        rest = &rest[position + 9..];

        if let Some(end) = rest.find(')') {
            let name = &rest[..end];

            if !name.is_empty() && !names.iter().any(|it| it == name) {
                names.push(name.to_string());
            }

            rest = &rest[end..];
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_non_overlapping_edits_cleanly() {
        let base = "one\ntwo\nthree\n";
        let ours = "ONE\ntwo\nthree\n";
        let theirs = "one\ntwo\nTHREE\n";

        assert_eq!(merge(base, ours, theirs).unwrap(), "ONE\ntwo\nTHREE\n");
    }

    #[test]
    fn reports_conflicting_edits_with_markers() {
        let base = "one\n";
        let conflicted = merge(base, "ours\n", "theirs\n").unwrap_err();

        assert!(conflicted.contains("<<<<<<<"));
        assert!(conflicted.contains(">>>>>>>"));
    }

    #[test]
    fn rewrites_upload_urls_both_ways() {
        let remote = "See ![toolbar](/u/toolbar-lnzfdA.png) here";
        let local = "See ![toolbar](assets/toolbar-lnzfdA.png) here";

        assert_eq!(localize_upload_urls(remote), local);
        assert_eq!(globalize_upload_urls(local), remote);
    }

    #[test]
    fn finds_referenced_assets_once_each() {
        let body = "![a](assets/a.png) and ![b](assets/b.png) and ![a](assets/a.png)";
        assert_eq!(referenced_assets(body), vec!["a.png", "b.png"]);
    }
}
