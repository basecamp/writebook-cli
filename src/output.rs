//! How commands print: a table (or a document) for people by default, or
//! Markdown, or JSON when asked for.
//!
//! JSON comes in an envelope, like hey's: the data, plus breadcrumbs that
//! suggest what to run next. --jq filters that envelope with a built-in jq,
//! and prints string results raw, as `jq -r` does.

use anyhow::{Result, anyhow, bail};
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
use jaq_json::Val;
use serde::Serialize;

pub enum Format {
    Table,
    Markdown,
    Json { jq: Option<String> },
}

impl Format {
    pub fn from_flags(json: bool, markdown: bool, jq: Option<String>) -> Result<Format> {
        if let Some(filter) = &jq {
            check_filter(filter)?;
        }

        if json || jq.is_some() {
            Ok(Format::Json { jq })
        } else if markdown {
            Ok(Format::Markdown)
        } else {
            Ok(Format::Table)
        }
    }
}

/// A command to run next, given in the JSON envelope.
#[derive(Serialize)]
pub struct Breadcrumb {
    pub action: &'static str,
    pub command: String,
    pub description: &'static str,
}

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    ok: bool,
    data: &'a T,
    #[serde(skip_serializing_if = "<[Breadcrumb]>::is_empty")]
    breadcrumbs: &'a [Breadcrumb],
}

/// Print the data in its envelope, or what the jq filter makes of it.
pub fn print_json<T: Serialize>(data: &T, breadcrumbs: &[Breadcrumb], jq: Option<&str>) -> Result<()> {
    let envelope = Envelope { ok: true, data, breadcrumbs };

    match jq {
        Some(filter) => print_filtered(&serde_json::to_string(&envelope)?, filter),
        None => {
            println!("{}", serde_json::to_string_pretty(&envelope)?);
            Ok(())
        }
    }
}

/// Columns padded to line up, two spaces apart. The first row is the header.
pub fn print_table<const COLUMNS: usize>(rows: &[[String; COLUMNS]]) {
    let rows: Vec<Vec<String>> = rows.iter().map(|row| row.iter().map(|cell| terminal_safe(cell)).collect()).collect();
    let widths: Vec<usize> = (0..COLUMNS).map(|column| rows.iter().map(|row| row[column].chars().count()).max().unwrap_or(0)).collect();

    for row in rows {
        let cells: Vec<String> = row.iter().zip(&widths).map(|(cell, width)| format!("{cell:width$}")).collect();
        println!("{}", cells.join("  ").trim_end());
    }
}

/// Print text that came from a server, as a line.
pub fn print_line(text: &str) {
    println!("{}", terminal_safe(text));
}

/// Print text that came from a server, as it is.
pub fn print_text(text: &str) {
    print!("{}", terminal_safe(text));
}

/// Text without control characters, other than line breaks and tabs. A
/// server could otherwise send escape sequences that rewrite what the
/// terminal shows, or worse.
pub fn terminal_safe(text: &str) -> String {
    text.chars().filter(|it| !it.is_control() || matches!(it, '\n' | '\t')).collect()
}

fn check_filter(filter: &str) -> Result<()> {
    let arena = Arena::default();
    compile(&arena, filter).map(|_| ())
}

fn print_filtered(json: &str, filter: &str) -> Result<()> {
    let arena = Arena::default();
    let compiled = compile(&arena, filter)?;
    let input = jaq_json::read::parse_single(json.as_bytes()).map_err(|error| anyhow!("could not read the JSON for --jq: {error}"))?;
    let ctx = Ctx::<data::JustLut<Val>>::new(&compiled.lut, Vars::new([]));

    for result in compiled.id.run((ctx, input)).map(unwrap_valr) {
        match result.map_err(|error| anyhow!("--jq failed: {error}"))? {
            Val::TStr(text) => println!("{}", String::from_utf8_lossy(&text)),
            value => println!("{}", pretty(&value.to_string())),
        }
    }

    Ok(())
}

fn compile<'a>(arena: &'a Arena, filter: &'a str) -> Result<jaq_core::Filter<data::JustLut<Val>>> {
    let defs = jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs());
    let funs = jaq_core::funs().chain(jaq_std::funs()).chain(jaq_json::funs());

    let Ok(modules) = Loader::new(defs).load(arena, File { code: filter, path: () }) else {
        bail!("--jq '{filter}' is not a valid jq expression");
    };

    match Compiler::default().with_funs(funs).compile(modules) {
        Ok(compiled) => Ok(compiled),
        Err(_) => bail!("--jq '{filter}' uses something jq doesn't know, like an undefined filter or variable"),
    }
}

/// jaq writes compact JSON; print it indented, like --json.
fn pretty(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| json.to_string()),
        Err(_) => json.to_string(),
    }
}
