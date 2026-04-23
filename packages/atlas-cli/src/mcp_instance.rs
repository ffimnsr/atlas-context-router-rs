use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli_paths::canonicalize_cli_path;

const INSTANCE_DIR_NAME: &str = "mcp";
const LOCK_FILE_NAME: &str = "mcp.instance.lock";
const METADATA_FILE_NAME: &str = "mcp.instance.json";
#[cfg(not(unix))]
const SOCKET_FILE_NAME: &str = "mcp.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpInstance {
    pub(crate) instance_id: String,
    pub(crate) repo_root: String,
    pub(crate) db_path: String,
    pub(crate) atlas_dir: PathBuf,
    pub(crate) instance_dir: PathBuf,
    pub(crate) lock_path: PathBuf,
    pub(crate) metadata_path: PathBuf,
    pub(crate) socket_path: PathBuf,
}

impl McpInstance {
    pub(crate) fn for_repo_and_db(repo_root: &str, db_path: &str) -> Result<Self> {
        let repo_root = canonicalize_cli_path(repo_root)?;
        let db_path = canonicalize_cli_path(db_path)?;
        let atlas_dir = atlas_engine::paths::atlas_dir(&repo_root);
        let instance_id = instance_identity(&repo_root, &db_path);
        let instance_dir = atlas_dir.join(INSTANCE_DIR_NAME).join(&instance_id);
        let socket_path = socket_path_for_instance(&instance_id, &instance_dir);
        Ok(Self {
            instance_id,
            repo_root,
            db_path,
            lock_path: instance_dir.join(LOCK_FILE_NAME),
            metadata_path: instance_dir.join(METADATA_FILE_NAME),
            socket_path,
            instance_dir,
            atlas_dir,
        })
    }

