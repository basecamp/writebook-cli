use ratatui::style::Color;

/// A book theme's color in the terminal, white for a book without one.
pub fn color(theme: Option<&str>) -> Color {
    match theme {
        Some("black") => Color::DarkGray,
        Some("blue") => Color::Blue,
        Some("green") => Color::Green,
        Some("magenta") => Color::Magenta,
        Some("orange") => Color::Indexed(208),
        Some("violet") => Color::Indexed(141),
        _ => Color::White,
    }
}
