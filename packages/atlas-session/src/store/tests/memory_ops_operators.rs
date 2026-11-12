use super::*;
use crate::store::memory_ops::{failure_signature, failure_topic, memory_op_now};
use serde_json::json;

fn entity() -> MemoryEntity {
    MemoryEntity {
        id: "m1".to_owned(),
        repo_root: "/repo".to_owned(),
        topic: "topic".to_owned(),
        title: "title".to_owned(),
        body: "body".to_owned(),
        importance: MemoryImportance::Normal,
        scope: MemoryScope::Project,
        source_id: None,
        metadata: json!({}),
        numeric_value: None,
        op_history: Vec::new(),
        superseded_by: None,
    }
}

#[test]
fn set_updates_scalar_and_metadata_fields() {
    let mut entity = entity();
    apply_memory_op(
        &mut entity,
        &MemoryOp::Set {
            field: "importance".to_owned(),
            value: json!("critical"),
        },
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    assert_eq!(entity.importance, MemoryImportance::Critical);

    apply_memory_op(
        &mut entity,
        &MemoryOp::Set {
            field: "metadata.owner".to_owned(),
            value: json!("core"),
        },
        "2026-01-01T00:00:01Z",
    )
    .unwrap();
    assert_eq!(entity.metadata["owner"], json!("core"));
    assert_eq!(entity.op_history.len(), 2);
}

#[test]
fn set_rejects_unknown_and_empty_body_fields() {
    let mut entity = entity();
    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::Set {
                field: "unknown".to_owned(),
                value: json!("x"),
            },
            "2026-01-01T00:00:00Z",
        )
        .is_err()
    );
    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::Set {
                field: "body".to_owned(),
                value: json!("   "),
            },
            "2026-01-01T00:00:00Z",
        )
        .is_err()
    );
}

#[test]
fn sum_and_count_are_overflow_checked() {
    let mut entity = entity();
    let now = "2026-01-01T00:00:00Z";
    apply_memory_op(
        &mut entity,
        &MemoryOp::Sum {
            field: "tokens".to_owned(),
            delta: 7,
        },
        now,
    )
    .unwrap();
    apply_memory_op(
        &mut entity,
        &MemoryOp::Sum {
            field: "tokens".to_owned(),
            delta: -2,
        },
        now,
    )
    .unwrap();
    assert_eq!(entity.metadata["tokens"], json!(5));
    assert_eq!(entity.numeric_value, Some(5.0));

    apply_memory_op(
        &mut entity,
        &MemoryOp::Count {
            field: "fail_count".to_owned(),
        },
        now,
    )
    .unwrap();
    assert_eq!(entity.metadata["fail_count"], json!(1));
    assert_eq!(entity.metadata["last_seen"], json!(now));

    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::Sum {
                field: "tokens".to_owned(),
                delta: i64::MAX,
            },
            now,
        )
        .is_err()
    );

    let mut malformed = self::entity();
    apply_memory_op(
        &mut malformed,
        &MemoryOp::Set {
            field: "metadata.tokens".to_owned(),
            value: json!("many"),
        },
        now,
    )
    .unwrap();
    assert!(
        apply_memory_op(
            &mut malformed,
            &MemoryOp::Sum {
                field: "tokens".to_owned(),
                delta: 1,
            },
            now,
        )
        .is_err(),
        "numeric operators must reject non-numeric metadata fields"
    );
}

#[test]
fn max_handles_numbers_and_timestamps() {
    let mut entity = entity();
    let now = "2026-01-01T00:00:00Z";
    apply_memory_op(
        &mut entity,
        &MemoryOp::Max {
            field: "peak".to_owned(),
            value: json!(3),
        },
        now,
    )
    .unwrap();
    apply_memory_op(
        &mut entity,
        &MemoryOp::Max {
            field: "peak".to_owned(),
            value: json!(9),
        },
        now,
    )
    .unwrap();
    apply_memory_op(
        &mut entity,
        &MemoryOp::Max {
            field: "peak".to_owned(),
            value: json!(4),
        },
        now,
    )
    .unwrap();
    assert_eq!(entity.metadata["peak"], json!(9));

    apply_memory_op(
        &mut entity,
        &MemoryOp::Max {
            field: "last_failed_at".to_owned(),
            value: json!("2026-01-02T00:00:00Z"),
        },
        now,
    )
    .unwrap();
    apply_memory_op(
        &mut entity,
        &MemoryOp::Max {
            field: "last_failed_at".to_owned(),
            value: json!("2026-01-01T12:00:00Z"),
        },
        now,
    )
    .unwrap();
    assert_eq!(
        entity.metadata["last_failed_at"],
        json!("2026-01-02T00:00:00Z")
    );

    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::Max {
                field: "mixed".to_owned(),
                value: json!(1),
            },
            now,
        )
        .is_ok()
    );
    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::Max {
                field: "mixed".to_owned(),
                value: json!("later"),
            },
            now,
        )
        .is_err()
    );
}

