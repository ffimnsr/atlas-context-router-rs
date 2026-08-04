use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use atlas_core::{
    ArchitectureReport, AtlasError, EdgeKind, InsightEvidence, InsightFinding, InsightLineRange,
    InsightSeverity, Node, NodeKind, PatternReport, Result,
};
use atlas_repo::CanonicalRepoPath;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::InsightsEngine;
use super::metrics::{GraphSnapshot, build_node_metrics, load_rust_complexity, module_id_for_file};

use super::helpers;

const SIMILARITY_HIGH_THRESHOLD: f64 = 0.72;
const SIMILARITY_MEDIUM_THRESHOLD: f64 = 0.55;
const SIMILARITY_LOW_THRESHOLD: f64 = 0.40;

const SIMILAR_SHINGLE_SIZE: usize = 3;
const DUPLICATE_SHINGLE_SIZE: usize = 4;
const MAX_SIMILAR_CANDIDATES_PER_SOURCE: usize = 256;
const FINGERPRINT_CACHE_VERSION: u32 = 1;
const FINGERPRINT_CACHE_FILE: &str = "insights-fingerprint-cache.v1.json";

mod duplicates;
mod findings;
mod fingerprint;
mod labels;
mod modules;
mod similar;
mod tokens;
mod types;

pub use types::*;

use findings::*;
use fingerprint::*;
use labels::*;
use tokens::*;
