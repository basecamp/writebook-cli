//! Putting text on the clipboard through the terminal (OSC 52), which works
//! over SSH too.

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::io::Write;
use url::Url;

pub fn copy(text: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    write!(stdout, "{}", sequence(text))?;
    stdout.flush()
}

fn sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// Whether a link from Writebook is safe to copy or show as a QR code: a web
/// address on the instance at `host`, with nothing a terminal would act on.
pub fn shareable(link: &str, host: &str) -> Result<()> {
    if link.chars().any(char::is_control) {
        bail!("Writebook's link has control characters in it, so it isn't shared");
    }

    let url = Url::parse(link).context("Writebook's link isn't a web address, so it isn't shared")?;
    let instance = Url::parse(host).context("the instance has no web address")?;

    if !matches!(url.scheme(), "http" | "https") {
        bail!("Writebook's link isn't a web address, so it isn't shared");
    } else if url.origin() != instance.origin() {
        bail!("Writebook's link goes to another site, so it isn't shared");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_clipboard_text_as_base64() {
        assert_eq!(
            sequence("https://books.example.com/2/guide"),
            "\x1b]52;c;aHR0cHM6Ly9ib29rcy5leGFtcGxlLmNvbS8yL2d1aWRl\x07"
        );
        assert_eq!(sequence("ab"), "\x1b]52;c;YWI=\x07");
    }

    #[test]
    fn shares_only_web_links_on_the_instance() {
        let host = "https://books.example.com";

        assert!(shareable("https://books.example.com/join/abc", host).is_ok());
        assert!(shareable("https://books.example.com:443/2/guide", host).is_ok());
        assert!(shareable("https://evil.example.org/join/abc", host).is_err());
        assert!(shareable("http://books.example.com/join/abc", host).is_err());
        assert!(shareable("javascript:alert(1)", host).is_err());
        assert!(shareable("https://books.example.com/join/\x1b]52;c;bad\x07", host).is_err());
        assert!(shareable("https://books.example.com/join/a\nb", host).is_err());
        assert!(shareable("/join/abc", host).is_err());
    }
}
