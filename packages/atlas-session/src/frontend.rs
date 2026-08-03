//! Frontend identity normalization for the shared memory surface (ICM-A3).
//!
//! Frontend identities are normalized to the canonical set
//! `claude`, `codex`, `copilot`, `cli`, and `mcp` so CLI and MCP memory
//! visibility rules agree on what a "frontend" is. Unknown names are rejected
//! unless config explicitly allows custom frontends.

use atlas_core::{AtlasError, Result};

/// Canonical frontend identities recognized by the shared memory surface.
pub const KNOWN_FRONTENDS: &[&str] = &["claude", "codex", "copilot", "cli", "mcp"];

/// Normalizes a frontend identity to its canonical lowercase form.
///
/// Recognized aliases (case-insensitive): `claude`, `claude code`,
/// `claude-code`, `codex`, `copilot`, `github copilot`, `cli`, `mcp`.
/// When `allow_custom` is set, any non-empty name is accepted verbatim
/// (trimmed and lowercased); otherwise unknown names are rejected.
pub fn normalize_frontend(value: &str, allow_custom: bool) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Err(AtlasError::Other(
            "frontend identifier must not be empty".to_owned(),
        ));
    }
    let canonical = match value.as_str() {
        "claude" | "claude code" | "claude-code" => "claude",
        "codex" => "codex",
        "copilot" | "github copilot" => "copilot",
        "cli" => "cli",
        "mcp" => "mcp",
        other => {
            if allow_custom {
                other
            } else {
                return Err(AtlasError::Other(format!(
                    "unknown frontend: {other}; expected one of {}",
                    KNOWN_FRONTENDS.join(", ")
                )));
            }
        }
    };
    Ok(canonical.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_frontends_normalize_case_insensitively() {
        for (input, expected) in [
            ("claude", "claude"),
            ("Claude", "claude"),
            ("Claude Code", "claude"),
            ("claude-code", "claude"),
            ("codex", "codex"),
            ("Codex", "codex"),
            ("copilot", "copilot"),
            ("GitHub Copilot", "copilot"),
            ("cli", "cli"),
            ("CLI", "cli"),
            ("mcp", "mcp"),
            (" MCP ", "mcp"),
        ] {
            assert_eq!(
                normalize_frontend(input, false).unwrap(),
                expected,
                "normalize {input:?}"
            );
        }
    }

    #[test]
    fn unknown_frontends_rejected_unless_custom_allowed() {
        for value in ["zed", "gemini", "agent"] {
            let error = normalize_frontend(value, false).unwrap_err().to_string();
            assert!(
                error.contains("unknown frontend"),
                "reject {value:?}: {error}"
            );
            assert!(
                error.contains("claude"),
                "error must list known frontends: {error}"
            );
        }
        for value in ["", " "] {
            let error = normalize_frontend(value, false).unwrap_err().to_string();
            assert!(error.contains("must not be empty"), "got: {error}");
        }
        assert_eq!(normalize_frontend("zed", true).unwrap(), "zed");
        assert_eq!(normalize_frontend(" My Agent ", true).unwrap(), "my agent");
    }

    #[test]
    fn known_set_matches_exact_values() {
        assert_eq!(
            KNOWN_FRONTENDS,
            &["claude", "codex", "copilot", "cli", "mcp"]
        );
    }
}
