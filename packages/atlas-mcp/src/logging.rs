use std::sync::{OnceLock, RwLock};

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
}

impl LogLevel {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "notice" => Ok(Self::Notice),
            "warning" | "warn" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            other => Err(anyhow!(
                "invalid logging level '{other}'; expected debug, info, notice, warning, or error"
            )),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Notice => "notice",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

fn shared_level() -> &'static RwLock<LogLevel> {
    static SHARED: OnceLock<RwLock<LogLevel>> = OnceLock::new();
    SHARED.get_or_init(|| RwLock::new(LogLevel::Info))
}

#[cfg(test)]
pub(crate) fn current_level() -> LogLevel {
    *shared_level()
        .read()
        .expect("shared log level lock poisoned")
}

pub(crate) fn set_level(level: LogLevel) {
    *shared_level()
        .write()
        .expect("shared log level lock poisoned") = level;
}

pub(crate) fn parse_set_level_params(params: Option<&Value>) -> Result<LogLevel> {
    let raw = params
        .and_then(|value| value.get("level").or_else(|| value.get("value")))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing logging level"))?;
    LogLevel::parse(raw)
}

pub(crate) fn should_emit(subscribed_level: Option<LogLevel>, level: LogLevel) -> bool {
    subscribed_level.is_some_and(|threshold| level >= threshold)
}

pub(crate) fn log_notification(level: LogLevel, logger: &str, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": {
            "level": level.as_str(),
            "logger": logger,
            "data": {
                "message": message.into(),
            }
        }
    })
}

pub(crate) fn write_stdio_log(level: LogLevel, message: &str) {
    eprintln!("atlas-mcp[{}]: {message}", level.as_str());
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, current_level, parse_set_level_params, set_level, should_emit};
    use serde_json::json;

    #[test]
    fn parse_set_level_accepts_level_and_value_fields() {
        assert_eq!(
            parse_set_level_params(Some(&json!({"level": "warning"}))).expect("level"),
            LogLevel::Warning
        );
        assert_eq!(
            parse_set_level_params(Some(&json!({"value": "debug"}))).expect("value"),
            LogLevel::Debug
        );
    }

    #[test]
    fn should_emit_respects_threshold() {
        assert!(!should_emit(None, LogLevel::Error));
        assert!(!should_emit(Some(LogLevel::Warning), LogLevel::Info));
        assert!(should_emit(Some(LogLevel::Warning), LogLevel::Error));
    }

    #[test]
    fn shared_level_updates() {
        set_level(LogLevel::Notice);
        assert_eq!(current_level(), LogLevel::Notice);
        set_level(LogLevel::Info);
    }
}