#[test]
fn avg_keeps_running_average_and_count() {
    let mut entity = entity();
    let now = "2026-01-01T00:00:00Z";
    for sample in [2.0, 4.0, 6.0] {
        apply_memory_op(
            &mut entity,
            &MemoryOp::Avg {
                field: "latency".to_owned(),
                sample,
            },
            now,
        )
        .unwrap();
    }
    assert_eq!(entity.metadata["latency"], json!(4.0));
    assert_eq!(entity.metadata["latency_count"], json!(3));
    assert_eq!(entity.numeric_value, Some(4.0));

    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::Avg {
                field: "latency".to_owned(),
                sample: f64::NAN,
            },
            now,
        )
        .is_err()
    );
}

#[test]
fn append_unique_dedupes_and_caps_items() {
    let mut entity = entity();
    let now = "2026-01-01T00:00:00Z";
    for item in ["a", "b", "a", "c"] {
        apply_memory_op(
            &mut entity,
            &MemoryOp::AppendUnique {
                field: "signatures".to_owned(),
                item: json!(item),
                max_items: 2,
            },
            now,
        )
        .unwrap();
    }
    assert_eq!(entity.metadata["signatures"], json!(["b", "c"]));

    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::AppendUnique {
                field: "signatures".to_owned(),
                item: json!("d"),
                max_items: 0,
            },
            now,
        )
        .is_err()
    );
}

#[test]
fn history_is_capped_and_defers_vm2_operators() {
    let mut entity = entity();
    for index in 0..(MAX_OP_HISTORY_ENTRIES + 4) {
        apply_memory_op(
            &mut entity,
            &MemoryOp::Count {
                field: "runs".to_owned(),
            },
            &format!("2026-01-01T00:00:{index:02}Z"),
        )
        .unwrap();
    }
    assert_eq!(entity.op_history.len(), MAX_OP_HISTORY_ENTRIES);
    assert_eq!(entity.op_history.last().unwrap().op, "count");

    let before = entity.clone();
    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::EuaPatch {
                search: "a".to_owned(),
                replace: "b".to_owned(),
                max_edit_distance: 1,
            },
            "2026-01-01T00:00:00Z",
        )
        .is_err()
    );
    assert_eq!(entity.op_history, before.op_history);
    assert!(
        apply_memory_op(
            &mut entity,
            &MemoryOp::TimeCompress {
                window_label: "week".to_owned(),
            },
            "2026-01-01T00:00:00Z",
        )
        .is_err()
    );
}

#[test]
fn failure_topic_and_signature_are_deterministic() {
    assert_eq!(failure_topic("cargo test --quiet"), FAILURE_TOPIC_TESTS);
    assert_eq!(failure_topic("cargo build"), FAILURE_TOPIC_BUILD);
    let payload = json!({ "stderr": "error:   missing semicolon" }).to_string();
    let first = failure_signature(&payload, "cargo build");
    let second = failure_signature(&payload, "cargo build");
    assert_eq!(first, second);
    assert_eq!(first.len(), 64);
    assert_eq!(
        first,
        failure_signature(&payload, "cargo check"),
        "same failure text must dedupe across commands"
    );
    let other_payload = json!({ "stderr": "error: unused import" }).to_string();
    assert_ne!(first, failure_signature(&other_payload, "cargo build"));
}

#[test]
fn now_timestamp_is_second_precision() {
    let now = memory_op_now();
    assert_eq!(now.len(), "2026-01-01T00:00:00Z".len());
    assert!(!now.contains('.'));
}
