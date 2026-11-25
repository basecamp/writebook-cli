//! `writebook tui` draws on a terminal, so without one it says so and stops
//! instead of garbling a pipe.

mod support;

use support::{stderr, writebook};

#[test]
fn needs_a_terminal() {
    let output = writebook(&["tui"], &[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("writebook tui needs a terminal"), "{}", stderr(&output));
}
