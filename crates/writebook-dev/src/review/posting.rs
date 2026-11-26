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
