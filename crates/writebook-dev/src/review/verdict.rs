//! What a reviewer answers, held to the rules' ids, and what the reviews add up to.

use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, PartialEq)]
pub struct Answer {
    pub result: String,
    pub summary: String,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct Finding {
    pub rule: String,
    pub location: String,
    pub finding: String,
    pub remedy: String,
}

pub struct Review {
    pub by: String,
    pub answer: Result<Answer, String>,
}

impl Answer {
    pub fn passed(&self) -> bool {
        self.result == "pass" && self.findings.is_empty()
    }
}

impl Review {
    pub fn passed(&self) -> bool {
        self.answer.as_ref().is_ok_and(Answer::passed)
    }
}

/// Every id in a `#### ID: Title` heading of the rules.
pub fn rule_ids(rules: &str) -> Vec<String> {
    rules
        .lines()
        .filter_map(|line| line.strip_prefix("#### "))
        .filter_map(|heading| heading.split_once(':'))
        .map(|(id, _)| id.trim().to_string())
        .collect()
}

pub fn schema(ids: &[String]) -> Value {
    let text = json!({"type": "string"});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["result", "summary", "findings"],
        "properties": {
            "result": {"type": "string", "enum": ["pass", "fail"]},
            "summary": text,
            "findings": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["rule", "location", "finding", "remedy"],
                "properties": {
                    "rule": {"type": "string", "enum": ids},
                    "location": text,
                    "finding": text,
                    "remedy": text,
                },
            }},
        },
    })
}

/// A reviewer's answer, refused when it isn't the schema's shape or cites a rule that isn't one.
pub fn parsed(answer: Value, ids: &[String]) -> Result<Answer, String> {
    let answer: Answer = serde_json::from_value(answer).map_err(|error| error.to_string())?;
    if let Some(unknown) = answer.findings.iter().find(|found| !ids.contains(&found.rule)) {
        Err(format!(
            "it cited {}, which is no rule of {}",
            unknown.rule,
            super::RULES
        ))
    } else {
        Ok(answer)
    }
}

pub fn passed(reviews: &[Review]) -> bool {
    reviews.iter().all(Review::passed)
}

pub fn report(reviews: &[Review]) -> String {
    let mut report = String::new();
    for review in reviews {
        match &review.answer {
            Ok(answer) => {
                let result = if answer.passed() { "pass" } else { "fail" };
                report.push_str(&format!("{}: {result}\n  {}\n", review.by, answer.summary));
                for found in &answer.findings {
                    report.push_str(&format!(
                        "\n  {} at {}\n    {}\n    Change: {}\n",
                        found.rule, found.location, found.finding, found.remedy
                    ));
                }
            }
            Err(error) => report.push_str(&format!("{}: gave no answer: {error}\n", review.by)),
        }
        report.push('\n');
    }
    let result = if passed(reviews) { "pass" } else { "fail" };
    report.push_str(&format!("Review: {result}\n"));
    report
}
