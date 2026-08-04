//! Insights configuration: thresholds, weights, layer rules, and external
//! layer-rules file handling.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{
    validate_f64_range, validate_nonempty_string, validate_ordered_score_thresholds,
    validate_positive_f64, validate_usize_limit,
};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InsightsLayerRule {
    pub name: String,
    pub path_prefixes: Vec<String>,
    pub module_prefixes: Vec<String>,
}

/// External layer-rules file shape: `[[layer_rules]]` entries mirror the
/// inline `[[insights.layer_rules]]` config surface.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayerRulesFile {
    layer_rules: Vec<InsightsLayerRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InsightsConfig {
    pub large_function_loc: usize,
    pub repeated_call_chain_min_length: usize,
    pub high_fan_in: usize,
    pub high_fan_out: usize,
    pub high_coupling: usize,
    pub deep_chain_length: usize,
    pub max_findings: usize,
    pub high_cyclomatic_complexity: usize,
    pub high_cognitive_complexity: usize,
    pub max_nesting_depth: usize,
    pub branch_count: usize,
    pub outlier_percentile_cutoff: usize,
    pub risk_public_api_weight: f64,
    pub risk_fan_in_weight: f64,
    pub risk_fan_out_weight: f64,
    pub risk_cross_module_dependency_weight: f64,
    pub risk_test_adjacency_mitigation_weight: f64,
    pub risk_dependency_depth_weight: f64,
    pub risk_unresolved_edge_weight: f64,
    pub risk_large_function_weight: f64,
    pub risk_loc_weight: f64,
    pub risk_cyclomatic_complexity_weight: f64,
    pub risk_cognitive_complexity_weight: f64,
    pub risk_nesting_depth_weight: f64,
    pub risk_cycle_participation_weight: f64,
    pub risk_medium_threshold: f64,
    pub risk_high_threshold: f64,
    pub similarity_high_threshold: f64,
    pub similarity_medium_threshold: f64,
    pub similarity_low_threshold: f64,
    pub duplicate_high_threshold: f64,
    pub duplicate_medium_threshold: f64,
    pub duplicate_low_threshold: f64,
    pub duplicate_suppressions: Vec<String>,
    pub ignore_files: Vec<String>,
    pub ignore_modules: Vec<String>,
    pub ignore_node_kinds: Vec<String>,
    pub layer_rules: Vec<InsightsLayerRule>,
    /// Optional path to a TOML file containing `[[layer_rules]]` entries.
    /// When set, replaces inline `layer_rules`. Relative paths resolve from
    /// the `.atlas/` directory, matching `sanitization.redaction_rules_file`.
    pub layer_rules_file: Option<String>,
}

impl Default for InsightsConfig {
    fn default() -> Self {
        Self {
            large_function_loc: 80,
            repeated_call_chain_min_length: 3,
            high_fan_in: 20,
            high_fan_out: 10,
            high_coupling: 15,
            deep_chain_length: 6,
            max_findings: 50,
            high_cyclomatic_complexity: 15,
            high_cognitive_complexity: 20,
            max_nesting_depth: 4,
            branch_count: 12,
            outlier_percentile_cutoff: 95,
            risk_public_api_weight: 1.5,
            risk_fan_in_weight: 1.25,
            risk_fan_out_weight: 0.75,
            risk_cross_module_dependency_weight: 1.0,
            risk_test_adjacency_mitigation_weight: 1.0,
            risk_dependency_depth_weight: 0.75,
            risk_unresolved_edge_weight: 1.25,
            risk_large_function_weight: 0.5,
            risk_loc_weight: 0.75,
            risk_cyclomatic_complexity_weight: 1.0,
            risk_cognitive_complexity_weight: 1.0,
            risk_nesting_depth_weight: 0.75,
            risk_cycle_participation_weight: 1.0,
            risk_medium_threshold: 35.0,
            risk_high_threshold: 70.0,
            similarity_high_threshold: 0.72,
            similarity_medium_threshold: 0.55,
            similarity_low_threshold: 0.40,
            duplicate_high_threshold: 0.86,
            duplicate_medium_threshold: 0.74,
            duplicate_low_threshold: 0.64,
            duplicate_suppressions: Vec::new(),
            ignore_files: Vec::new(),
            ignore_modules: Vec::new(),
            ignore_node_kinds: Vec::new(),
            layer_rules: Vec::new(),
            layer_rules_file: None,
        }
    }
}

