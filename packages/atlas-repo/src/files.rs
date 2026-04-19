use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use std::process::Command;

/// Default maximum file size in bytes (10 MiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Bytes examined for binary detection (8 KiB).
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Collect all git-tracked files under `repo_root`, filtering out:
/// - files larger than `max_bytes` (defaults to [`DEFAULT_MAX_FILE_BYTES`])
/// - binary files (null byte in first 8 KiB)
///
/// Returned paths are repo-relative, forward-slash separated.
pub fn collect_files(repo_root: &Utf8Path, max_bytes: Option<u64>) -> Result<Vec<Utf8PathBuf>> {
    let threshold = max_bytes.unwrap_or(DEFAULT_MAX_FILE_BYTES);
    let raw = git_ls_files(repo_root)?;
    let mut results = Vec::with_capacity(raw.len());

    for rel_path in raw {
        let abs = repo_root.join(&rel_path);
        match check_file(&abs, threshold) {
            Ok(true) => results.push(rel_path),
            Ok(false) => {
                tracing::debug!("skipping '{}': too large or binary", rel_path);
            }
            Err(e) => {
                tracing::warn!("skipping '{}': {}", rel_path, e);
            }
        }
    }

    Ok(results)
}

/// Run `git ls-files` and return repo-relative paths.
fn git_ls_files(repo_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
        .current_dir(repo_root)
        .output()
        .context("failed to run git ls-files")?;

    anyhow::ensure!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Null-separated output from `-z` flag.
    let stdout = std::str::from_utf8(&output.stdout)
        .context("git ls-files output is not valid UTF-8")?;

    let paths = stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(Utf8PathBuf::from)
        .collect();

    Ok(paths)
}

/// Return `true` if the file should be included (exists, small enough, not binary).
fn check_file(abs: &Utf8Path, max_bytes: u64) -> Result<bool> {
    let meta = abs
        .as_std_path()
        .metadata()
        .with_context(|| format!("metadata for '{abs}'"))?;

    if !meta.is_file() {
        return Ok(false);
    }
    if meta.len() > max_bytes {
        return Ok(false);
    }
    if is_binary(abs)? {
        return Ok(false);
    }
    Ok(true)
}

/// Sniff first `BINARY_SNIFF_BYTES` for null bytes.
fn is_binary(path: &Utf8Path) -> Result<bool> {
    use std::io::Read;
    let mut f = std::fs::File::open(path.as_std_path())
        .with_context(|| format!("open '{path}'"))?;
    let mut buf = vec![0u8; BINARY_SNIFF_BYTES];
    let n = f.read(&mut buf).with_context(|| format!("read '{path}'"))?;
    Ok(buf[..n].contains(&0u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_rust_sources_in_workspace() {
        let root = Utf8Path::new(
            // Go up 2 levels from packages/atlas-repo to the workspace root.
            concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),
        );
        // Canonicalize to resolve any `..` components.
        let abs = root
            .canonicalize_utf8()
            .expect("canonicalize workspace root");

        let files = collect_files(&abs, None).expect("collect_files");
        assert!(
            !files.is_empty(),
            "should find at least one tracked file"
        );
        assert!(
            files.iter().any(|p| p.extension() == Some("rs")),
            "should include .rs files"
        );
    }

    #[test]
    fn binary_detection_rejects_null_bytes() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let p = Utf8Path::from_path(dir.path()).unwrap().join("bin.dat");
        let mut f = std::fs::File::create(p.as_std_path()).unwrap();
        f.write_all(&[0x00, 0x01, 0x02]).unwrap();
        assert!(is_binary(&p).unwrap());
    }
}
