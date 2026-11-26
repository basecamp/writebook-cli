//! The reviewers in config.toml, asked at once, each read-only in the repository and handed the
//! prompt on stdin, since a diff can outgrow what one command-line argument may hold.

use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;

use serde::Deserialize;
use serde_json::Value;

use super::verdict::{self, Review};

const CONFIG: &str = include_str!("config.toml");
const READING_TOOLS: &str = "Read,Grep,Glob";
// No MCP server, no ChatGPT apps, no web search: the command line alone sets model, effort and sandbox.
const CODEX_BARE: [&str; 5] = [
    "--ignore-user-config",
    "--disable",
    "apps",
    "-c",
    "web_search=\"disabled\"",
];
// A fresh reviewer session gets none of these; CLAUDE_CODE_EFFORT_LEVEL would outrank its --effort.
const SESSION_VARIABLES: [&str; 11] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_EFFORT_LEVEL",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
];

#[derive(Deserialize)]
struct Config {
    reviewers: Vec<Reviewer>,
}

#[derive(Deserialize)]
struct Reviewer {
    tool: Tool,
    model: String,
    effort: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Tool {
    Codex,
    Claude,
}

/// Every reviewer's review, in the config's order, whether it answered or not.
pub fn ask(prompt: &str, ids: &[String]) -> Result<Vec<Review>, Box<dyn Error>> {
    let config: Config = toml::from_str(CONFIG)?;
    let schema = verdict::schema(ids).to_string();
    let answers = tempfile::Builder::new().prefix("writebook-dev-review-").tempdir()?;
    fs::write(answers.path().join("schema.json"), &schema)?;

    Ok(thread::scope(|scope| {
        let asking: Vec<_> = config
            .reviewers
            .iter()
            .map(|reviewer| scope.spawn(|| reviewer.review(prompt, ids, &schema, answers.path())))
            .collect();
        asking
            .into_iter()
            .map(|asked| asked.join().expect("a reviewer doesn't panic"))
            .collect()
    }))
}

impl Reviewer {
    fn review(&self, prompt: &str, ids: &[String], schema: &str, answers: &Path) -> Review {
        let (by, answer) = match self.tool {
            Tool::Codex => ("codex", self.codex(prompt, answers)),
            Tool::Claude => ("claude", self.claude(prompt, schema)),
        };
        Review {
            by: format!("{by} {}", self.model),
            answer: answer.and_then(|answer| verdict::parsed(answer, ids)),
        }
    }

    fn codex(&self, prompt: &str, answers: &Path) -> Result<Value, String> {
        let answer = answers.join("answer-codex.json");
        let done = asked(codex(&self.model, &self.effort, answers, &answer), prompt)
            .map_err(|error| format!("codex could not be run: {error}"))?;
        if done.status.success() {
            let read = fs::read_to_string(&answer).map_err(|error| error.to_string())?;
            serde_json::from_str(&read).map_err(|error| error.to_string())
        } else {
            Err(format!(
                "codex gave no verdict: {}",
                last_said(&done.stdout, &done.stderr)
            ))
        }
    }

    fn claude(&self, prompt: &str, schema: &str) -> Result<Value, String> {
        let done = asked(claude(&self.model, &self.effort, schema), prompt)
            .map_err(|error| format!("claude could not be run: {error}"))?;
        structured(done.status.success(), &done.stdout, &done.stderr)
    }
}

fn codex(model: &str, effort: &str, answers: &Path, answer: &Path) -> Command {
    let mut command = Command::new("codex");
    command
        .args(["exec", "-m", model, "-c"])
        .arg(format!("model_reasoning_effort={effort}"))
        .args(["--sandbox", "read-only", "--skip-git-repo-check"])
        .args(CODEX_BARE)
        .arg("--output-schema")
        .arg(answers.join("schema.json"))
        .arg("-o")
        .arg(answer)
        .arg("-");
    command
}

fn claude(model: &str, effort: &str, schema: &str) -> Command {
    let mut command = Command::new("claude");
    command
        .args(["-p", "--model", model, "--effort", effort])
        .args(["--output-format", "json", "--json-schema", schema])
        .args(["--tools", READING_TOOLS, "--allowedTools", READING_TOOLS])
        .args(["--strict-mcp-config", "--safe-mode", "--no-session-persistence"]);
    for variable in SESSION_VARIABLES {
        command.env_remove(variable);
    }
    command
}

fn asked(mut command: Command, prompt: &str) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    thread::scope(|scope| {
        let writing = scope.spawn(move || stdin.write_all(prompt.as_bytes()));
        let done = child.wait_with_output()?;
        let written = writing.join().expect("writing the prompt doesn't panic");
        if done.status.success() {
            written?;
        }
        Ok(done)
    })
}

