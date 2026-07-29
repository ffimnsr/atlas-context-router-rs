//! Notification emitters for progress and trace notifications.

use std::io::Write;

use anyhow::Result;

use super::jsonrpc::jsonrpc_notification;
use super::types::{ProgressEventKind, TraceLevel, TraceThreshold};

// ---------------------------------------------------------------------------
// Write helpers
// ---------------------------------------------------------------------------

pub(crate) fn write_response<W: Write>(writer: &mut W, response: &str) -> Result<()> {
    writeln!(writer, "{response}")?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn write_notification<W: Write>(writer: &mut W, notification: &str) -> Result<()> {
    writeln!(writer, "{notification}")?;
    writer.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Progress notification
// ---------------------------------------------------------------------------

pub(crate) fn emit_progress_notification<W: Write>(
    writer: &mut W,
    token: Option<&serde_json::Value>,
    kind: ProgressEventKind,
    message: &str,
) -> Result<()> {
    let Some(token) = token else {
        return Ok(());
    };
    let value = match kind {
        ProgressEventKind::Begin => serde_json::json!({
            "kind": "begin",
            "title": "Atlas MCP tool call",
            "message": message,
        }),
        ProgressEventKind::Report => serde_json::json!({
            "kind": "report",
            "message": message,
        }),
        ProgressEventKind::End => serde_json::json!({
            "kind": "end",
            "message": message,
        }),
    };
    write_notification(
        writer,
        &jsonrpc_notification(
            "$/progress",
            serde_json::json!({
                "token": token,
                "value": value,
            }),
        ),
    )
}

// ---------------------------------------------------------------------------
// Trace log
// ---------------------------------------------------------------------------

pub(crate) fn emit_trace_log<W: Write>(
    writer: &mut W,
    trace: TraceLevel,
    threshold: TraceThreshold,
    level: &str,
    message: String,
) -> Result<()> {
    let enabled = match (trace, threshold) {
        (TraceLevel::Off, _) => false,
        (TraceLevel::Messages, TraceThreshold::Messages) => true,
        (TraceLevel::Messages, TraceThreshold::Verbose) => false,
        (TraceLevel::Verbose, _) => true,
    };
    if !enabled {
        return Ok(());
    }

    write_notification(
        writer,
        &jsonrpc_notification(
            "$/logMessage",
            serde_json::json!({
                "level": level,
                "message": message,
            }),
        ),
    )
}
