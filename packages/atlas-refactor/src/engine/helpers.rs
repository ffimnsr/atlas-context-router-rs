use super::*;

pub(super) fn parse_file_content_with_registry(
    parser_registry: &ParserRegistry,
    file_path: &str,
    content: &str,
    validation: &mut RefactorValidationResult,
) -> Option<(atlas_core::ParsedFile, Option<tree_sitter::Tree>)> {
    if !parser_registry.supports(file_path) {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    if content.is_empty() {
        validation.warnings.push(format!(
            "skipping parser revalidation for empty file `{file_path}`"
        ));
        return None;
    }

    parser_registry.parse(file_path, &hash, content.as_bytes(), None)
}

// ---------------------------------------------------------------------------
// Import detection helpers
// ---------------------------------------------------------------------------

/// Language family for import detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lang {
    Rust,
    Python,
    JsTs,
    Go,
    Other,
}

pub(super) fn detect_language(path: &str) -> Lang {
    if path.ends_with(".rs") {
        Lang::Rust
    } else if path.ends_with(".py") {
        Lang::Python
    } else if path.ends_with(".js")
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".jsx")
    {
        Lang::JsTs
    } else if path.ends_with(".go") {
        Lang::Go
    } else {
        Lang::Other
    }
}

pub(super) struct ImportInfo<'a> {
    pub(super) line_no: u32,
    pub(super) imported_names: Vec<String>,
    pub(super) raw_line: &'a str,
}

