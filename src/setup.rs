//! First-run setup: install the agent skill, wire up shell completions for
//! the current shell, and end with enough instructions to get going.

use anyhow::Result;

use crate::completions;
use crate::skill;

pub fn run() -> Result<()> {
    println!("Setting up writebook...");
    println!();

    for line in skill::install_files()? {
        println!("  {line}");
    }

    install_completions();

    println!();
    println!("You're set. The basics:");
    println!();
    println!("  writebook clone <book-url>   copy a book to disk (signs you in via the browser)");
    println!("  writebook status             see what changed locally and remotely");
    println!("  writebook diff               review your changes");
    println!("  writebook pull               merge remote changes into your files");
    println!("  writebook push               upload your changes");
    println!();
    println!("A book is plain Markdown: one file per page, and index.md is the book —");
    println!("front matter up top, table of contents below. Line order is book order,");
    println!("so reordering pages means moving lines.");
    Ok(())
}

fn install_completions() {
    match current_shell() {
        Some(shell) => {
            if let Err(error) = completions::install(&shell) {
                println!("  Skipped {shell} completions ({error:#}).");
                println!("  Try `writebook shell-completion install {shell}` yourself.");
            }
        }
        None => {
            println!("  Skipped shell completions — couldn't tell your shell from $SHELL.");
            println!("  Install them manually with `writebook shell-completion install <shell>`.");
        }
    }
}

fn current_shell() -> Option<String> {
    let shell = std::env::var("SHELL").ok()?;
    let name = shell.rsplit('/').next()?.to_string();

    if name.is_empty() { None } else { Some(name) }
}