impl InsightsConfig {
    pub fn validate(&self) -> Result<()> {
        validate_usize_limit(
            "insights.large_function_loc",
            self.large_function_loc,
            usize::MAX,
        )?;
        validate_usize_limit(
            "insights.repeated_call_chain_min_length",
            self.repeated_call_chain_min_length,
            usize::MAX,
        )?;
        if self.repeated_call_chain_min_length < 2 {
            anyhow::bail!(
                "invalid config: insights.repeated_call_chain_min_length={} must be at least 2",
                self.repeated_call_chain_min_length,
            );
        }
        validate_usize_limit("insights.high_fan_in", self.high_fan_in, usize::MAX)?;
        validate_usize_limit("insights.high_fan_out", self.high_fan_out, usize::MAX)?;
        validate_usize_limit("insights.high_coupling", self.high_coupling, usize::MAX)?;
        validate_usize_limit(
            "insights.deep_chain_length",
            self.deep_chain_length,
            usize::MAX,
        )?;
        validate_usize_limit("insights.max_findings", self.max_findings, usize::MAX)?;
        validate_usize_limit(
            "insights.high_cyclomatic_complexity",
            self.high_cyclomatic_complexity,
            usize::MAX,
        )?;
        validate_usize_limit(
            "insights.high_cognitive_complexity",
            self.high_cognitive_complexity,
            usize::MAX,
        )?;
        validate_usize_limit(
            "insights.max_nesting_depth",
            self.max_nesting_depth,
            usize::MAX,
        )?;
        validate_usize_limit("insights.branch_count", self.branch_count, usize::MAX)?;
        validate_usize_limit(
            "insights.outlier_percentile_cutoff",
            self.outlier_percentile_cutoff,
            100,
        )?;
        validate_positive_f64(
            "insights.risk_public_api_weight",
            self.risk_public_api_weight,
        )?;
        validate_positive_f64("insights.risk_fan_in_weight", self.risk_fan_in_weight)?;
        validate_positive_f64("insights.risk_fan_out_weight", self.risk_fan_out_weight)?;
        validate_positive_f64(
            "insights.risk_cross_module_dependency_weight",
            self.risk_cross_module_dependency_weight,
        )?;
        validate_positive_f64(
            "insights.risk_test_adjacency_mitigation_weight",
            self.risk_test_adjacency_mitigation_weight,
        )?;
        validate_positive_f64(
            "insights.risk_dependency_depth_weight",
            self.risk_dependency_depth_weight,
        )?;
        validate_positive_f64(
            "insights.risk_unresolved_edge_weight",
            self.risk_unresolved_edge_weight,
        )?;
        validate_positive_f64(
            "insights.risk_large_function_weight",
            self.risk_large_function_weight,
        )?;
        validate_positive_f64("insights.risk_loc_weight", self.risk_loc_weight)?;
        validate_positive_f64(
            "insights.risk_cyclomatic_complexity_weight",
            self.risk_cyclomatic_complexity_weight,
        )?;
        validate_positive_f64(
            "insights.risk_cognitive_complexity_weight",
            self.risk_cognitive_complexity_weight,
        )?;
        validate_positive_f64(
            "insights.risk_nesting_depth_weight",
            self.risk_nesting_depth_weight,
        )?;
        validate_positive_f64(
            "insights.risk_cycle_participation_weight",
            self.risk_cycle_participation_weight,
        )?;
        validate_f64_range(
            "insights.risk_medium_threshold",
            self.risk_medium_threshold,
            0.0,
            100.0,
        )?;
        validate_f64_range(
            "insights.risk_high_threshold",
            self.risk_high_threshold,
            0.0,
            100.0,
        )?;
        if self.risk_medium_threshold >= self.risk_high_threshold {
            anyhow::bail!(
                "invalid config: insights.risk_medium_threshold ({}) must be less than insights.risk_high_threshold ({})",
                self.risk_medium_threshold,
                self.risk_high_threshold,
            );
        }
        validate_ordered_score_thresholds(
            "insights similarity thresholds",
            self.similarity_low_threshold,
            self.similarity_medium_threshold,
            self.similarity_high_threshold,
        )?;
        validate_ordered_score_thresholds(
            "insights duplicate thresholds",
            self.duplicate_low_threshold,
            self.duplicate_medium_threshold,
            self.duplicate_high_threshold,
        )?;

        for (index, value) in self.duplicate_suppressions.iter().enumerate() {
            validate_nonempty_string(&format!("insights.duplicate_suppressions[{index}]"), value)?;
        }
        for (index, value) in self.ignore_files.iter().enumerate() {
            validate_nonempty_string(&format!("insights.ignore_files[{index}]"), value)?;
        }
        for (index, value) in self.ignore_modules.iter().enumerate() {
            validate_nonempty_string(&format!("insights.ignore_modules[{index}]"), value)?;
        }
        for (index, value) in self.ignore_node_kinds.iter().enumerate() {
            validate_nonempty_string(&format!("insights.ignore_node_kinds[{index}]"), value)?;
        }

        if self.layer_rules_file.is_some() {
            return Ok(());
        }

        let mut seen_names = std::collections::BTreeSet::new();
        for (index, rule) in self.layer_rules.iter().enumerate() {
            let name = validate_nonempty_string(
                &format!("insights.layer_rules[{index}].name"),
                &rule.name,
            )?;
            if !seen_names.insert(name.clone()) {
                anyhow::bail!(
                    "invalid config: insights.layer_rules[{index}].name duplicates layer `{name}`"
                );
            }
            if rule.path_prefixes.is_empty() && rule.module_prefixes.is_empty() {
                anyhow::bail!(
                    "invalid config: insights.layer_rules[{index}] must define path_prefixes or module_prefixes"
                );
            }
            for (matcher_index, matcher) in rule.path_prefixes.iter().enumerate() {
                validate_nonempty_string(
                    &format!("insights.layer_rules[{index}].path_prefixes[{matcher_index}]"),
                    matcher,
                )?;
            }
            for (matcher_index, matcher) in rule.module_prefixes.iter().enumerate() {
                validate_nonempty_string(
                    &format!("insights.layer_rules[{index}].module_prefixes[{matcher_index}]"),
                    matcher,
                )?;
            }
        }

        Ok(())
    }

