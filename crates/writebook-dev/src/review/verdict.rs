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

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> Vec<String> {
        vec![String::from("SEC-1"), String::from("TEST-1")]
    }

    fn finding() -> Value {
        json!({
            "rule": "SEC-1",
            "location": "src/creds.rs:12",
            "finding": "The token is logged.",
            "remedy": "Log the host instead.",
        })
    }

    #[test]
    fn rule_ids_are_read_from_the_rules_headings() {
        let rules = "## Rules\n\n### Security\n\n#### SEC-1: Tokens stay secret\n\nText.\n\n#### TEST-1: No test loosened to pass\n";

        assert_eq!(ids(), rule_ids(rules));
    }

    #[test]
    fn a_failing_answer_is_parsed_with_its_findings() {
        let answer = parsed(
            json!({"result": "fail", "summary": "It leaks.", "findings": [finding()]}),
            &ids(),
        )
        .unwrap();

        assert!(!answer.passed());
        assert_eq!(
            vec![Finding {
                rule: String::from("SEC-1"),
                location: String::from("src/creds.rs:12"),
                finding: String::from("The token is logged."),
                remedy: String::from("Log the host instead."),
            }],
            answer.findings
        );
    }

    #[test]
    fn a_pass_with_findings_is_a_fail() {
        let answer = parsed(
            json!({"result": "pass", "summary": "Fine.", "findings": [finding()]}),
            &ids(),
        )
        .unwrap();

        assert!(!answer.passed());
    }

    #[test]
    fn an_answer_citing_no_rule_is_refused() {
        let mut cited = finding();
        cited["rule"] = json!("PERF-1");

        assert_eq!(
            Err(String::from("it cited PERF-1, which is no rule of doc/review.md")),
            parsed(
                json!({"result": "fail", "summary": "Slow.", "findings": [cited]}),
                &ids()
            )
        );
    }

    #[test]
    fn an_answer_of_another_shape_is_refused() {
        assert!(parsed(json!({"result": "pass"}), &ids()).is_err());
    }

    #[test]
    fn the_review_fails_when_either_reviewer_fails_or_gives_no_answer() {
        let pass = || Review {
            by: String::from("codex gpt-6.1-sol"),
            answer: Ok(Answer {
                result: String::from("pass"),
                summary: String::from("Fine."),
                findings: Vec::new(),
            }),
        };
        let silent = Review {
            by: String::from("claude claude-opus-5-5"),
            answer: Err(String::from("claude is out of quota")),
        };

        assert!(passed(&[pass(), pass()]));
        assert!(!passed(&[pass(), silent]));
    }

    #[test]
    fn the_report_lists_each_reviewers_findings() {
        let reviews = [
            Review {
                by: String::from("codex gpt-6.1-sol"),
                answer: parsed(
                    json!({"result": "fail", "summary": "It leaks.", "findings": [finding()]}),
                    &ids(),
                ),
            },
            Review {
                by: String::from("claude claude-opus-5-5"),
                answer: Err(String::from("claude is out of quota")),
            },
        ];

        assert_eq!(
            "codex gpt-6.1-sol: fail\n  It leaks.\n\n  SEC-1 at src/creds.rs:12\n    The token is logged.\n    Change: Log the host instead.\n\n\
             claude claude-opus-5-5: gave no answer: claude is out of quota\n\n\
             Review: fail\n",
            report(&reviews)
        );
    }
}
