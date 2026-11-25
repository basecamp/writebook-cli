//! Putting text on the clipboard through the terminal (OSC 52), which works
//! over SSH too.

use std::io::Write;

pub fn copy(text: &str) {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{}\x07", base64(text.as_bytes()));
    let _ = stdout.flush();
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();

    for chunk in bytes.chunks(3) {
        let group = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));

        for position in 0..4 {
            if position <= chunk.len() {
                encoded.push(ALPHABET[(group >> (18 - 6 * position) & 63) as usize] as char);
            } else {
                encoded.push('=');
            }
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_clipboard_text_as_base64() {
        assert_eq!(base64(b"https://books.example.com/2/guide"), "aHR0cHM6Ly9ib29rcy5leGFtcGxlLmNvbS8yL2d1aWRl");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
    }
}