    /// Resolve `layer_rules_file` to a concrete path. Returns `None` when the
    /// option is unset; rejects blank values.
    pub fn resolve_layer_rules_file(&self, atlas_dir: &Path) -> Result<Option<PathBuf>> {
        let Some(raw_path) = self.layer_rules_file.as_deref() else {
            return Ok(None);
        };
        let trimmed = validate_nonempty_string("insights.layer_rules_file", raw_path)?;
        let candidate = Path::new(&trimmed);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            atlas_dir.join(candidate)
        };
        Ok(Some(resolved))
    }

    /// Parse a layer-rules file into rules. Shared by validation and the
    /// runtime loader so both surfaces agree on the file shape.
    fn load_layer_rules_file(path: &Path) -> Result<Vec<InsightsLayerRule>> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("cannot read layer-rules file {}", path.display()))?;
        let parsed: LayerRulesFile = toml::from_str(&raw)
            .with_context(|| format!("cannot parse layer-rules file {}", path.display()))?;
        Ok(parsed.layer_rules)
    }

    /// Runtime layer rules: external file contents when `layer_rules_file` is
    /// set, otherwise the inline rules. Rules are validated before return.
    pub fn effective_layer_rules(&self, atlas_dir: &Path) -> Result<Vec<InsightsLayerRule>> {
        let rules = match self.resolve_layer_rules_file(atlas_dir)? {
            Some(path) => Self::load_layer_rules_file(&path)?,
            None => self.layer_rules.clone(),
        };
        let resolved = Self {
            layer_rules: rules,
            layer_rules_file: None,
            ..self.clone()
        };
        resolved.validate()?;
        Ok(resolved.layer_rules)
    }

    /// Clone with runtime layer rules loaded in place, ready for engine
    /// construction at CLI/MCP entry points.
    pub fn with_loaded_layer_rules(&self, atlas_dir: &Path) -> Result<Self> {
        Ok(Self {
            layer_rules: self.effective_layer_rules(atlas_dir)?,
            ..self.clone()
        })
    }

    /// Validate the external file reference: blank values, missing paths,
    /// non-file paths, and malformed contents all fail with the config key
    /// and resolved path named in the error.
    pub fn validate_layer_rules_file(&self, atlas_dir: &Path) -> Result<()> {
        let Some(path) = self.resolve_layer_rules_file(atlas_dir)? else {
            return Ok(());
        };
        if !path.exists() {
            anyhow::bail!(
                "invalid config: insights.layer_rules_file points to missing file {}",
                path.display()
            );
        }
        if !path.is_file() {
            anyhow::bail!(
                "invalid config: insights.layer_rules_file must point to a readable file, got {}",
                path.display()
            );
        }
        let rules = Self::load_layer_rules_file(&path).with_context(|| {
            format!(
                "invalid config: insights.layer_rules_file={} failed validation",
                path.display()
            )
        })?;
        // Reuse the inline rule validation (duplicate names, empty matchers,
        // missing both matcher lists) on the external file contents.
        let candidate = Self {
            layer_rules: rules,
            layer_rules_file: None,
            ..self.clone()
        };
        candidate.validate().with_context(|| {
            format!(
                "invalid config: insights.layer_rules_file={} failed validation",
                path.display()
            )
        })?;
        Ok(())
    }
}
