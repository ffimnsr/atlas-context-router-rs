//! Frontmatter value model and canonical scalar encoding.

/// A frontmatter value.
///
/// The model is intentionally small: scalars, ordered lists, and flat ordered
/// maps (used by link entries and `metadata`). Nested containers are rejected
/// by the serializer so canonical output stays predictable.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// YAML `null`.
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    Int(i64),
    /// Finite float. Serialization keeps a `.0` suffix for whole numbers.
    Float(f64),
    /// String scalar.
    Str(String),
    /// Ordered sequence.
    List(Vec<Value>),
    /// Flat ordered map with scalar values only.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// Convenience constructor for string values.
    pub fn str(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }

    /// Convenience constructor for list values.
    pub fn list(items: impl IntoIterator<Item = Value>) -> Self {
        Self::List(items.into_iter().collect())
    }

    /// Borrow as string when this is [`Value::Str`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow as bool when this is [`Value::Bool`].
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Borrow as integer when this is [`Value::Int`].
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    /// Borrow as slice when this is [`Value::List`].
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    /// Borrow as map entries when this is [`Value::Map`].
    pub fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

/// Render a scalar canonically.
pub(crate) fn format_scalar(value: &Value) -> Result<String, String> {
    Ok(match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => format_float(*value)?,
        Value::Str(value) => {
            if is_plain_safe(value) {
                value.clone()
            } else {
                quote_string(value)
            }
        }
        Value::List(_) | Value::Map(_) => return Err("expected a scalar value".to_string()),
    })
}

fn format_float(value: f64) -> Result<String, String> {
    if !value.is_finite() {
        return Err("non-finite floats are not supported".to_string());
    }
    let mut out = format!("{value}");
    if !out.contains(['.', 'e', 'E']) {
        out.push_str(".0");
    }
    Ok(out)
}

/// `true` when a string can be emitted without quotes and parse back as a
/// string (not as null, bool, or number).
///
/// Interior spaces and colons are allowed so human titles, timestamps, and
/// qualified names stay readable in diffs. Characters with structural meaning
/// in flow maps (`,`, `{`, `}`, `#`, quotes) force quoting.
pub(crate) fn is_plain_safe(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.trim() != value {
        return false;
    }
    if matches!(value, "null" | "true" | "false") {
        return false;
    }
    if value.parse::<i64>().is_ok() || looks_like_float(value) {
        return false;
    }
    if !value
        .chars()
        .next()
        .map(|ch| ch.is_ascii_alphanumeric() || ch == '/')
        .unwrap_or(false)
    {
        return false;
    }
    value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | '/' | '+' | '-' | ':' | ' ')
    })
}

fn looks_like_float(value: &str) -> bool {
    value.contains(['.', 'e', 'E']) && value.parse::<f64>().is_ok()
}

/// Render a string as a double-quoted scalar with YAML-compatible escapes.
pub(crate) fn quote_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Parse a scalar token (already trimmed for unquoted values).
pub(crate) fn parse_scalar(raw: &str) -> Result<Value, String> {
    let value = raw.trim();
    if value.starts_with('"') {
        return unquote_string(value).map(Value::Str);
    }
    match value {
        "null" | "~" => Ok(Value::Null),
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        _ => {
            if let Ok(parsed) = value.parse::<i64>() {
                return Ok(Value::Int(parsed));
            }
            if looks_like_float(value)
                && let Ok(parsed) = value.parse::<f64>()
            {
                return Ok(Value::Float(parsed));
            }
            Ok(Value::Str(value.to_string()))
        }
    }
}

/// Reverse [`quote_string`].
pub(crate) fn unquote_string(value: &str) -> Result<String, String> {
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err("unterminated quoted scalar".to_string());
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let mut hex = String::with_capacity(4);
                for _ in 0..4 {
                    match chars.next() {
                        Some(digit) => hex.push(digit),
                        None => return Err("truncated \\u escape".to_string()),
                    }
                }
                let code = u32::from_str_radix(&hex, 16)
                    .map_err(|_| format!("invalid \\u escape: {hex}"))?;
                let decoded =
                    char::from_u32(code).ok_or_else(|| format!("invalid unicode scalar: {hex}"))?;
                out.push(decoded);
            }
            Some(other) => return Err(format!("unsupported escape: \\{other}")),
            None => return Err("trailing escape".to_string()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_safe_rejects_reserved_and_numeric() {
        assert!(!is_plain_safe(""));
        assert!(!is_plain_safe("true"));
        assert!(!is_plain_safe("null"));
        assert!(!is_plain_safe("42"));
        assert!(!is_plain_safe("1.5"));
        assert!(!is_plain_safe("-leading"));
        assert!(!is_plain_safe(" leading"));
        assert!(!is_plain_safe("trailing "));
        assert!(!is_plain_safe("has,comma"));
        assert!(!is_plain_safe("{brace}"));
        assert!(!is_plain_safe("hash # tag"));
        assert!(is_plain_safe("ci/cd"));
        assert!(is_plain_safe("atlas-store-markdown"));
        assert!(is_plain_safe("user@example.com"));
        assert!(is_plain_safe("Storage direction markdown truth"));
        assert!(is_plain_safe("2026-11-19T13:26:36Z"));
        assert!(is_plain_safe(
            "packages/atlas-session/src/store/memory.rs::fn::store_memory"
        ));
        assert!(is_plain_safe("/home/user/repo"));
    }

    #[test]
    fn string_quoting_round_trips() {
        for raw in [
            "plain",
            "with space",
            "with: colon",
            "quote \" inside",
            "newline\nhere",
            "tab\there",
            "back\\slash",
            "emoji \u{1f600}",
            "cjk \u{8a18}\u{61b6}",
            "",
        ] {
            let encoded = quote_string(raw);
            let decoded = unquote_string(&encoded).expect("decode");
            assert_eq!(decoded, raw, "encoded={encoded}");
        }
    }

    #[test]
    fn scalar_types_round_trip() {
        assert_eq!(parse_scalar("null").unwrap(), Value::Null);
        assert_eq!(parse_scalar("true").unwrap(), Value::Bool(true));
        assert_eq!(parse_scalar("42").unwrap(), Value::Int(42));
        assert_eq!(parse_scalar("1.0").unwrap(), Value::Float(1.0));
        assert_eq!(parse_scalar("\"true\"").unwrap(), Value::Str("true".into()));
        assert_eq!(format_scalar(&Value::Float(1.0)).unwrap(), "1.0");
        assert_eq!(format_scalar(&Value::Float(0.8)).unwrap(), "0.8");
        assert_eq!(
            format_scalar(&Value::Float(1e20)).unwrap(),
            "100000000000000000000.0"
        );
    }

    #[test]
    fn non_finite_floats_rejected() {
        assert!(format_scalar(&Value::Float(f64::NAN)).is_err());
        assert!(format_scalar(&Value::Float(f64::INFINITY)).is_err());
    }
}
