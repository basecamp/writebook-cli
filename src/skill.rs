//! The agent skill embedded in the binary: SKILL.md teaches coding agents how
//! to work on a checkout. It installs to the shared ~/.agents/skills/ home and
//! gets symlinked into ~/.claude/skills/ when Claude Code is around.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const SKILL: &str = include_str!("../skills/writebook/SKILL.md");

pub fn print() -> Result<()> {
    print!("{SKILL}");
    Ok(())
}

pub fn install() -> Result<()> {
    for line in install_files()? {
        println!("{line}");
    }

    Ok(())
}

pub fn install_files() -> Result<Vec<String>> {
    let home = dirs::home_dir().expect("could not determine the home directory");
    let skill_dir = home.join(".agents/skills/writebook");

    fs::create_dir_all(&skill_dir)?;
    fs::write(skill_dir.join("SKILL.md"), SKILL)
        .with_context(|| format!("could not write {}", skill_dir.join("SKILL.md").display()))?;

    let mut lines = vec!["Installed the agent skill to ~/.agents/skills/writebook/SKILL.md".to_string()];

    if home.join(".claude").is_dir() {
        lines.push(link_into_claude(&home)?);
    }

    Ok(lines)
}

#[cfg(unix)]
fn link_into_claude(home: &Path) -> Result<String> {
    let links_dir = home.join(".claude/skills");
    let link = links_dir.join("writebook");
    let target = std::path::PathBuf::from("../../.agents/skills/writebook");

    fs::create_dir_all(&links_dir)?;

    match fs::symlink_metadata(&link) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() && fs::read_link(&link).ok() == Some(target.clone()) {
                Ok("Claude Code was already linked at ~/.claude/skills/writebook".to_string())
            } else {
                Ok("Left ~/.claude/skills/writebook alone — something else already lives there".to_string())
            }
        }
        Err(_) => {
            std::os::unix::fs::symlink(&target, &link).with_context(|| format!("could not link {}", link.display()))?;
            Ok("Linked ~/.claude/skills/writebook → ../../.agents/skills/writebook".to_string())
        }
    }
}

#[cfg(windows)]
fn link_into_claude(home: &Path) -> Result<String> {
    let skill = home.join(".claude/skills/writebook/SKILL.md");

    let ours = match fs::read_to_string(&skill) {
        Ok(existing) => existing.lines().any(|line| line == "name: writebook"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(error).with_context(|| format!("could not read {}", skill.display())),
    };

    if ours {
        fs::create_dir_all(skill.parent().unwrap())?;
        fs::write(&skill, SKILL).with_context(|| format!("could not write {}", skill.display()))?;
        Ok("Installed the agent skill for Claude Code to ~/.claude/skills/writebook/SKILL.md".to_string())
    } else {
        Ok("Left ~/.claude/skills/writebook alone — something else already lives there".to_string())
    }
}
