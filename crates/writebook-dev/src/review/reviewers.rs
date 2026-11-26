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
