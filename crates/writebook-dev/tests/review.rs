//! `bin/dev code review` end to end, in a throwaway git repository, with stand-ins for `cargo`,
//! `gh`, `codex` and `claude` on the PATH.

#![cfg(test)]
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

const RULES: &str =
    "## Rules\n\n### Security\n\n#### SEC-1: Tokens stay secret\n\nText.\n\n#### TEST-1: No test loosened to pass\n";

const CARGO: &str = "#!/bin/sh\nif [ \"$1\" = metadata ]; then echo '{\"packages\":[]}'; fi\n";

const GH: &str = r#"#!/bin/sh
case "$1" in
  pr)
    if [ -f "$LOG/gh.pr" ]; then cat "$LOG/gh.pr"; else echo "no pull requests found for branch" >&2; exit 1; fi ;;
  api)
    printf '%s\n' "$@" > "$LOG/gh.api" ;;
esac
"#;

const CODEX: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$LOG/codex.args"
cat > "$LOG/codex.prompt"
while [ $# -gt 0 ]; do
  if [ "$1" = "-o" ]; then cp "$LOG/codex.answer" "$2"; fi
  shift
done
"#;

const CLAUDE: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$LOG/claude.args"
echo "${CLAUDECODE-unset}" > "$LOG/claude.env"
cat > "$LOG/claude.prompt"
cat "$LOG/claude.stdout"
cat "$LOG/claude.stderr" >&2
exit "$(cat "$LOG/claude.status")"
"#;

struct Scenario {
    rules: String,
    pull_request: Option<Value>,
    codex: Value,
    claude_stdout: String,
    claude_stderr: &'static str,
    claude_status: i32,
    arguments: &'static [&'static str],
    untracked: Option<&'static str>,
}

struct Outcome {
    success: bool,
    stdout: String,
    stderr: String,
    head: String,
    base: String,
    log: TempDir,
    _repository: TempDir,
}

fn passing() -> Value {
    json!({"result": "pass", "summary": "Fine.", "findings": []})
}

fn failing(rules: &[&str]) -> Value {
    let findings: Vec<Value> = rules
        .iter()
        .map(|rule| {
            json!({
                "rule": rule,
                "location": "src/creds.rs:12",
                "finding": "The token is logged.",
                "remedy": "Log the host instead.",
            })
        })
        .collect();
    json!({"result": "fail", "summary": "It leaks.", "findings": findings})
}

fn structured(answer: &Value) -> String {
    json!({"is_error": false, "structured_output": answer}).to_string()
}

fn pull_request(head: &str) -> Value {
    json!({"headRefOid": head, "title": "Sign in with a device code", "body": "Adds {rules} and {diff}."})
}

impl Scenario {
    fn new() -> Self {
        Self {
            rules: RULES.to_string(),
            pull_request: Some(pull_request("{head}")),
            codex: passing(),
            claude_stdout: structured(&passing()),
            claude_stderr: "",
            claude_status: 0,
            arguments: &[],
            untracked: None,
        }
    }

    fn run(self) -> Outcome {
        let repository = tempfile::tempdir().expect("a temporary directory");
        let work = repository.path().join("work");
        let origin = repository.path().join("origin.git");
        let bin = repository.path().join("bin");
        let log = tempfile::tempdir().expect("a temporary directory");

        git(repository.path(), &["init", "--bare", "-q", "-b", "main", "origin.git"]);
        fs::create_dir_all(work.join(".agents/skills/code-style")).expect("the skill directory is made");
        fs::create_dir_all(work.join("doc")).expect("the doc directory is made");
        fs::write(work.join("doc/review.md"), &self.rules).expect("the rules are written");
        fs::write(work.join("AGENTS.md"), "Use bin/dev.\n").expect("AGENTS.md is written");
        fs::write(work.join(".agents/skills/code-style/SKILL.md"), "Write clearly.\n").expect("the skill is written");
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "Start"]);
        git(&work, &["remote", "add", "origin", &origin.display().to_string()]);
        git(&work, &["push", "-q", "origin", "main"]);
        let base = git(&work, &["rev-parse", "HEAD"]);
        git(&work, &["checkout", "-q", "-b", "feature"]);
        fs::write(work.join("change.txt"), "sign_in\n").expect("the change is written");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "Sign in"]);
        let head = git(&work, &["rev-parse", "HEAD"]);
        if let Some(name) = self.untracked {
            fs::write(work.join(name), "mod new_module;\n").expect("the untracked file is written");
        }

        fs::create_dir_all(&bin).expect("the bin directory is made");
        for (name, script) in [("cargo", CARGO), ("gh", GH), ("codex", CODEX), ("claude", CLAUDE)] {
            install(&bin, name, script);
        }
        if let Some(found) = &self.pull_request {
            write(log.path(), "gh.pr", &found.to_string().replace("{head}", &head));
        }
        write(log.path(), "codex.answer", &self.codex.to_string());
        write(log.path(), "claude.stdout", &self.claude_stdout);
        write(log.path(), "claude.stderr", self.claude_stderr);
        write(log.path(), "claude.status", &self.claude_status.to_string());

        let path = format!("{}:{}", bin.display(), std::env::var("PATH").expect("PATH is set"));
        let done = Command::new(env!("CARGO_BIN_EXE_writebook-dev"))
            .args(["code", "review"])
            .args(self.arguments)
            .current_dir(&work)
            .env("PATH", path)
            .env("LOG", log.path())
            .env("CLAUDECODE", "1")
            .output()
            .expect("writebook-dev runs");

        Outcome {
            success: done.status.success(),
            stdout: String::from_utf8(done.stdout).expect("its output is text"),
            stderr: String::from_utf8(done.stderr).expect("its errors are text"),
            head,
            base,
            log,
            _repository: repository,
        }
    }
}

