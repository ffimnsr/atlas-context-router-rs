/// Hidden working directory created in the repo root.
pub const ATLAS_DIR: &str = ".atlas";

/// Default SQLite database filename inside the atlas work directory.
pub const ATLAS_DB: &str = "worldtree.db";

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

/// Return the config file path given a repo root.
pub fn config_path(repo_root: &str) -> std::path::PathBuf {
    atlas_dir(repo_root).join(ATLAS_CONFIG)
}