    pub(crate) fn acquire_lock_blocking(&self) -> Result<McpInstanceLock> {
        let file = self.open_lock_file()?;
        file.lock_exclusive()
            .with_context(|| format!("cannot lock {}", self.lock_path.display()))?;
        Ok(McpInstanceLock { file })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn acquire_lock(&self) -> Result<McpInstanceLock> {
        let file = self.open_lock_file()?;
        file.try_lock_exclusive()
            .with_context(|| format!("cannot lock {}", self.lock_path.display()))?;
        Ok(McpInstanceLock { file })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn read_metadata(&self) -> Result<Option<McpInstanceMetadata>> {
        if !self.metadata_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&self.metadata_path)
            .with_context(|| format!("cannot read {}", self.metadata_path.display()))?;
        let metadata = serde_json::from_str(&content)
            .with_context(|| format!("cannot parse {}", self.metadata_path.display()))?;
        Ok(Some(metadata))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn write_metadata(&self, metadata: &McpInstanceMetadata) -> Result<()> {
        fs::create_dir_all(&self.instance_dir)
            .with_context(|| format!("cannot create {}", self.instance_dir.display()))?;
        let content =
            serde_json::to_string_pretty(metadata).context("cannot encode MCP metadata")?;
        fs::write(&self.metadata_path, format!("{content}\n"))
            .with_context(|| format!("cannot write {}", self.metadata_path.display()))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn clear_runtime_state(&self) -> Result<()> {
        remove_if_exists(&self.metadata_path)?;
        remove_if_exists(&self.socket_path)?;
        cleanup_empty_dirs(&self.instance_dir, &self.atlas_dir)?;
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inspect_metadata(&self) -> Result<McpInstanceStatus> {
        let metadata = match self.read_metadata() {
            Ok(Some(metadata)) => metadata,
            Ok(None) => return Ok(McpInstanceStatus::Missing),
            Err(_) => {
                return Ok(McpInstanceStatus::Stale(McpInstanceStale {
                    metadata: None,
                    reasons: vec![McpInstanceStaleReason::InvalidMetadata],
                }));
            }
        };

        let mut reasons = Vec::new();
        if metadata.repo_root != self.repo_root {
            reasons.push(McpInstanceStaleReason::RepoMismatch);
        }
        if metadata.db_path != self.db_path {
            reasons.push(McpInstanceStaleReason::DbMismatch);
        }
        if !socket_exists(Path::new(&metadata.socket_path)) {
            reasons.push(McpInstanceStaleReason::SocketMissing);
        }
        if !process_exists(metadata.pid) {
            reasons.push(McpInstanceStaleReason::ProcessMissing);
        }

        if reasons.is_empty() {
            Ok(McpInstanceStatus::Ready(metadata))
        } else {
            Ok(McpInstanceStatus::Stale(McpInstanceStale {
                metadata: Some(metadata),
                reasons,
            }))
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn default_metadata(
        &self,
        pid: u32,
        protocol_version: &str,
        started_at: &str,
    ) -> McpInstanceMetadata {
        McpInstanceMetadata {
            repo_root: self.repo_root.clone(),
            db_path: self.db_path.clone(),
            socket_path: self.socket_path.to_string_lossy().into_owned(),
            pid,
            protocol_version: protocol_version.to_owned(),
            started_at: started_at.to_owned(),
        }
    }

    fn open_lock_file(&self) -> Result<File> {
        fs::create_dir_all(&self.instance_dir)
            .with_context(|| format!("cannot create {}", self.instance_dir.display()))?;
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .with_context(|| format!("cannot open {}", self.lock_path.display()))
    }
}

#[derive(Debug)]
pub(crate) struct McpInstanceLock {
    file: File,
}

impl Drop for McpInstanceLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct McpInstanceMetadata {
    pub(crate) repo_root: String,
    pub(crate) db_path: String,
    pub(crate) socket_path: String,
    pub(crate) pid: u32,
    pub(crate) protocol_version: String,
    pub(crate) started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpInstanceStatus {
    Missing,
    Ready(McpInstanceMetadata),
    Stale(McpInstanceStale),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpInstanceStale {
    pub(crate) metadata: Option<McpInstanceMetadata>,
    pub(crate) reasons: Vec<McpInstanceStaleReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpInstanceStaleReason {
    InvalidMetadata,
    RepoMismatch,
    DbMismatch,
    SocketMissing,
    ProcessMissing,
}

fn socket_path_for_instance(instance_id: &str, _instance_dir: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        Path::new("/tmp").join(format!("atlas-mcp-{instance_id}.sock"))
    }

    #[cfg(not(unix))]
    {
        instance_dir.join(SOCKET_FILE_NAME)
    }
}

fn instance_identity(repo_root: &str, db_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.as_bytes());
    hasher.update([0]);
    hasher.update(db_path.as_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn process_exists(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        true
    }
}

fn socket_exists(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;

        fs::metadata(path)
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

fn cleanup_empty_dirs(instance_dir: &Path, atlas_dir: &Path) -> Result<()> {
    match fs::remove_dir(instance_dir) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::DirectoryNotEmpty
                    | std::io::ErrorKind::Other
            ) => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot remove {}", instance_dir.display()));
        }
    }

    let instances_dir = atlas_dir.join(INSTANCE_DIR_NAME);
    match fs::remove_dir(&instances_dir) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::DirectoryNotEmpty
                    | std::io::ErrorKind::Other
            ) =>
        {
            Ok(())
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot remove {}", instances_dir.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use std::os::unix::net::UnixListener;

    struct CwdGuard {
        previous: PathBuf,
    }

    impl CwdGuard {
        fn change_to(path: &Path) -> Self {
            let previous = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(path).expect("set cwd");
            Self { previous }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous).expect("restore cwd");
        }
    }

    #[test]
    fn instance_canonicalizes_relative_repo_and_db_paths() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join("subdir")).unwrap();

        let _guard = CwdGuard::change_to(temp.path());
        let instance =
            McpInstance::for_repo_and_db("repo/./subdir/..", "repo/.atlas/../.atlas/worldtree.db")
                .unwrap();
        let expected_repo = canonicalize_cli_path(repo.to_string_lossy().as_ref()).unwrap();
        let expected_db =
            canonicalize_cli_path(repo.join(".atlas/worldtree.db").to_string_lossy().as_ref())
                .unwrap();

        assert_eq!(instance.repo_root, expected_repo);
        assert_eq!(instance.db_path, expected_db);
        let instance_root = repo
            .join(".atlas")
            .join(INSTANCE_DIR_NAME)
            .join(&instance.instance_id);
        assert_eq!(instance.instance_dir, instance_root);
        assert_eq!(instance.lock_path, instance_root.join(LOCK_FILE_NAME));
        assert_eq!(
            instance.metadata_path,
            instance_root.join(METADATA_FILE_NAME)
        );
        #[cfg(unix)]
        assert_eq!(
            instance.socket_path,
            Path::new("/tmp").join(format!("atlas-mcp-{}.sock", instance.instance_id))
        );
        #[cfg(not(unix))]
        assert_eq!(instance.socket_path, instance_root.join(SOCKET_FILE_NAME));
    }

    #[test]
    fn instance_paths_differ_for_same_repo_with_different_db_paths() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".atlas")).unwrap();

        let default_instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        let alternate_instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/alternate.db").to_str().unwrap(),
        )
        .unwrap();

