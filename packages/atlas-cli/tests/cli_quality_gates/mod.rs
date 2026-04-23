use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use atlas_core::{EdgeKind, NodeKind};
use atlas_store_sqlite::Store;
use serde_json::{json, Value};
use tempfile::TempDir;

const GIT_LOCAL_ENV_VARS: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_KEY_0",
    "GIT_CONFIG_VALUE_0",
    "GIT_DIR",
    "GIT_GRAFT_FILE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_NAMESPACE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE",
];

mod context;
mod core;
mod resolution;
mod shell;
mod watch;
mod workspace;

fn sanitized_command(program: &str) -> Command {
    let mut command = Command::new(program);
    for env_var in GIT_LOCAL_ENV_VARS {
        command.env_remove(env_var);
    }
    command
}

fn run_shell_with_input(repo_path: &Path, input: &[u8]) -> String {
    let mut child = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["shell"])
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas shell: {err}"));

    child
        .stdin
        .as_mut()
        .expect("shell stdin")
        .write_all(input)
        .expect("write shell input");

    let output = child.wait_with_output().expect("wait for atlas shell");
    assert!(
        output.status.success(),
        "atlas shell failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn setup_fixture_repo() -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    copy_dir_all(&fixture_repo_root(), temp_dir.path());
    init_git_repo(temp_dir.path());
    temp_dir
}

fn setup_repo(files: &[(&str, &str)]) -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    for (relative_path, content) in files {
        let path = temp_dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create test dir");
        }
        fs::write(path, content).expect("write test file");
    }
    init_git_repo(temp_dir.path());
    temp_dir
}

fn setup_repo_with_submodule(
    files: &[(&str, &str)],
    submodule_path: &str,
    submodule_files: &[(&str, &str)],
) -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    for (relative_path, content) in files {
        let path = temp_dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create test dir");
        }
        fs::write(path, content).expect("write test file");
    }
    init_git_repo(temp_dir.path());
    let _submodule = init_committed_submodule(temp_dir.path(), submodule_path, submodule_files);
    temp_dir
}

fn init_committed_submodule(
    parent: &Path,
    path: &str,
    files: &[(&str, &str)],
) -> TempDir {
    let submodule_dir = tempfile::tempdir().expect("submodule temp dir");
    for (relative_path, content) in files {
        let file_path = submodule_dir.path().join(relative_path);
        if let Some(parent_dir) = file_path.parent() {
            fs::create_dir_all(parent_dir).expect("create submodule dir");
        }
        fs::write(file_path, content).expect("write submodule file");
    }

    init_git_repo(submodule_dir.path());
    run_command(
        parent,
        "git",
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule_dir.path().to_str().expect("submodule path"),
            path,
        ],
    );
    run_command(parent, "git", &["commit", "--quiet", "-am", "add submodule"]);

    submodule_dir
}

struct DetachedWorktree {
    temp_dir: TempDir,
    source_repo: PathBuf,
    path: PathBuf,
}

impl DetachedWorktree {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DetachedWorktree {
    fn drop(&mut self) {
        let _ = sanitized_command("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .current_dir(&self.source_repo)
            .output();
        let _ = &self.temp_dir;
    }
}

fn setup_current_repo_detached_worktree() -> DetachedWorktree {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("repo-worktree");
    let source_repo = current_repo_root();

    run_command(
        &source_repo,
        "git",
        &[
            "worktree",
            "add",
            "--detach",
            path.to_str().expect("worktree path"),
            "HEAD",
        ],
    );

    DetachedWorktree {
        temp_dir,
        source_repo,
        path,
    }
}

fn init_git_repo(path: &Path) {
    run_command(path, "git", &["init", "--quiet"]);
    run_command(path, "git", &["config", "user.name", "Atlas Tests"]);
    run_command(
        path,
        "git",
        &["config", "user.email", "atlas-tests@example.com"],
    );
    run_command(path, "git", &["add", "."]);
    run_command(
        path,
        "git",
        &["commit", "--quiet", "-m", "fixture baseline"],
    );
}

fn fixture_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_repo")
}

fn current_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace repo root")
        .to_path_buf()
}

