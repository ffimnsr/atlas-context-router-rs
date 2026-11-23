use super::*;

#[test]
fn register_first_dimension_stores_entry() {
    let mut store = open_store();
    store
        .register_embedding_dimension("ollama", "nomic-embed-text", 768)
        .unwrap();
    let entry = store
        .get_embedding_provider("ollama", "nomic-embed-text")
        .unwrap()
        .expect("entry should be stored");
    assert_eq!(entry.provider_name, "ollama");
    assert_eq!(entry.model_name, "nomic-embed-text");
    assert_eq!(entry.dimension, 768);
    assert_eq!(entry.index_schema_version, 1);
    assert!(!entry.discovered_at.is_empty());
}

#[test]
fn register_same_dimension_is_idempotent() {
    let mut store = open_store();
    store
        .register_embedding_dimension("ollama", "nomic-embed-text", 768)
        .unwrap();
    // Second call with same dimension must succeed silently.
    store
        .register_embedding_dimension("ollama", "nomic-embed-text", 768)
        .unwrap();
    let entries = store.list_embedding_providers().unwrap();
    assert_eq!(entries.len(), 1, "idempotent: must not create duplicate");
}

#[test]
fn dimension_mismatch_on_insert_returns_error() {
    let mut store = open_store();
    store
        .register_embedding_dimension("ollama", "nomic-embed-text", 768)
        .unwrap();
    let result = store.register_embedding_dimension("ollama", "nomic-embed-text", 1024);
    assert!(result.is_err(), "mismatched dimension must return error");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("dimension mismatch"),
        "error message must mention 'dimension mismatch'; got: {msg}"
    );
    assert!(
        msg.contains("768"),
        "error must include expected dimension 768; got: {msg}"
    );
    assert!(
        msg.contains("1024"),
        "error must include actual dimension 1024; got: {msg}"
    );
}

#[test]
fn dimension_mismatch_on_query_returns_error() {
    let mut store = open_store();
    store
        .register_embedding_dimension("openai", "text-embedding-3-small", 1536)
        .unwrap();
    // Simulate a query embedding with wrong dimension.
    let result = store.check_embedding_dimension("openai", "text-embedding-3-small", 768);
    assert!(result.is_err(), "dimension mismatch on query must fail");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("1536"), "expected 1536; got: {msg}");
    assert!(msg.contains("768"), "actual 768; got: {msg}");
}

#[test]
fn check_dimension_succeeds_when_no_entry_and_registers() {
    let mut store = open_store();
    // No entry yet — first call acts as registration.
    store
        .check_embedding_dimension("ollama", "mxbai-embed-large", 512)
        .unwrap();
    let entry = store
        .get_embedding_provider("ollama", "mxbai-embed-large")
        .unwrap();
    assert!(entry.is_some(), "first check must register the entry");
    assert_eq!(entry.unwrap().dimension, 512);
}

#[test]
fn cached_dimension_avoids_db_lookup() {
    let mut store = open_store();
    store
        .register_embedding_dimension("ollama", "all-minilm", 384)
        .unwrap();
    // Cache should be populated after register.
    assert_eq!(
        store.cached_dimension("ollama", "all-minilm"),
        Some(384),
        "dimension must be in cache after registration"
    );
    // Second check uses cache path — must still pass.
    store
        .check_embedding_dimension("ollama", "all-minilm", 384)
        .unwrap();
}

#[test]
fn provider_switch_with_incompatible_existing_index() {
    let mut store = open_store();
    // Register ollama/nomic at 768.
    store
        .register_embedding_dimension("ollama", "nomic-embed-text", 768)
        .unwrap();
    // Simulate switching to openai/ada-002 at 1536 — different provider+model is allowed.
    store
        .register_embedding_dimension("openai", "text-embedding-ada-002", 1536)
        .unwrap();
    let entries = store.list_embedding_providers().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "two distinct provider+model combos allowed"
    );
    // But if openai/ada-002 is re-registered with a different dimension it should fail.
    let result = store.register_embedding_dimension("openai", "text-embedding-ada-002", 768);
    assert!(result.is_err(), "incompatible dimension swap must fail");
}

#[test]
fn list_embedding_providers_returns_all_entries_sorted() {
    let mut store = open_store();
    store
        .register_embedding_dimension("openai", "text-embedding-3-small", 1536)
        .unwrap();
    store
        .register_embedding_dimension("ollama", "nomic-embed-text", 768)
        .unwrap();
    store
        .register_embedding_dimension("ollama", "all-minilm", 384)
        .unwrap();
    let entries = store.list_embedding_providers().unwrap();
    assert_eq!(entries.len(), 3);
    // Sorted by provider_name then model_name.
    assert_eq!(entries[0].provider_name, "ollama");
    assert_eq!(entries[0].model_name, "all-minilm");
    assert_eq!(entries[1].provider_name, "ollama");
    assert_eq!(entries[1].model_name, "nomic-embed-text");
    assert_eq!(entries[2].provider_name, "openai");
}

#[test]
fn warm_dimension_cache_pre_populates_from_db() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);

    // First store instance: register dimension, then close.
    {
        let mut store = ContentStore::open(&path).unwrap();
        store.migrate().unwrap();
        store
            .register_embedding_dimension("ollama", "nomic-embed-text", 768)
            .unwrap();
    }

    // Second store instance: warm cache and verify it reflects persisted data.
    {
        let mut store = ContentStore::open(&path).unwrap();
        store.migrate().unwrap();
        store.warm_dimension_cache().unwrap();
        assert_eq!(
            store.cached_dimension("ollama", "nomic-embed-text"),
            Some(768),
            "cache must be pre-populated from persisted registry"
        );
    }
}

#[test]
fn dimension_mismatch_error_display_is_descriptive() {
    let err = super::DimensionMismatchError {
        provider_name: "ollama".to_owned(),
        model_name: "nomic-embed-text".to_owned(),
        expected_dimension: 768,
        actual_dimension: 1024,
    };
    let msg = err.to_string();
    assert!(msg.contains("ollama"));
    assert!(msg.contains("nomic-embed-text"));
    assert!(msg.contains("768"));
    assert!(msg.contains("1024"));
    assert!(msg.contains("rebuild"));
}