fn structured(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<Value, String> {
    let result: Value = serde_json::from_slice(stdout).unwrap_or(Value::Null);
    match result.get("structured_output") {
        Some(found) if success && result["is_error"] != true => Ok(found.clone()),
        _ => Err(format!("claude gave no verdict: {}", last_said(stdout, stderr))),
    }
}

fn last_said(stdout: &[u8], stderr: &[u8]) -> String {
    let said = String::from_utf8_lossy(stderr);
    let said = if said.trim().is_empty() {
        String::from_utf8_lossy(stdout)
    } else {
        said
    };
    let lines: Vec<&str> = said.trim().lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn arguments(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_config_asks_codex_and_claude_one_model_each() {
        let config: Config = toml::from_str(CONFIG).unwrap();

        let asked: Vec<_> = config
            .reviewers
            .iter()
            .map(|reviewer| (&reviewer.tool, reviewer.model.as_str(), reviewer.effort.as_str()))
            .collect();
        assert!(matches!(
            asked[..],
            [
                (Tool::Codex, "gpt-6.1-sol", "high"),
                (Tool::Claude, "claude-opus-5-5", "high")
            ]
        ));
    }

    #[test]
    fn codex_reviews_read_only_with_the_prompt_on_stdin() {
        let command = codex(
            "gpt-6.1-sol",
            "high",
            Path::new("/answers"),
            Path::new("/answers/answer-codex.json"),
        );

        assert_eq!(
            [
                "exec",
                "-m",
                "gpt-6.1-sol",
                "-c",
                "model_reasoning_effort=high",
                "--sandbox",
                "read-only",
                "--skip-git-repo-check",
                "--ignore-user-config",
                "--disable",
                "apps",
                "-c",
                "web_search=\"disabled\"",
                "--output-schema",
                "/answers/schema.json",
                "-o",
                "/answers/answer-codex.json",
                "-",
            ]
            .map(String::from)
            .to_vec(),
            arguments(&command)
        );
    }

    #[test]
    fn claude_reviews_with_only_the_tools_that_read() {
        let command = claude("claude-opus-5-5", "high", "{}");

        assert_eq!(
            [
                "-p",
                "--model",
                "claude-opus-5-5",
                "--effort",
                "high",
                "--output-format",
                "json",
                "--json-schema",
                "{}",
                "--tools",
                "Read,Grep,Glob",
                "--allowedTools",
                "Read,Grep,Glob",
                "--strict-mcp-config",
                "--safe-mode",
                "--no-session-persistence",
            ]
            .map(String::from)
            .to_vec(),
            arguments(&command)
        );
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == "CLAUDECODE" && value.is_none())
        );
    }

    #[test]
    fn claudes_verdict_is_its_structured_output() {
        let stdout = json!({"is_error": false, "structured_output": {"result": "pass"}}).to_string();

        assert_eq!(Ok(json!({"result": "pass"})), structured(true, stdout.as_bytes(), b""));
    }

    #[test]
    fn claude_failing_gives_no_verdict() {
        let stdout = json!({"is_error": true, "result": "Credit balance is too low"}).to_string();

        assert_eq!(
            Err(String::from("claude gave no verdict: out of credit")),
            structured(false, stdout.as_bytes(), b"out of credit")
        );
        assert_eq!(
            Err(format!("claude gave no verdict: {stdout}")),
            structured(true, stdout.as_bytes(), b"")
        );
    }
}
