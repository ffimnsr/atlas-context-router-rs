use anyhow::{Context, Result};
use camino::Utf8Path;
use sha2::{Digest, Sha256};
use std::io::Read;

/// Compute the SHA-256 hash of a file's contents and return it as a lowercase hex string.
pub fn hash_file(path: &Utf8Path) -> Result<String> {
    let mut f = std::fs::File::open(path.as_std_path())
        .with_context(|| format!("open '{path}' for hashing"))?;

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];

    loop {
        let n = f.read(&mut buf).with_context(|| format!("read '{path}'"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let digest = hasher.finalize();
    Ok(format!("{digest:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn known_hash() {
        // SHA-256 of empty string
        let dir = tempfile::tempdir().unwrap();
        let p = camino::Utf8Path::from_path(dir.path())
            .unwrap()
            .join("empty.txt");
        std::fs::File::create(p.as_std_path()).unwrap();
        assert_eq!(
            hash_file(&p).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let p = camino::Utf8Path::from_path(dir.path())
            .unwrap()
            .join("data.txt");
        let mut f = std::fs::File::create(p.as_std_path()).unwrap();
        f.write_all(b"hello world").unwrap();
        drop(f);
        let h1 = hash_file(&p).unwrap();
        let h2 = hash_file(&p).unwrap();
        assert_eq!(h1, h2);
    }
}
