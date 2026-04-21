/// Hidden working directory created in the repo root.
pub const ATLAS_DIR: &str = ".atlas";

/// Default SQLite database filename inside the atlas work directory.
pub const ATLAS_DB: &str = "worldtree.db";

/// Default content-store SQLite filename inside the atlas work directory.
pub const ATLAS_CONTENT_DB: &str = "context.db";

/// Default session-store SQLite filename inside the atlas work directory.
pub const ATLAS_SESSION_DB: &str = "session.db";

/// Default config filename inside the atlas work directory.
pub const ATLAS_CONFIG: &str = "config.toml";

/// Return the path to the atlas work directory given a repo root.
pub fn atlas_dir(repo_root: &str) -> std::path::PathBuf {
    std::path::Path::new(repo_root).join(ATLAS_DIR)
}

/// Return the default DB path given a repo root.
pub fn default_db_path(repo_root: &str) -> String {
    atlas_dir(repo_root)
        .join(ATLAS_DB)
        .to_string_lossy()
        .into_owned()
}

fn sibling_db_path(db_path: &str, default_name: &str) -> String {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        parent.join(default_name).to_string_lossy().into_owned()
    } else {
        default_name.to_string()
    }
}

/// Return session-store DB path next to graph DB path.
pub fn session_db_path(db_path: &str) -> String {
    sibling_db_path(db_path, ATLAS_SESSION_DB)
}

/// Return content-store DB path next to graph DB path.
pub fn content_db_path(db_path: &str) -> String {
    sibling_db_path(db_path, ATLAS_CONTENT_DB)
}

/// Return the config file path given a repo root.
pub fn config_path(repo_root: &str) -> std::path::PathBuf {
    atlas_dir(repo_root).join(ATLAS_CONFIG)
}
