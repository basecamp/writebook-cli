//! How list commands print: a table for people by default, or Markdown or
//! JSON when asked for.

pub enum Format {
    Table,
    Markdown,
    Json,
}

impl Format {
    pub fn from_flags(json: bool, markdown: bool) -> Format {
        if json {
            Format::Json
        } else if markdown {
            Format::Markdown
        } else {
            Format::Table
        }
    }
}

/// Columns padded to line up, two spaces apart. The first row is the header.
pub fn print_table<const COLUMNS: usize>(rows: &[[String; COLUMNS]]) {
    let widths: Vec<usize> = (0..COLUMNS).map(|column| rows.iter().map(|row| row[column].chars().count()).max().unwrap_or(0)).collect();

    for row in rows {
        let cells: Vec<String> = row.iter().zip(&widths).map(|(cell, width)| format!("{cell:width$}")).collect();
        println!("{}", cells.join("  ").trim_end());
    }
}
