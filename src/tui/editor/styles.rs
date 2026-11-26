//! Markdown styled as it's typed, like hey's composer: the Markdown stays as
//! typed, **bold** turns bold, *italic* italic, ~~struck~~ struck through,
//! `code` and # headings take their colors, and the markers themselves dim. A
//! span styles only once its markers pair up on one line, so half-typed markup
//! stays plain.

use ratatui::style::{Color, Modifier, Style};

use crate::tui::markdown::{CODE, HEADING};

/// How each character of a line of Markdown is styled as it's written.
pub fn markdown_styles(line: &[char]) -> Vec<Style> {
    let mut styles = vec![Style::default(); line.len()];
    let marker = Style::default().add_modifier(Modifier::DIM);

    let hashes = line.iter().take_while(|it| **it == '#').count();
    if (1..=6).contains(&hashes) && line.get(hashes) == Some(&' ') {
        for (index, style) in styles.iter_mut().enumerate() {
            *style = if index < hashes {
                marker
            } else {
                Style::default().fg(HEADING).add_modifier(Modifier::BOLD)
            };
        }
        styles
    } else {
        mark_inline(line, styles, marker)
    }
}

fn mark_inline(line: &[char], mut styles: Vec<Style>, marker: Style) -> Vec<Style> {
    let in_code = mark_code(line, &mut styles, marker);
    mark_links(line, &mut styles, &in_code, marker);
    mark_pairs(line, &mut styles, &in_code, "**", Modifier::BOLD, marker);
    mark_pairs(line, &mut styles, &in_code, "__", Modifier::BOLD, marker);
    mark_pairs(line, &mut styles, &in_code, "~~", Modifier::CROSSED_OUT, marker);
    mark_pairs(line, &mut styles, &in_code, "*", Modifier::ITALIC, marker);
    mark_pairs(line, &mut styles, &in_code, "_", Modifier::ITALIC, marker);

    if line.first() == Some(&'>') {
        styles[0] = marker;
    }

    styles
}

fn mark_links(line: &[char], styles: &mut [Style], in_code: &[bool], marker: Style) {
    let link = Style::default().fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
    let mut index = 0;

    while index < line.len() {
        if line[index] != '[' || in_code[index] {
            index += 1;
            continue;
        }

        let image = index > 0 && line[index - 1] == '!';
        let Some(close) = (index + 1..line.len()).find(|it| line[*it] == ']') else {
            break;
        };
        let Some(end) = (line.get(close + 1) == Some(&'('))
            .then(|| (close + 2..line.len()).find(|it| line[*it] == ')'))
            .flatten()
        else {
            index += 1;
            continue;
        };

        let start = if image { index - 1 } else { index };
        for (position, style) in styles.iter_mut().enumerate().take(end + 1).skip(start) {
            *style = if position > index && position < close {
                link
            } else {
                marker
            };
        }

        index = end + 1;
    }
}

fn mark_code(line: &[char], styles: &mut [Style], marker: Style) -> Vec<bool> {
    let mut inside = vec![false; line.len()];
    let ticks: Vec<usize> = (0..line.len()).filter(|index| line[*index] == '`').collect();

    for pair in ticks.chunks(2) {
        if let [open, close] = pair {
            for index in *open..=*close {
                inside[index] = true;
                styles[index] = if index == *open || index == *close {
                    marker
                } else {
                    Style::default().fg(CODE)
                };
            }
        }
    }

    inside
}

fn mark_pairs(
    line: &[char],
    styles: &mut [Style],
    in_code: &[bool],
    delimiter: &str,
    modifier: Modifier,
    marker: Style,
) {
    let delimiter: Vec<char> = delimiter.chars().collect();
    let width = delimiter.len();
    let mut open: Option<usize> = None;
    let mut index = 0;

    while index + width <= line.len() {
        let here = line[index..index + width] == delimiter[..]
            && !in_code[index]
            && styles[index].add_modifier.is_empty()
            && line.get(index + width) != Some(&delimiter[0])
            && (index == 0 || line[index - 1] != delimiter[0]);

        if !here {
            index += 1;
            continue;
        }

        let before = index.checked_sub(1).map(|it| line[it]);
        let after = line.get(index + width).copied();
        let word_edge = delimiter[0] != '_'
            || (before.is_none_or(|it| !it.is_alphanumeric()) && after.is_none_or(|it| !it.is_alphanumeric()))
            || (before.is_some_and(char::is_alphanumeric) != after.is_some_and(char::is_alphanumeric));

        match open {
            Some(start) if before.is_some_and(|it| !it.is_whitespace()) && word_edge => {
                for (position, style) in styles.iter_mut().enumerate().take(index + width).skip(start) {
                    let is_marker = position < start + width || position >= index;
                    *style = if is_marker {
                        marker
                    } else {
                        style.add_modifier(modifier)
                    };
                }
                open = None;
            }
            None if after.is_some_and(|it| !it.is_whitespace()) && word_edge => open = Some(index),
            _ => {}
        }

        index += width;
    }
}