impl Outcome {
    fn logged(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.log.path().join(name)).ok()
    }

    fn lines(&self, name: &str) -> Vec<String> {
        self.logged(name)
            .unwrap_or_else(|| panic!("{name} was logged"))
            .lines()
            .map(String::from)
            .collect()
    }
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let done = Command::new("git")
        .args([
            "-c",
            "user.name=Dev",
            "-c",
            "user.email=dev@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("git runs");
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    String::from_utf8(done.stdout)
        .expect("its output is text")
        .trim()
        .to_string()
}

fn install(bin: &Path, name: &str, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    let path: PathBuf = bin.join(name);
    fs::write(&path, script).expect("the script is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("the script is executable");
}

fn write(directory: &Path, name: &str, contents: &str) {
    fs::write(directory.join(name), contents).expect("the file is written");
}

#[test]
fn both_reviewers_read_the_branch_and_a_pass_is_posted_on_the_pull_request() {
    let outcome = Scenario::new().run();
    let head = &outcome.head;

    assert!(outcome.success);
    assert_eq!(
        format!(
            "codex gpt-6.1-sol: pass\n  Fine.\n\nclaude claude-opus-5-5: pass\n  Fine.\n\nReview: pass\nPosted the verdict as the `review` status on {head}.\n"
        ),
        outcome.stdout
    );

    let prompt = outcome.logged("codex.prompt").expect("codex was asked");
    assert_eq!(Some(prompt.clone()), outcome.logged("claude.prompt"));
    assert!(prompt.contains(&format!("on main at {}", outcome.base)));
    assert!(prompt.contains(&format!("change's head, {head}")));
    assert!(prompt.contains(&format!("<rules>\n{}\n</rules>", RULES.trim_end())));
    assert!(prompt.contains("<file path=\"AGENTS.md\">\nUse bin/dev.\n</file>"));
    assert!(prompt.contains("<file path=\".agents/skills/code-style/SKILL.md\">\nWrite clearly.\n</file>"));
    assert!(prompt.contains("<description>\nSign in with a device code\n\nAdds {rules} and {diff}.\n</description>"));
    assert!(prompt.contains("+sign_in\n</diff>"));

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
        ],
        outcome.lines("codex.args")[..14]
    );
    assert_eq!("-", outcome.lines("codex.args")[17]);

    let mut claude = outcome.lines("claude.args");
    let schema: Value = serde_json::from_str(&claude[8]).expect("the schema is JSON");
    claude[8] = String::from("SCHEMA");
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
            "SCHEMA",
            "--tools",
            "Read,Grep,Glob",
            "--allowedTools",
            "Read,Grep,Glob",
            "--strict-mcp-config",
            "--safe-mode",
            "--no-session-persistence",
        ],
        claude[..]
    );
    assert_eq!(
        json!(["SEC-1", "TEST-1"]),
        schema["properties"]["findings"]["items"]["properties"]["rule"]["enum"]
    );
    assert_eq!(["unset"], outcome.lines("claude.env")[..]);

    assert_eq!(
        [
            "api".to_string(),
            "-X".to_string(),
            "POST".to_string(),
            format!("repos/basecamp/writebook-cli/statuses/{head}"),
            "-f".to_string(),
            "state=success".to_string(),
            "-f".to_string(),
            "context=review".to_string(),
            "-f".to_string(),
            "description=pass".to_string(),
        ],
        outcome.lines("gh.api")[..]
    );
}

#[test]
fn a_branch_without_a_pull_request_is_reviewed_but_not_posted() {
    let mut scenario = Scenario::new();
    scenario.pull_request = None;

    let outcome = scenario.run();

    assert!(outcome.success);
    assert!(
        outcome
            .stdout
            .ends_with("Not posted: the branch has no pull request.\n")
    );
    assert!(
        outcome
            .logged("codex.prompt")
            .expect("codex was asked")
            .contains("<description>\nThe branch has no pull request, so there is no description.\n</description>")
    );
    assert_eq!(None, outcome.logged("gh.api"));
}

