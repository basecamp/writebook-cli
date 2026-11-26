//! The read-only commands against a real Writebook, seeded with
//! tests/support/seed.rb. Skipped unless WRITEBOOK_LIVE_URL points at one:
//!
//!   docker run -d -p 3000:80 -e SECRET_KEY_BASE=$(openssl rand -hex 64) -e DISABLE_SSL=1 ghcr.io/basecamp/writebook:main
//!   docker exec -i <container> bin/rails runner - < tests/support/seed.rb
//!   WRITEBOOK_LIVE_URL=http://localhost:3000 bin/dev test run
//!
//! CI does exactly this.

#![cfg(test)]

#[cfg(test)]
mod support;

use support::{stderr, stdout, writebook};

fn live_url() -> Option<String> {
    let url = std::env::var("WRITEBOOK_LIVE_URL").ok().filter(|it| !it.is_empty());

    if url.is_none() {
        // In CI a missing URL means the job is misconfigured, not that the
        // tests should quietly pass.
        assert!(std::env::var_os("CI").is_none(), "WRITEBOOK_LIVE_URL is not set in CI");
        eprintln!("skipped: set WRITEBOOK_LIVE_URL to run against a real Writebook");
    }

    url
}

#[test]
fn reads_a_published_page_by_its_link_in_one_command() {
    let Some(url) = live_url() else { return };

    let output = writebook(&["read", &format!("{url}/1/ops-on-call-handbook/2/deploys")], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("title: \"Deploys\""), "{}", stdout(&output));
    assert!(
        stdout(&output).contains("To roll back a deploy, run `bin/rollback`."),
        "{}",
        stdout(&output)
    );
}

#[test]
fn reads_a_whole_published_book() {
    let Some(url) = live_url() else { return };

    let output = writebook(&["read", "--json", &format!("{url}/1/ops-on-call-handbook")], &[]);
    assert!(output.status.success(), "{}", stderr(&output));

    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();
    assert_eq!(json["title"], "Ops & On-call Handbook");
    assert!(json["markdown"].as_str().unwrap().contains("check the workers first"));
}

#[test]
fn lists_published_books_only() {
    let Some(url) = live_url() else { return };

    let output = writebook(&["book", "list", "--json", "--instance", &url], &[]);
    assert!(output.status.success(), "{}", stderr(&output));

    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();
    let titles: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|it| it["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Ops & On-call Handbook"]);
}

#[test]
fn searches_every_published_book() {
    let Some(url) = live_url() else { return };

    let output = writebook(&["search", "resque queue", "--json", "--instance", &url], &[]);
    assert!(output.status.success(), "{}", stderr(&output));

    let json = serde_json::from_str::<serde_json::Value>(&stdout(&output)).unwrap()["data"].clone();
    assert_eq!(json[0]["title"], "Queue backlog & workers");
    assert_eq!(
        json[0]["url"],
        format!("{url}/1/ops-on-call-handbook/3/queue-backlog-workers")
    );
}

#[test]
fn private_pages_need_a_token() {
    let Some(url) = live_url() else { return };

    let output = writebook(&["read", &format!("{url}/2/private-runbooks/4/database-failover")], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("WRITEBOOK_TOKEN"), "{}", stderr(&output));
}
