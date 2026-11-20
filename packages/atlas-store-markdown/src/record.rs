//! Whole-record read/write with bounded frontmatter loading.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::error::MarkdownError;
use crate::frontmatter::{Frontmatter, parse_document};

/// Default cap for the frontmatter region when reading only L0 data.
pub const DEFAULT_MAX_FRONTMATTER_BYTES: usize = 64 * 1024;

/// A markdown record: ordered frontmatter plus verbatim body.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkdownRecord {
    /// Frontmatter fields in canonical order.
    pub frontmatter: Frontmatter,
    /// Body bytes interpreted as UTF-8 and preserved verbatim.
    pub body: String,
}

impl MarkdownRecord {
    /// Build a record from parts.
    pub fn new(frontmatter: Frontmatter, body: impl Into<String>) -> Self {
        Self {
            frontmatter,
            body: body.into(),
        }
    }

    /// Parse a full document.
    pub fn parse(bytes: &[u8], path: &Path) -> Result<Self, MarkdownError> {
        let (frontmatter, body_offset) = parse_document(bytes, path)?;
        let body = std::str::from_utf8(&bytes[body_offset..])
            .map_err(|_| MarkdownError::InvalidUtf8 {
                path: path.to_path_buf(),
            })?
            .to_string();
        Ok(Self { frontmatter, body })
    }

    /// Serialize canonically: frontmatter, closing fence, then body verbatim.
    pub fn to_bytes(&self, path: &Path) -> Result<Vec<u8>, MarkdownError> {
        let mut bytes = self.frontmatter.to_bytes(path)?;
        bytes.extend_from_slice(self.body.as_bytes());
        Ok(bytes)
    }
}

/// Read a full record from disk.
pub fn read_record(path: &Path) -> Result<MarkdownRecord, MarkdownError> {
    let bytes = std::fs::read(path).map_err(|error| MarkdownError::io(path, error))?;
    MarkdownRecord::parse(&bytes, path)
}

/// Read only the frontmatter (L0), bounded by [`DEFAULT_MAX_FRONTMATTER_BYTES`].
pub fn read_frontmatter(path: &Path) -> Result<Frontmatter, MarkdownError> {
    read_frontmatter_bounded(path, DEFAULT_MAX_FRONTMATTER_BYTES)
}

/// Read only the frontmatter with an explicit byte cap.
///
/// The body is never loaded, so this stays cheap for large artifacts and long
/// session ledgers.
pub fn read_frontmatter_bounded(
    path: &Path,
    max_bytes: usize,
) -> Result<Frontmatter, MarkdownError> {
    let file = File::open(path).map_err(|error| MarkdownError::io(path, error))?;
    let mut reader = BufReader::new(file);
    let mut head = Vec::new();
    let mut line = Vec::new();
    let mut seen_open = false;
    let mut line_number = 0usize;

    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| MarkdownError::io(path, error))?;
        if read == 0 {
            return Err(MarkdownError::MissingFence {
                path: path.to_path_buf(),
            });
        }
        line_number += 1;
        head.extend_from_slice(&line);
        if head.len() > max_bytes {
            return Err(MarkdownError::FrontmatterTooLarge {
                path: path.to_path_buf(),
                limit: max_bytes,
            });
        }
        let text = std::str::from_utf8(&line).map_err(|_| MarkdownError::InvalidFrontmatter {
            path: path.to_path_buf(),
            line: line_number,
            message: "frontmatter line is not valid utf-8".to_string(),
        })?;
        let trimmed = text.trim_end_matches(['\r', '\n']);
        if !seen_open {
            if trimmed != "---" {
                return Err(MarkdownError::MissingFence {
                    path: path.to_path_buf(),
                });
            }
            seen_open = true;
            continue;
        }
        if trimmed == "---" {
            let (frontmatter, _) = parse_document(&head, path)?;
            return Ok(frontmatter);
        }
    }
}