#[test]
fn a_file_that_isnt_committed_stops_the_review_before_anyone_reads_it() {
    let mut scenario = Scenario::new();
    scenario.untracked = Some("new_module.rs");

    let outcome = scenario.run();

    assert!(!outcome.success);
    assert!(
        outcome
            .stderr
            .contains("the worktree has changes or files that aren't committed"),
        "{}",
        outcome.stderr
    );
    assert_eq!(None, outcome.logged("codex.prompt"));
    assert_eq!(None, outcome.logged("gh.api"));
}

#[test]
fn no_post_prints_the_verdict_and_posts_nothing() {
    let mut scenario = Scenario::new();
    scenario.arguments = &["--no-post"];

    let outcome = scenario.run();

    assert!(outcome.success);
    assert!(outcome.stdout.ends_with("Not posted: --no-post was given.\n"));
    assert_eq!(None, outcome.logged("gh.api"));
}

#[test]
fn a_pull_request_whose_head_is_another_commit_is_not_posted_on() {
    let mut scenario = Scenario::new();
    scenario.pull_request = Some(pull_request("old"));

    let outcome = scenario.run();

    assert!(outcome.success);
    assert!(
        outcome
            .stdout
            .ends_with("Not posted: the pull request's head isn't this commit, so push it first.\n")
    );
    assert_eq!(None, outcome.logged("gh.api"));
}

#[test]
fn a_failing_review_names_the_rules_and_the_silent_reviewer_and_posts_a_failure() {
    let mut scenario = Scenario::new();
    scenario.codex = failing(&["SEC-1", "TEST-1", "SEC-1"]);
    scenario.claude_stdout = json!({"is_error": true, "result": "Credit balance is too low"}).to_string();
    scenario.claude_stderr = "claude is out of quota";
    scenario.claude_status = 1;

    let outcome = scenario.run();

    assert!(!outcome.success);
    assert_eq!(
        format!(
            "codex gpt-6.1-sol: fail\n  It leaks.\n\n  SEC-1 at src/creds.rs:12\n    The token is logged.\n    Change: Log the host instead.\n\n  TEST-1 at src/creds.rs:12\n    The token is logged.\n    Change: Log the host instead.\n\n  SEC-1 at src/creds.rs:12\n    The token is logged.\n    Change: Log the host instead.\n\n\
             claude claude-opus-5-5: gave no answer: claude gave no verdict: claude is out of quota\n\n\
             Review: fail\nPosted the verdict as the `review` status on {}.\n",
            outcome.head
        ),
        outcome.stdout
    );
    let posted = outcome.lines("gh.api");
    assert_eq!("state=failure", posted[5]);
    assert_eq!(
        "description=fail: SEC-1, TEST-1, claude claude-opus-5-5 gave no answer",
        posted[9]
    );
}

#[test]
fn an_answer_that_passes_with_findings_or_cites_no_rule_or_has_another_shape_does_not_pass() {
    let mut cited = failing(&["SEC-1"]);
    cited["findings"][0]["rule"] = json!("PERF-1");

    let mut scenario = Scenario::new();
    scenario.codex = json!({"result": "pass", "summary": "Fine.", "findings": failing(&["SEC-1"])["findings"]});
    scenario.claude_stdout = structured(&cited);
    let outcome = scenario.run();

    assert!(!outcome.success);
    assert!(
        outcome
            .stdout
            .starts_with("codex gpt-6.1-sol: fail\n  Fine.\n\n  SEC-1 at src/creds.rs:12\n")
    );
    assert!(
        outcome
            .stdout
            .contains("claude claude-opus-5-5: gave no answer: it cited PERF-1, which is no rule of doc/review.md\n")
    );

    let mut scenario = Scenario::new();
    scenario.codex = json!({"result": "pass"});
    let outcome = scenario.run();

    assert!(!outcome.success);
    assert!(
        outcome
            .stdout
            .starts_with("codex gpt-6.1-sol: gave no answer: missing field `summary`\n")
    );
}

#[test]
fn a_failed_review_with_a_claude_error_it_calls_a_success_shows_what_claude_said() {
    let said = json!({"is_error": true, "result": "Credit balance is too low"}).to_string();
    let mut scenario = Scenario::new();
    scenario.claude_stdout = said.clone();

    let outcome = scenario.run();

    assert!(!outcome.success);
    assert!(outcome.stdout.contains(&format!(
        "claude claude-opus-5-5: gave no answer: claude gave no verdict: {said}\n"
    )));
}

#[test]
fn a_long_description_is_cut_to_what_github_takes() {
    let headings: Vec<String> = (1..=40)
        .map(|rule| format!("#### TEST-{rule}: Rule {rule}\n"))
        .collect();
    let rules: Vec<String> = (1..=40).map(|rule| format!("TEST-{rule}")).collect();
    let rules: Vec<&str> = rules.iter().map(String::as_str).collect();
    let mut scenario = Scenario::new();
    scenario.rules = headings.join("\n");
    scenario.codex = failing(&rules);

    let outcome = scenario.run();

    let description = outcome.lines("gh.api")[9].clone();
    assert!(description.starts_with("description=fail: TEST-1, TEST-2, TEST-3"));
    assert_eq!(140, description.trim_start_matches("description=").chars().count());
}