fn read_golden_json(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("golden file")).expect("golden json")
}

fn normalize_query_results(value: &mut Value) {
    value["latency_ms"] = json!(0);
    let Some(results) = value["results"].as_array_mut() else {
        panic!("query output results should be an array");
    };

    for result in results {
        result["score"] = json!(0.0);
        result["node"]["id"] = json!(0);
        result["node"]["file_hash"] = json!("<hash>");
    }
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8")
}

fn assert_contains_all(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected output to contain {needle:?}\nstdout:\n{haystack}"
        );
    }
}

fn read_json_output(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("valid json output")
}

fn parse_jsonrpc_lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8(stdout.to_vec())
        .expect("jsonrpc stdout utf-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("jsonrpc response line"))
        .collect()
}

fn list_mcp_instance_metadata(repo_root: &Path) -> Vec<Value> {
    let instances_dir = repo_root.join(".atlas").join("mcp");
    let mut entries = Vec::new();
    if !instances_dir.exists() {
        return entries;
    }

    for entry in fs::read_dir(&instances_dir).expect("mcp instances dir") {
        let entry = entry.expect("mcp instance entry");
        let metadata_path = entry.path().join("mcp.instance.json");
        if !metadata_path.exists() {
            continue;
        }
        let value: Value = serde_json::from_str(
            &fs::read_to_string(&metadata_path).expect("mcp instance metadata text"),
        )
        .expect("mcp instance metadata json");
        entries.push(value);
    }

    entries.sort_by(|left, right| left["db_path"].as_str().cmp(&right["db_path"].as_str()));
    entries
}

fn pid_exists(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        return Path::new("/proc").join(pid.to_string()).exists();
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }

        let result = unsafe { libc::kill(pid as i32, 0) };
        if result == 0 {
            return true;
        }

        return matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        );
    }

    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn kill_pid(pid: u32) {
    let output = sanitized_command("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .unwrap_or_else(|err| panic!("failed to kill pid {pid}: {err}"));
    assert!(
        output.status.success(),
        "kill -9 {pid} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(predicate(), "condition did not become true within {timeout:?}");
}

fn read_json_data_output(command: &str, output: Output) -> Value {
    let value = read_json_output(output);
    assert_eq!(value["schema_version"], json!("atlas_cli.v1"));
    assert_eq!(value["command"], json!(command));
    value["data"].clone()
}

fn normalize_context_result(value: &mut serde_json::Value) {
    if let Some(nodes) = value["nodes"].as_array_mut() {
        for n in nodes.iter_mut() {
            n["node"]["id"] = json!(0);
            n["node"]["file_hash"] = json!("<hash>");
        }
    }
    if let Some(edges) = value["edges"].as_array_mut() {
        for e in edges.iter_mut() {
            e["edge"]["id"] = json!(0);
        }
    }
}

fn write_repo_file(repo_root: &Path, relative_path: &str, content: &str) {
    fs::write(repo_root.join(relative_path), content).expect("write repo file");
}

fn rewrite_fixture_helper(repo_root: &Path) {
    write_repo_file(
        repo_root,
        "src/lib.rs",
        r#"pub struct Greeter {
    times: usize,
}

impl Greeter {
    pub fn greet_twice(name: &str) -> String {
        Self::new(2).render(name)
    }

    pub fn new(times: usize) -> Self {
        Self { times }
    }

    pub fn render(&self, name: &str) -> String {
        format!("Hello, {name}! Hello again, {name}! x{}", self.times)
    }
}

pub fn helper(name: &str, suffix: &str) -> String {
    let greeting = Greeter::greet_twice(name);
    format!("{greeting} [{suffix}]")
}
"#,
    );
}

fn open_store(repo_root: &Path) -> Store {
    let db_path = repo_root.join(".atlas").join("worldtree.db");
    Store::open(db_path.to_str().expect("atlas db path string")).expect("open atlas store")
}

struct LookupEvalCase<'a> {
    query: &'a str,
    expected_qn: &'a str,
}

