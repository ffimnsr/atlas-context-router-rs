//! Typed argument-schema structs (schemars) used by the typed-input
//! builders. One file per registry split; structs are shared across
//! tool families, so they are re-exported through `super::schemas::*`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

mod analysis;
mod docs;
mod graph;
mod session;

pub(crate) use self::analysis::*;
pub(crate) use self::docs::*;
pub(crate) use self::graph::*;
pub(crate) use self::session::*;
