//! Structured logs for `writebook mcp`: one JSON object per line on stderr.
//!
//! They say what happened, not who did it or with what. No tokens, IP
//! addresses, tool arguments, book contents, or error messages, since a
//! message can quote a title, a URL or a search. Only events from this
//! module's own target are written, since rmcp's can quote a request.

use axum::http::Method;
use serde_json::Value;
use std::time::Duration;
use tracing::{Level, info, warn};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;

use super::tools;
use crate::api::{ApiError, TokenRejected};

const TARGET: &str = "writebook::mcp";

pub fn start() {
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(std::io::stderr)
        .with_filter(Targets::new().with_target(TARGET, Level::INFO));

    tracing_subscriber::registry().with(layer).init();
}

pub fn serving(instance: &str, transport: &str, address: Option<String>, read_only: bool) {
    info!(target: TARGET, event = "serving", instance, transport, address, read_only);
}

pub fn http_request(method: &Method, status: u16, duration: Duration) {
    info!(target: TARGET, event = "http_request", method = method.as_str(), status, duration_ms = duration.as_millis() as u64);
}

pub fn tool_call(tool: &str, outcome: &anyhow::Result<Value>, duration: Duration) {
    let tool = if tools::exists(tool) { tool } else { "unknown" };
    let duration_ms = duration.as_millis() as u64;

    match outcome {
        Ok(_) => info!(target: TARGET, event = "tool_call", tool, outcome = "ok", duration_ms),
        Err(error) => {
            let (reason, writebook_status) = reason(error);
            warn!(target: TARGET, event = "tool_call", tool, outcome = "error", reason, writebook_status, duration_ms);
        }
    }
}

fn reason(error: &anyhow::Error) -> (&'static str, Option<u16>) {
    if error.downcast_ref::<TokenRejected>().is_some() {
        ("token_rejected", Some(401))
    } else if let Some(error) = error.downcast_ref::<ApiError>() {
        ("writebook_error", Some(error.status))
    } else {
        ("failed", None)
    }
}
