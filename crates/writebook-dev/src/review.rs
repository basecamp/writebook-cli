//! `bin/dev code review`: the branch's diff against main, held to main's `doc/review.md` by two
//! reviewers at once, with the verdict posted as the `review` status on the pull request's head.

mod posting;
mod reviewers;
mod verdict;

use std::error::Error;
use std::process::Command;

use posting::{Posting, PullRequest};

const REPOSITORY: &str = "basecamp/writebook-cli";
const RULES: &str = "doc/review.md";
const GUIDANCE: [&str; 2] = ["AGENTS.md", ".agents/skills/code-style/SKILL.md"];
const PROMPT: &str = include_str!("review/prompt.md");

pub fn run(post: bool) -> Result<(), Box<dyn Error>> {
    crate::lint::run()?;
    refuse_uncommitted()?;

    let base = fetched_main()?;
    let head = git(&["rev-parse", "HEAD"])?;
    let merge_base = git(&["merge-base", &base, &head])?;
    let diff = git(&["diff", &format!("{merge_base}..{head}")])?;
    let rules = git(&["show", &format!("{base}:{RULES}")])?;
    let guidance = guidance_on(&base)?;
    let pull_request = posting::pull_request()?;

    let prompt = prompt(&base, &head, &rules, &guidance, pull_request.as_ref(), &diff);
    let reviews = reviewers::ask(&prompt, &verdict::rule_ids(&rules))?;
    print!("{}", verdict::report(&reviews));

    match posting::posting(post, pull_request.as_ref(), &head) {
        Posting::Post => {
            posting::post(&head, &reviews)?;
            println!("Posted the verdict as the `{}` status on {head}.", posting::CONTEXT);
        }
        Posting::Skip(reason) => println!("Not posted: {reason}."),
    }

    if verdict::passed(&reviews) {
        Ok(())
    } else {
        Err("the review failed".into())
    }
}

fn refuse_uncommitted() -> Result<(), Box<dyn Error>> {
    if git(&["status", "--porcelain"])?.is_empty() {
        Ok(())
    } else {
        Err(
            "the worktree has changes or files that aren't committed: a review reads commits, so commit them first"
                .into(),
        )
    }
}

fn fetched_main() -> Result<String, Box<dyn Error>> {
    if git(&["fetch", "--quiet", "origin", "main"]).is_ok() {
        git(&["rev-parse", "origin/main"])
    } else {
        git(&[
            "-c",
            "credential.helper=!gh auth git-credential",
            "fetch",
            "--quiet",
            &format!("https://github.com/{REPOSITORY}.git"),
            "main",
        ])?;
        git(&["rev-parse", "FETCH_HEAD"])
    }
}

fn guidance_on(base: &str) -> Result<String, Box<dyn Error>> {
    let mut guidance = String::new();
    for path in GUIDANCE {
        let text = git(&["show", &format!("{base}:{path}")])?;
        guidance.push_str(&format!("<file path=\"{path}\">\n{text}\n</file>\n"));
    }
    Ok(guidance)
}

fn prompt(
    base: &str,
    head: &str,
    rules: &str,
    guidance: &str,
    pull_request: Option<&PullRequest>,
    diff: &str,
) -> String {
    let description = match pull_request {
        Some(pull_request) => format!("{}\n\n{}", pull_request.title, pull_request.body),
        None => String::from("The branch has no pull request, so there is no description."),
    };
    filled(
        PROMPT,
        &[
            ("base", base),
            ("head", head),
            ("rules", rules),
            ("guidance", guidance),
            ("description", &description),
            ("diff", diff),
        ],
    )
}

fn filled(template: &str, values: &[(&str, &str)]) -> String {
    let mut filled = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        filled.push_str(&rest[..start]);
        rest = &rest[start..];
        let found = values.iter().find(|(name, _)| {
            rest.strip_prefix('{')
                .and_then(|after| after.strip_prefix(name))
                .is_some_and(|after| after.starts_with('}'))
        });
        if let Some((name, value)) = found {
            filled.push_str(value);
            rest = &rest[name.len() + 2..];
        } else {
            filled.push('{');
            rest = &rest[1..];
        }
    }
    filled.push_str(rest);
    filled
}

fn git(arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    run_tool("git", arguments)
}

fn gh(arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    run_tool("gh", arguments)
}

fn run_tool(program: &str, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    let done = Command::new(program).args(arguments).output()?;
    if done.status.success() {
        Ok(String::from_utf8(done.stdout)?.trim_end().to_string())
    } else {
        Err(format!(
            "{program} {} failed: {}",
            arguments.first().unwrap_or(&""),
            String::from_utf8_lossy(&done.stderr).trim()
        )
        .into())
    }
}