fn atlas_query_qnames(data: &Value) -> Vec<String> {
    data["results"]
        .as_array()
        .expect("query results array")
        .iter()
        .filter_map(|result| result["node"]["qualified_name"].as_str().map(str::to_owned))
        .collect()
}

fn plain_grep_ranked_candidates(
    repo_root: &Path,
    store: &Store,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let needle = query.to_ascii_lowercase();
    let mut candidates: Vec<(String, usize)> = tracked_repo_files(repo_root)
        .into_iter()
        .filter_map(|file| {
            let content = fs::read_to_string(repo_root.join(&file)).ok()?;
            let count = content.to_ascii_lowercase().matches(&needle).count();
            (count > 0).then_some((file, count))
        })
        .collect();
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut qnames = Vec::new();
    for (file, _) in candidates {
        let nodes = store.nodes_by_file(&file).expect("nodes by file from grep baseline");
        for node in nodes {
            let name_matches = node.name.eq_ignore_ascii_case(query);
            let qname_matches = node
                .qualified_name
                .split("::")
                .any(|part| part.eq_ignore_ascii_case(query));
            if name_matches || qname_matches {
                qnames.push(node.qualified_name);
                if qnames.len() >= limit {
                    return qnames;
                }
            }
        }
    }
    qnames
}

fn tracked_repo_files(repo_root: &Path) -> Vec<String> {
    let output = run_command(repo_root, "git", &["ls-files"]);
    String::from_utf8(output.stdout)
        .expect("git ls-files output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn run_atlas(repo_root: &Path, args: &[&str]) -> Output {
    run_command(repo_root, env!("CARGO_BIN_EXE_atlas"), args)
}

fn json_path(value: &Value) -> PathBuf {
    PathBuf::from(value.as_str().expect("json path string"))
}

fn canonical_path(path: &Path) -> PathBuf {
    path.canonicalize().expect("canonical path")
}

fn run_command(repo_root: &Path, program: &str, args: &[&str]) -> Output {
    let output = sanitized_command(program)
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("failed to run {program} {:?}: {err}", args));
    assert!(
        output.status.success(),
        "command failed: {program} {:?}\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

struct SpawnedWatch {
    child: Child,
    stdout_rx: mpsc::Receiver<String>,
    stderr_rx: mpsc::Receiver<String>,
}

impl SpawnedWatch {
    fn recv_stdout_line(&mut self, timeout: Duration) -> String {
        self.stdout_rx
            .recv_timeout(timeout)
            .unwrap_or_else(|err| {
                let stderr = drain_lines(&self.stderr_rx);
                panic!("timed out waiting for watch stdout: {err}; stderr={stderr:?}")
            })
    }
}

impl Drop for SpawnedWatch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = drain_lines(&self.stdout_rx);
        let _ = drain_lines(&self.stderr_rx);
    }
}

fn spawn_atlas_watch(repo_root: &Path, args: &[&str]) -> SpawnedWatch {
    let mut child = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(args)
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas watch: {err}"));

    let stdout = child.stdout.take().expect("watch stdout");
    let stderr = child.stderr.take().expect("watch stderr");

    SpawnedWatch {
        child,
        stdout_rx: spawn_line_reader(stdout),
        stderr_rx: spawn_line_reader(stderr),
    }
}

fn spawn_line_reader<R>(reader: R) -> mpsc::Receiver<String>
where
    R: std::io::Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let buffered = BufReader::new(reader);
        for line in buffered.lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

fn drain_lines(rx: &mpsc::Receiver<String>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(line) = rx.try_recv() {
        lines.push(line);
    }
    lines
}

fn copy_dir_all(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).expect("fixture dir") {
        let entry = entry.expect("fixture entry");
        let file_type = entry.file_type().expect("fixture entry type");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&dst_path).expect("create fixture dir");
            copy_dir_all(&src_path, &dst_path);
        } else {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent).expect("create fixture file parent");
            }
            fs::copy(&src_path, &dst_path).expect("copy fixture file");
        }
    }
}