/// Read at most `max_body_bytes` of the body after parsing frontmatter.
///
/// Used for bounded previews; the returned body may be shorter than the file.
pub fn read_record_bounded(
    path: &Path,
    max_body_bytes: usize,
) -> Result<MarkdownRecord, MarkdownError> {
    let mut file = File::open(path).map_err(|error| MarkdownError::io(path, error))?;
    let mut reader = BufReader::new(&mut file);
    let mut head = Vec::new();
    let mut line = Vec::new();
    let mut seen_open = false;
    let mut line_number = 0usize;

    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| MarkdownError::io(path, error))?;
        if read == 0 {
            return Err(MarkdownError::MissingFence {
                path: path.to_path_buf(),
            });
        }
        line_number += 1;
        head.extend_from_slice(&line);
        if head.len() > DEFAULT_MAX_FRONTMATTER_BYTES {
            return Err(MarkdownError::FrontmatterTooLarge {
                path: path.to_path_buf(),
                limit: DEFAULT_MAX_FRONTMATTER_BYTES,
            });
        }
        let text = std::str::from_utf8(&line).map_err(|_| MarkdownError::InvalidFrontmatter {
            path: path.to_path_buf(),
            line: line_number,
            message: "frontmatter line is not valid utf-8".to_string(),
        })?;
        let trimmed = text.trim_end_matches(['\r', '\n']);
        if !seen_open {
            if trimmed != "---" {
                return Err(MarkdownError::MissingFence {
                    path: path.to_path_buf(),
                });
            }
            seen_open = true;
            continue;
        }
        if trimmed == "---" {
            break;
        }
    }

    let (frontmatter, _) = parse_document(&head, path)?;
    let mut body_bytes = Vec::new();
    reader
        .take(max_body_bytes as u64)
        .read_to_end(&mut body_bytes)
        .map_err(|error| MarkdownError::io(path, error))?;
    let body = String::from_utf8_lossy(&body_bytes).into_owned();
    Ok(MarkdownRecord { frontmatter, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_file(contents: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp");
        file.write_all(contents.as_bytes()).expect("write");
        file
    }

    #[test]
    fn body_is_verbatim_even_when_it_looks_like_frontmatter() {
        let input = "---\ntitle: x\n---\n---\nkey: value\n---\n";
        let record = MarkdownRecord::parse(input.as_bytes(), Path::new("r.md")).expect("parse");
        assert_eq!(record.body, "---\nkey: value\n---\n");
        let bytes = record.to_bytes(Path::new("r.md")).expect("serialize");
        assert_eq!(bytes, input.as_bytes());
    }

    #[test]
    fn crlf_and_bom_body_round_trip() {
        let input = "---\ntitle: x\n---\n\u{feff}line one\r\nline two\r\n";
        let record = MarkdownRecord::parse(input.as_bytes(), Path::new("r.md")).expect("parse");
        let bytes = record.to_bytes(Path::new("r.md")).expect("serialize");
        assert_eq!(bytes, input.as_bytes());
    }

    #[test]
    fn empty_body_round_trips() {
        let input = "---\ntitle: x\n---\n";
        let record = MarkdownRecord::parse(input.as_bytes(), Path::new("r.md")).expect("parse");
        assert_eq!(record.body, "");
        assert_eq!(
            record.to_bytes(Path::new("r.md")).expect("serialize"),
            input.as_bytes()
        );
    }

    #[test]
    fn long_single_line_body_round_trips() {
        let long = "x".repeat(10_000);
        let input = format!("---\ntitle: x\n---\n{long}");
        let record = MarkdownRecord::parse(input.as_bytes(), Path::new("r.md")).expect("parse");
        assert_eq!(
            record.to_bytes(Path::new("r.md")).expect("serialize"),
            input.as_bytes()
        );
    }

    #[test]
    fn bounded_read_ignores_body_and_caps_frontmatter() {
        let input = "---\ntitle: x\n---\nthis body is huge but never loaded";
        let file = write_file(input);
        let frontmatter = read_frontmatter_bounded(file.path(), 128).expect("bounded read");
        assert_eq!(frontmatter.get_str("title"), Some("x"));

        let error = read_frontmatter_bounded(file.path(), 4).expect_err("must cap");
        assert!(matches!(error, MarkdownError::FrontmatterTooLarge { .. }));
    }

    #[test]
    fn bounded_record_read_truncates_body() {
        let file = write_file("---\ntitle: x\n---\n0123456789");
        let record = read_record_bounded(file.path(), 4).expect("bounded record");
        assert_eq!(record.body, "0123");
        let _ = Value::Null;
    }

    #[test]
    fn missing_file_reports_not_found() {
        let error = read_frontmatter(Path::new("/definitely/not/here.md")).expect_err("missing");
        match error {
            MarkdownError::Io { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
