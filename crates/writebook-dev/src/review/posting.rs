//! The verdict as the `review` commit status on the pull request's head.

use std::error::Error;

use serde::Deserialize;

use super::verdict::{self, Review};
use super::{REPOSITORY, gh};

pub const CONTEXT: &str = "review";
const DESCRIPTION_LIMIT: usize = 140;

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    #[serde(rename = "headRefOid")]
    pub head: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, PartialEq)]
pub enum Posting {
    Post,
    Skip(&'static str),
}

/// The checked-out branch's pull request, or none when it has none.
pub fn pull_request() -> Result<Option<PullRequest>, Box<dyn Error>> {
    match gh(&["pr", "view", "--json", "headRefOid,title,body"]) {
        Ok(found) => Ok(Some(serde_json::from_str(&found)?)),
        Err(error) if error.to_string().contains("no pull requests found") => Ok(None),
        Err(error) => Err(error),
    }
}

/// A verdict is posted only on the head it read, so only when that head is the pull request's.
pub fn posting(post: bool, pull_request: Option<&PullRequest>, head: &str) -> Posting {
    if !post {
        Posting::Skip("--no-post was given")
    } else {
        match pull_request {
            None => Posting::Skip("the branch has no pull request"),
            Some(pull_request) if pull_request.head != head => {
                Posting::Skip("the pull request's head isn't this commit, so push it first")
            }
            Some(_) => Posting::Post,
        }
    }
}

pub fn post(head: &str, reviews: &[Review]) -> Result<(), Box<dyn Error>> {
    gh(&[
        "api",
        "-X",
        "POST",
        &format!("repos/{REPOSITORY}/statuses/{head}"),
        "-f",
        &format!("state={}", state(reviews)),
        "-f",
        &format!("context={CONTEXT}"),
        "-f",
        &format!("description={}", description(reviews)),
    ])?;
    Ok(())
}

fn state(reviews: &[Review]) -> &'static str {
    if verdict::passed(reviews) { "success" } else { "failure" }
}

fn description(reviews: &[Review]) -> String {
    if verdict::passed(reviews) {
        String::from("pass")
    } else {
        let mut failed: Vec<String> = Vec::new();
        for review in reviews {
            match &review.answer {
                Ok(answer) => {
                    for found in &answer.findings {
                        if !failed.contains(&found.rule) {
                            failed.push(found.rule.clone());
                        }
                    }
                }
                Err(_) => failed.push(format!("{} gave no answer", review.by)),
            }
        }
        if failed.is_empty() {
            failed.push(String::from("a reviewer failed it with no finding"));
        }
        format!("fail: {}", failed.join(", "))
            .chars()
            .take(DESCRIPTION_LIMIT)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::verdict::{Answer, Finding};
    use super::*;

    fn pull_request(head: &str) -> PullRequest {
        PullRequest {
            head: head.to_string(),
            title: String::from("Sign in with a device code"),
            body: String::new(),
        }
    }

    fn reviewed(result: &str, rules: &[&str]) -> Review {
        Review {
            by: String::from("codex gpt-6.1-sol"),
            answer: Ok(Answer {
                result: result.to_string(),
                summary: String::from("Fine."),
                findings: rules
                    .iter()
                    .map(|rule| Finding {
                        rule: rule.to_string(),
                        location: String::from("src/creds.rs:12"),
                        finding: String::from("The token is logged."),
                        remedy: String::from("Log the host instead."),
                    })
                    .collect(),
            }),
        }
    }

    #[test]
    fn the_verdict_is_posted_only_on_a_pushed_pull_requests_head() {
        assert_eq!(Posting::Post, posting(true, Some(&pull_request("abc")), "abc"));
        assert_eq!(
            Posting::Skip("the pull request's head isn't this commit, so push it first"),
            posting(true, Some(&pull_request("old")), "abc")
        );
        assert_eq!(
            Posting::Skip("the branch has no pull request"),
            posting(true, None, "abc")
        );
        assert_eq!(
            Posting::Skip("--no-post was given"),
            posting(false, Some(&pull_request("abc")), "abc")
        );
    }

    #[test]
    fn a_pull_request_is_read_from_gh() {
        let found: PullRequest =
            serde_json::from_str(r#"{"headRefOid": "abc", "title": "Sign in", "body": "Adds it."}"#).unwrap();

        assert_eq!(
            ("abc", "Sign in", "Adds it."),
            (found.head.as_str(), found.title.as_str(), found.body.as_str())
        );
    }

    #[test]
    fn a_passing_review_posts_success() {
        let reviews = [reviewed("pass", &[]), reviewed("pass", &[])];

        assert_eq!("success", state(&reviews));
        assert_eq!("pass", description(&reviews));
    }

    #[test]
    fn a_failing_review_posts_failure_naming_the_rules_and_the_silent_reviewers() {
        let silent = Review {
            by: String::from("claude claude-opus-5-5"),
            answer: Err(String::from("claude is out of quota")),
        };
        let reviews = [reviewed("fail", &["SEC-1", "TEST-2", "SEC-1"]), silent];

        assert_eq!("failure", state(&reviews));
        assert_eq!(
            "fail: SEC-1, TEST-2, claude claude-opus-5-5 gave no answer",
            description(&reviews)
        );
    }

    #[test]
    fn a_long_description_is_cut_to_what_github_takes() {
        let rules: Vec<String> = (1..=40).map(|rule| format!("TEST-{rule}")).collect();
        let rules: Vec<&str> = rules.iter().map(String::as_str).collect();

        assert_eq!(
            DESCRIPTION_LIMIT,
            description(&[reviewed("fail", &rules)]).chars().count()
        );
    }
}
