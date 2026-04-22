use super::*;

#[test]
fn interactive_shell_accepts_query_and_context_requests() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut child = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["shell", "--fuzzy"])
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas shell: {err}"));

    child
        .stdin
        .as_mut()
        .expect("shell stdin")
        .write_all(b"/query greet_twice\nwhat calls greet_twice?\nexit\n")
        .expect("write shell input");

    let output = child.wait_with_output().expect("wait for atlas shell");
    assert!(
        output.status.success(),
        "atlas shell failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Query results:"),
        "missing query output: {stdout}"
    );
    assert!(
        stdout.contains("Call chains:") || stdout.contains("Nodes ("),
        "missing context output: {stdout}"
    );
}

#[test]
fn shell_stats_shows_graph_counts() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let stdout = run_shell_with_input(repo.path(), b"/stats\nexit\n");
    assert!(
        stdout.contains("Graph stats:"),
        "missing stats header: {stdout}"
    );
    assert!(stdout.contains("Nodes"), "missing nodes count: {stdout}");
    assert!(stdout.contains("Edges"), "missing edges count: {stdout}");
}

#[test]
fn shell_help_lists_slash_commands() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let stdout = run_shell_with_input(repo.path(), b"help\nexit\n");
    assert!(stdout.contains("/query"), "help missing /query: {stdout}");
    assert!(stdout.contains("/stats"), "help missing /stats: {stdout}");
    assert!(
        stdout.contains("/changes"),
        "help missing /changes: {stdout}"
    );
    assert!(
        stdout.contains("/neighbors"),
        "help missing /neighbors: {stdout}"
    );
    assert!(
        stdout.contains("/traverse"),
        "help missing /traverse: {stdout}"
    );
}

#[test]
fn shell_neighbors_returns_callers_and_callees() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let stdout = run_shell_with_input(repo.path(), b"/neighbors greet_twice\nexit\n");
    assert!(
        stdout.contains("Neighbors of") || stdout.contains("No neighbors found"),
        "unexpected neighbors output: {stdout}"
    );
}

#[test]
fn shell_traverse_returns_reachable_nodes() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let stdout = run_shell_with_input(repo.path(), b"/traverse greet_twice\nexit\n");
    assert!(
        stdout.contains("Traverse from") || stdout.contains("Reachable nodes"),
        "unexpected traverse output: {stdout}"
    );
}

#[test]
fn shell_changes_reports_no_changes_on_clean_tree() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let stdout = run_shell_with_input(repo.path(), b"/changes\nexit\n");
    assert!(
        stdout.contains("No changed files") || stdout.contains("Changed files:"),
        "unexpected changes output: {stdout}"
    );
}

#[test]
fn shell_session_status_runs_without_error() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let stdout = run_shell_with_input(repo.path(), b"/session status\nexit\n");
    assert!(
        stdout.contains("Session status:") || stdout.contains("No active session"),
        "unexpected session output: {stdout}"
    );
}

#[test]
fn shell_flows_and_communities_list_without_error() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let flows_out = run_shell_with_input(repo.path(), b"/flows list\nexit\n");
    assert!(
        flows_out.contains("No flows") || flows_out.contains("Flows ("),
        "unexpected flows output: {flows_out}"
    );

    let comm_out = run_shell_with_input(repo.path(), b"/communities list\nexit\n");
    assert!(
        comm_out.contains("No communities") || comm_out.contains("Communities ("),
        "unexpected communities output: {comm_out}"
    );
}
