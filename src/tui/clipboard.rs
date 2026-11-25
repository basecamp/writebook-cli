//! Putting text on the clipboard through the terminal (OSC 52), which works
//! over SSH too.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::io::Write;

pub fn copy(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "{}", sequence(text));
    let _ = stdout.flush();
}

fn sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_clipboard_text_as_base64() {
        assert_eq!(sequence("https://books.example.com/2/guide"), "\x1b]52;c;aHR0cHM6Ly9ib29rcy5leGFtcGxlLmNvbS8yL2d1aWRl\x07");
        assert_eq!(sequence("ab"), "\x1b]52;c;YWI=\x07");
    }
}