        assert_ne!(default_instance.instance_id, alternate_instance.instance_id);
        assert_ne!(
            default_instance.instance_dir,
            alternate_instance.instance_dir
        );
        assert_ne!(default_instance.socket_path, alternate_instance.socket_path);
    }

    #[test]
    fn metadata_round_trip_preserves_fields() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        let metadata = instance.default_metadata(42, "2024-11-05", "2026-04-23T00:00:00Z");

        instance.write_metadata(&metadata).unwrap();

        assert_eq!(instance.read_metadata().unwrap(), Some(metadata));
    }

    #[test]
    fn inspect_metadata_marks_missing_socket_stale() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        let metadata =
            instance.default_metadata(std::process::id(), "2024-11-05", "2026-04-23T00:00:00Z");
        instance.write_metadata(&metadata).unwrap();

        let status = instance.inspect_metadata().unwrap();
        assert_eq!(
            status,
            McpInstanceStatus::Stale(McpInstanceStale {
                metadata: Some(metadata),
                reasons: vec![McpInstanceStaleReason::SocketMissing],
            })
        );
    }

    #[test]
    fn inspect_metadata_marks_process_missing_stale() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        fs::create_dir_all(&instance.instance_dir).unwrap();
        let _listener = UnixListener::bind(&instance.socket_path).unwrap();
        let metadata = instance.default_metadata(u32::MAX, "2024-11-05", "2026-04-23T00:00:00Z");
        instance.write_metadata(&metadata).unwrap();

        let status = instance.inspect_metadata().unwrap();
        assert_eq!(
            status,
            McpInstanceStatus::Stale(McpInstanceStale {
                metadata: Some(metadata),
                reasons: vec![McpInstanceStaleReason::ProcessMissing],
            })
        );
    }

    #[test]
    fn inspect_metadata_marks_repo_and_db_mismatch_stale() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let other_repo = temp.path().join("other-repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&other_repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        let metadata = McpInstanceMetadata {
            repo_root: other_repo.to_string_lossy().into_owned(),
            db_path: other_repo
                .join(".atlas/other.db")
                .to_string_lossy()
                .into_owned(),
            socket_path: instance.socket_path.to_string_lossy().into_owned(),
            pid: std::process::id(),
            protocol_version: "2024-11-05".to_owned(),
            started_at: "2026-04-23T00:00:00Z".to_owned(),
        };
        fs::create_dir_all(&instance.instance_dir).unwrap();
        let _listener = UnixListener::bind(&instance.socket_path).unwrap();
        instance.write_metadata(&metadata).unwrap();

        let status = instance.inspect_metadata().unwrap();
        assert_eq!(
            status,
            McpInstanceStatus::Stale(McpInstanceStale {
                metadata: Some(metadata),
                reasons: vec![
                    McpInstanceStaleReason::RepoMismatch,
                    McpInstanceStaleReason::DbMismatch,
                ],
            })
        );
    }

    #[test]
    fn inspect_metadata_returns_ready_when_pid_and_socket_match() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        fs::create_dir_all(&instance.instance_dir).unwrap();
        let _listener = UnixListener::bind(&instance.socket_path).unwrap();
        let metadata =
            instance.default_metadata(std::process::id(), "2024-11-05", "2026-04-23T00:00:00Z");
        instance.write_metadata(&metadata).unwrap();

        let status = instance.inspect_metadata().unwrap();
        assert_eq!(status, McpInstanceStatus::Ready(metadata));
    }

    #[test]
    fn inspect_metadata_marks_invalid_json_stale() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".atlas")).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        fs::create_dir_all(&instance.instance_dir).unwrap();
        let mut file = File::create(&instance.metadata_path).unwrap();
        writeln!(file, "not json").unwrap();

        let status = instance.inspect_metadata().unwrap();
        assert_eq!(
            status,
            McpInstanceStatus::Stale(McpInstanceStale {
                metadata: None,
                reasons: vec![McpInstanceStaleReason::InvalidMetadata],
            })
        );
    }

    #[test]
    fn instance_lock_is_exclusive() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        let _first = instance.acquire_lock().unwrap();

        let second = instance.acquire_lock();
        assert!(
            second.is_err(),
            "second lock acquisition should fail while first is held"
        );
    }

    #[test]
    fn clear_runtime_state_removes_metadata_and_socket() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let instance = McpInstance::for_repo_and_db(
            repo.to_str().unwrap(),
            repo.join(".atlas/worldtree.db").to_str().unwrap(),
        )
        .unwrap();
        fs::create_dir_all(&instance.instance_dir).unwrap();
        fs::write(&instance.metadata_path, "{}").unwrap();
        let _listener = UnixListener::bind(&instance.socket_path).unwrap();

        instance.clear_runtime_state().unwrap();

        assert!(!instance.metadata_path.exists());
        assert!(!instance.socket_path.exists());
    }
}