pub(super) fn normalize_import_block_layout(file_path: &str, content: &str) -> String {
    let lang = detect_language(file_path);
    if matches!(lang, Lang::Other) {
        return content.to_string();
    }

    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    let mut idx = 0usize;
    while idx < lines.len() {
        if !is_import_line(lines[idx].trim(), lang) {
            idx += 1;
            continue;
        }

        let start = idx;
        while idx < lines.len() && is_import_line(lines[idx].trim(), lang) {
            idx += 1;
        }

        if idx.saturating_sub(start) > 1 {
            lines[start..idx].sort_by(|a, b| a.trim().cmp(b.trim()));
        }

        while idx + 1 < lines.len()
            && lines[idx].trim().is_empty()
            && lines[idx + 1].trim().is_empty()
        {
            lines.remove(idx + 1);
        }
    }

    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub(super) fn is_import_line(line: &str, lang: Lang) -> bool {
    match lang {
        Lang::Rust => line.starts_with("use "),
        Lang::Python => line.starts_with("import ") || line.starts_with("from "),
        Lang::JsTs => line.starts_with("import "),
        Lang::Go => line.starts_with("import "),
        Lang::Other => false,
    }
}

pub(super) fn find_imports<'a>(lines: &[&'a str], lang: Lang) -> Vec<ImportInfo<'a>> {
    let mut imports = Vec::new();
    for (i, &line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = (i + 1) as u32;

        match lang {
            Lang::Rust => {
                if trimmed.starts_with("use ") {
                    let names = extract_rust_use_names(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::Python => {
                if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                    let names = extract_python_import_names(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::JsTs => {
                if trimmed.starts_with("import ") {
                    let names = extract_js_import_names(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::Go => {
                // Go multi-line imports handled elsewhere; single-line:
                if trimmed.starts_with("import ") {
                    let names = extract_go_import_name(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::Other => {}
        }
    }
    imports
}

/// Extract the leaf identifier(s) from a Rust `use` statement.
///
/// e.g. `use std::io::{Read, Write};` → `["Read", "Write"]`
/// e.g. `use foo::bar::Baz;` → `["Baz"]`
pub(super) fn extract_rust_use_names(line: &str) -> Vec<String> {
    // Strip `use ` prefix and trailing `;`.
    let body = line.trim_start_matches("use ").trim_end_matches(';').trim();

    if body.contains('{') {
        // Multi-import: extract identifiers inside braces.
        let start = body.find('{').unwrap();
        let end = body.find('}').unwrap_or(body.len());
        body[start + 1..end]
            .split(',')
            .map(|s| {
                // Handle `Name as Alias` → take Alias if present.
                let s = s.trim();
                if let Some(pos) = s.rfind(" as ") {
                    s[pos + 4..].trim().to_string()
                } else {
                    s.split("::").last().unwrap_or(s).to_string()
                }
            })
            .filter(|s| !s.is_empty() && s != "*")
            .collect()
    } else {
        // Single import: last segment.
        let raw = body.split("::").last().unwrap_or(body);
        let name = if let Some(pos) = raw.rfind(" as ") {
            raw[pos + 4..].trim().to_string()
        } else {
            raw.trim().to_string()
        };
        if name.is_empty() || name == "*" {
            vec![]
        } else {
            vec![name]
        }
    }
}

/// Extract imported name(s) from a Python import statement.
pub(super) fn extract_python_import_names(line: &str) -> Vec<String> {
    if let Some(rest) = line.strip_prefix("from ") {
        // `from module import Name, Other`
        if let Some(idx) = rest.find(" import ") {
            return rest[idx + 8..]
                .split(',')
                .map(|s| {
                    let s = s.trim();
                    if let Some(p) = s.find(" as ") {
                        s[p + 4..].trim().to_string()
                    } else {
                        s.to_string()
                    }
                })
                .filter(|s| !s.is_empty() && s != "*")
                .collect();
        }
    } else if let Some(rest) = line.strip_prefix("import ") {
        return rest
            .split(',')
            .map(|s| {
                let s = s.trim();
                if let Some(p) = s.find(" as ") {
                    s[p + 4..].trim().to_string()
                } else {
                    s.split('.').next().unwrap_or(s).to_string()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
    }
    vec![]
}

/// Extract imported names from a JS/TS import statement.
pub(super) fn extract_js_import_names(line: &str) -> Vec<String> {
    // `import { Foo, Bar } from '...'` or `import Foo from '...'`
    let body = line.trim_start_matches("import").trim();
    if body.starts_with('{') {
        if let Some(end) = body.find('}') {
            return body[1..end]
                .split(',')
                .map(|s| {
                    let s = s.trim();
                    if let Some(p) = s.find(" as ") {
                        s[p + 4..].trim().to_string()
                    } else {
                        s.to_string()
                    }
                })
                .filter(|s| !s.is_empty())
                .collect();
        }
    } else {
        // Default import: first word.
        let name: String = body
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && name != "type" {
            return vec![name];
        }
    }
    vec![]
}

/// Extract the package alias from a Go import line.
pub(super) fn extract_go_import_name(line: &str) -> Vec<String> {
    // `import "pkg/path"` or `import alias "pkg/path"`.
    let body = line.trim_start_matches("import").trim().trim_matches('"');
    if body.is_empty() {
        return vec![];
    }
    // Last path component without quotes.
    let name = body
        .trim_matches('"')
        .split('/')
        .next_back()
        .unwrap_or(body)
        .to_string();
    if name.is_empty() { vec![] } else { vec![name] }
}

/// Check if `line` contains `word` as a whole-word occurrence.
pub(super) fn contains_word(line: &str, word: &str) -> bool {
    let bytes = line.as_bytes();
    let wb = word.as_bytes();
    let mut pos = 0;
    while pos + wb.len() <= bytes.len() {
        if bytes[pos..].starts_with(wb) {
            let before_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let after = pos + wb.len();
            let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
            if before_ok && after_ok {
                return true;
            }
        }
        pos += 1;
    }
    false
}

pub(super) fn normalize_qn_kind_tokens(qname: &str) -> String {
    let Some(after_file) = qname.find("::") else {
        return qname.to_owned();
    };
    let (file_part, rest) = qname.split_at(after_file);
    let rest = &rest[2..];

    let (kind_token, symbol_rest) = if let Some(pos) = rest.find("::") {
        (&rest[..pos], &rest[pos..])
    } else {
        return qname.to_owned();
    };

    let kind_lower = kind_token.to_ascii_lowercase();
    let canonical_kind = match kind_lower.as_str() {
        "function" | "func" => "fn",
        "meth" => "method",
        "constant" => "const",
        other => other,
    };

    if canonical_kind == kind_token {
        return qname.to_owned();
    }
    format!("{file_part}::{canonical_kind}{symbol_rest}")
}

#[inline]
pub(super) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
