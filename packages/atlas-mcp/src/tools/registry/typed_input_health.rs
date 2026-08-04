//! Typed input-schema arm for the `health` tool family.
//!
//! Dispatched by `super::typed_input_schema_for()`; schema structs
//! come from `super::schemas`.

use super::*;
use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::Value;

pub(super) fn typed_input_schema_for(name: &str) -> Option<Value> {
    match name {
        "broker_status" | "status" | "doctor" => {
            Some(typed_schema_with_descriptions::<HealthOutputFormatArgs>(&[
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]))
        }
        "db_check" => Some(typed_schema_with_descriptions::<DbCheckArgsSchema>(&[
            (
                "properties/limit",
                "Maximum orphan/dangling samples to return (default 100).",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "debug_graph" => Some(typed_schema_with_descriptions::<DebugGraphArgsSchema>(&[
            (
                "properties/limit",
                "Maximum orphan/dangling samples to return (default 20).",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        _ => None,
    }
}
