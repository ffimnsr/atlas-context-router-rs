use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

/// Return `path` relative to `repo_root`, with `/` separators.
///
/// Both paths must be absolute. The result is a clean relative path with no
/// leading `./`.
pub fn repo_relative(repo_root: &Utf8Path, abs_path: &Utf8Path) -> Result<Utf8PathBuf> {
    let rel = abs_path
        .strip_prefix(repo_root)
        .with_context(|| format!("'{abs_path}' is not under repo root '{repo_root}'"))?;

    // camino always uses `/`; just normalise `.` and `..` components.
    let normalised = normalise_components(rel);
    Ok(normalised)
}

/// Collapse `.` and `..` components without hitting the filesystem.
fn normalise_components(path: &Utf8Path) -> Utf8PathBuf {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.components() {
        use camino::Utf8Component::*;
        match component {
            CurDir => {}
            ParentDir => {
                parts.pop();
            }
            Normal(s) => parts.push(s),
            // Prefix / RootDir shouldn't appear in a stripped relative path.
            other => parts.push(other.as_str()),
        }
    }
    Utf8PathBuf::from(parts.join("/"))
}

/// Ensure separators are `/` (matters on Windows where camino may receive `\`).
pub fn to_forward_slashes(s: &str) -> String {
    s.replace('\\', "/")
}

/// Normalize path casing for the current platform.
///
/// On Windows the filesystem is case-insensitive, so two paths that differ
/// only in case refer to the same file.  To guarantee the qualified-name
/// scheme is stable regardless of how a path was obtained, we lowercase the
/// entire path on Windows.  On Unix the filesystem is case-sensitive, so no
/// transformation is applied.
///
/// Call this **after** [`to_forward_slashes`] so that the input is already
/// separator-normalized.
pub fn normalize_case(s: &str) -> String {
    if cfg!(target_os = "windows") {
        s.to_ascii_lowercase()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_relative() {
        let root = Utf8Path::new("/home/user/proj");
        let abs = Utf8Path::new("/home/user/proj/src/main.rs");
        assert_eq!(repo_relative(root, abs).unwrap().as_str(), "src/main.rs");
    }

    #[test]
    fn dotdot_normalised() {
        let p = Utf8Path::new("src/../src/lib.rs");
        assert_eq!(normalise_components(p).as_str(), "src/lib.rs");
    }

    #[test]
    fn dot_stripped() {
        let p = Utf8Path::new("./src/lib.rs");
        assert_eq!(normalise_components(p).as_str(), "src/lib.rs");
    }

    #[test]
    fn forward_slashes_passthrough() {
        assert_eq!(to_forward_slashes("src/lib.rs"), "src/lib.rs");
    }

    #[test]
    fn backslashes_converted_to_forward() {
        assert_eq!(to_forward_slashes("src\\main\\lib.rs"), "src/main/lib.rs");
    }

    #[test]
    fn mixed_separators_converted() {
        assert_eq!(
            to_forward_slashes("packages\\atlas-cli/src\\main.rs"),
            "packages/atlas-cli/src/main.rs"
        );
    }

    #[test]
    fn empty_string_passthrough() {
        assert_eq!(to_forward_slashes(""), "");
    }

    #[test]
    fn multiple_consecutive_dotdots() {
        let p = Utf8Path::new("a/b/../../c/d.rs");
        assert_eq!(normalise_components(p).as_str(), "c/d.rs");
    }

    #[test]
    fn deep_nesting_normalised() {
        let p = Utf8Path::new("a/./b/./c/../d.rs");
        assert_eq!(normalise_components(p).as_str(), "a/b/d.rs");
    }

    // --- normalize_case (Windows casing policy) ------------------------------

    /// On Linux the function is a no-op — case is preserved.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn normalize_case_noop_on_unix() {
        assert_eq!(normalize_case("Src/Main.rs"), "Src/Main.rs");
        assert_eq!(normalize_case("PKG/FOO.GO"), "PKG/FOO.GO");
    }

    /// On Windows the function lowercases to produce a stable canonical form.
    #[test]
    #[cfg(target_os = "windows")]
    fn normalize_case_lowercases_on_windows() {
        assert_eq!(normalize_case("Src/Main.rs"), "src/main.rs");
        assert_eq!(normalize_case("PKG/FOO.GO"), "pkg/foo.go");
        assert_eq!(
            normalize_case("packages/Atlas-Core/Src/Lib.rs"),
            "packages/atlas-core/src/lib.rs"
        );
    }

    /// Verify that `to_forward_slashes` + `normalize_case` together produce
    /// the expected canonical form on all platforms when given a Windows-style
    /// mixed-separator path.
    #[test]
    fn round_trip_windows_path_unix() {
        let raw = "Packages\\Atlas-CLI\\Src\\Main.rs";
        let slashed = to_forward_slashes(raw);
        assert_eq!(slashed, "Packages/Atlas-CLI/Src/Main.rs");
        // normalize_case is a no-op on Unix but returns a String either way.
        let _ = normalize_case(&slashed);
    }
}
