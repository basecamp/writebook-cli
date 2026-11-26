//! Windows has no file mode, so the 0700 and 0600 of Unix become an access
//! list that names only the signed-in user. It's protected, so nothing the
//! folders above allow carries over, and files created inside inherit it.
//! icacls sets it, so the CLI needs no unsafe calls into the Win32 API.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

/// In three steps, since icacls given them at once leaves entries behind:
/// drop any added ones, grant the user, then drop the inherited ones. A path
/// already restricted is left as it is: rewriting its list briefly locks out
/// another command reading a file inside it.
pub fn restrict(path: &Path) -> Result<()> {
    let user = current_user()?;
    let inheritance = if path.is_dir() { "(OI)(CI)" } else { "" };
    let only_the_user = format!("{}:{inheritance}(F)", user.name);

    if !matches!(entries(path)?.as_slice(), [entry] if entry.eq_ignore_ascii_case(&only_the_user)) {
        icacls(path, &["/reset"])?;
        icacls(path, &["/grant:r", &format!("*{}:{inheritance}F", user.sid)])?;
        icacls(path, &["/inheritance:r"])?;
    }
    Ok(())
}

fn entries(path: &Path) -> Result<Vec<String>> {
    let listing = icacls(path, &[])?;
    let shown = path.display().to_string();

    Ok(listing
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .map(|line| line.strip_prefix(shown.as_str()).unwrap_or(line).trim().to_string())
        .collect())
}

fn icacls(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("icacls")
        .arg(path)
        .args(args)
        .output()
        .context("could not run icacls")?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if output.status.success() {
        Ok(said)
    } else {
        bail!("could not restrict {} to the current user: {said}", path.display())
    }
}

struct User {
    name: String,
    sid: String,
}

fn current_user() -> Result<&'static User> {
    static USER: OnceLock<User> = OnceLock::new();

    match USER.get() {
        Some(user) => Ok(user),
        None => {
            let output = Command::new("whoami")
                .args(["/user", "/fo", "csv", "/nh"])
                .output()
                .context("could not run whoami")?;
            let line = String::from_utf8_lossy(&output.stdout);
            let fields: Vec<&str> = line.trim().split(',').map(|it| it.trim_matches('"')).collect();

            match fields.as_slice() {
                [name, sid] if output.status.success() && sid.starts_with("S-") => Ok(USER.get_or_init(|| User {
                    name: name.to_string(),
                    sid: sid.to_string(),
                })),
                _ => bail!("could not look up the current user"),
            }
        }
    }
}
