use serde::Serialize;

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

pub(crate) fn should_emit(requested_level: Option<LogLevel>, level: LogLevel) -> bool {
    requested_level.is_some_and(|threshold| level >= threshold)
}

pub(crate) fn write_stdio_log(level: LogLevel, message: &str) {
    eprintln!("atlas-mcp[{}]: {message}", level.as_str());
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, should_emit};

    #[test]
    fn should_emit_respects_threshold() {
        assert!(!should_emit(None, LogLevel::Error));
        assert!(!should_emit(Some(LogLevel::Warning), LogLevel::Info));
        assert!(should_emit(Some(LogLevel::Warning), LogLevel::Error));
    }
}
