#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod histogram;
mod labeled;
mod snapshot;

use std::collections::BTreeMap;
use std::sync::OnceLock;

pub use histogram::{
    Counter, FILE_COUNT_BUCKETS, Histogram, HistogramBucketSnapshot, HistogramSnapshot,
    LATENCY_MS_BUCKETS,
};
pub use labeled::{LabeledCounter, LabeledHistogram};
pub use snapshot::{McpToolCallCounts, MetricsSnapshot};

/// Process-wide metric families for Atlas build, query, parser, and MCP work.
///
/// Instantiate with [`Metrics::new`] for isolated tests; production code uses
/// the process-global registry through [`metrics`].
#[derive(Debug)]
pub struct Metrics {
    build_runs: LabeledCounter,
    build_failures: LabeledCounter,
    build_duration_ms: LabeledHistogram,
    build_parsed_files: LabeledHistogram,
    parser_parses: Counter,
    parser_tree_reuses: Counter,
    query_calls: LabeledCounter,
    query_duration_ms: LabeledHistogram,
    mcp_tool_calls_ok: LabeledCounter,
    mcp_tool_calls_error: LabeledCounter,
    mcp_tool_duration_ms: LabeledHistogram,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            build_runs: LabeledCounter::new(),
            build_failures: LabeledCounter::new(),
            build_duration_ms: LabeledHistogram::new(LATENCY_MS_BUCKETS),
            build_parsed_files: LabeledHistogram::new(FILE_COUNT_BUCKETS),
            parser_parses: Counter::new(),
            parser_tree_reuses: Counter::new(),
            query_calls: LabeledCounter::new(),
            query_duration_ms: LabeledHistogram::new(LATENCY_MS_BUCKETS),
            mcp_tool_calls_ok: LabeledCounter::new(),
            mcp_tool_calls_error: LabeledCounter::new(),
            mcp_tool_duration_ms: LabeledHistogram::new(LATENCY_MS_BUCKETS),
        }
    }

    /// Record one completed build or update run.
    ///
    /// `parsed_files` is `None` for failed runs so the per-run file-count
    /// histogram is not polluted with zeroes from runs that never parsed.
    pub fn record_build_run(
        &self,
        kind: &str,
        duration_ms: u64,
        parsed_files: Option<u64>,
        ok: bool,
    ) {
        self.build_runs.increment(kind);
        if !ok {
            self.build_failures.increment(kind);
        }
        self.build_duration_ms.observe(kind, duration_ms);
        if let Some(parsed_files) = parsed_files {
            self.build_parsed_files.observe(kind, parsed_files);
        }
    }

    /// Record one `ParserRegistry::parse` call, noting whether a cached tree
    /// was handed to tree-sitter for incremental reuse.
    pub fn record_parse_attempt(&self, reused_old_tree: bool) {
        self.parser_parses.increment();
        if reused_old_tree {
            self.parser_tree_reuses.increment();
        }
    }

    /// Record one query execution by execution mode.
    pub fn record_query(&self, mode: &str, duration_ms: u64) {
        self.query_calls.increment(mode);
        self.query_duration_ms.observe(mode, duration_ms);
    }

    /// Record one MCP tool call by tool name and outcome.
    pub fn record_mcp_tool_call(&self, tool: &str, duration_ms: u64, ok: bool) {
        if ok {
            self.mcp_tool_calls_ok.increment(tool);
        } else {
            self.mcp_tool_calls_error.increment(tool);
        }
        self.mcp_tool_duration_ms.observe(tool, duration_ms);
    }

    /// Build a serializable point-in-time snapshot of every family.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let parser_parses_total = self.parser_parses.value();
        let parser_tree_reuses_total = self.parser_tree_reuses.value();
        let parser_cache_reuse_ratio = if parser_parses_total == 0 {
            0.0
        } else {
            parser_tree_reuses_total as f64 / parser_parses_total as f64
        };

        let mut mcp_tool_calls: BTreeMap<String, McpToolCallCounts> = BTreeMap::new();
        for (tool, ok) in self.mcp_tool_calls_ok.snapshot() {
            mcp_tool_calls.entry(tool).or_default().ok = ok;
        }
        for (tool, error) in self.mcp_tool_calls_error.snapshot() {
            mcp_tool_calls.entry(tool).or_default().error = error;
        }

        MetricsSnapshot {
            build_runs: self.build_runs.snapshot(),
            build_failures: self.build_failures.snapshot(),
            build_duration_ms: self.build_duration_ms.snapshot(),
            build_parsed_files: self.build_parsed_files.snapshot(),
            parser_parses_total,
            parser_tree_reuses_total,
            parser_cache_reuse_ratio,
            query_calls: self.query_calls.snapshot(),
            query_duration_ms: self.query_duration_ms.snapshot(),
            mcp_tool_calls,
            mcp_tool_duration_ms: self.mcp_tool_duration_ms.snapshot(),
        }
    }
}

static GLOBAL: OnceLock<Metrics> = OnceLock::new();

/// Process-global metrics registry used by Atlas library boundaries.
///
/// The registry is append-only and lock-free on the recording path, so a
/// single `OnceLock` avoids threading a registry through every parse, query,
/// and tool-dispatch call site.
pub fn metrics() -> &'static Metrics {
    GLOBAL.get_or_init(Metrics::new)
}

/// Snapshot the process-global registry.
pub fn snapshot() -> MetricsSnapshot {
    metrics().snapshot()
}

/// Record a completed build/update run in the process-global registry.
pub fn record_build_run(kind: &str, duration_ms: u64, parsed_files: Option<u64>, ok: bool) {
    metrics().record_build_run(kind, duration_ms, parsed_files, ok);
}

/// Record a parser invocation in the process-global registry.
pub fn record_parse_attempt(reused_old_tree: bool) {
    metrics().record_parse_attempt(reused_old_tree);
}

/// Record a query execution in the process-global registry.
pub fn record_query(mode: &str, duration_ms: u64) {
    metrics().record_query(mode, duration_ms);
}

/// Record an MCP tool call in the process-global registry.
pub fn record_mcp_tool_call(tool: &str, duration_ms: u64, ok: bool) {
    metrics().record_mcp_tool_call(tool, duration_ms, ok);
}

#[cfg(test)]
mod tests;
