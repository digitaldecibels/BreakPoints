//! What a scan returns. Every detector produces these shapes and nothing else,
//! which is what lets a new detector be added without touching the merge.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a width came from, in the order that decides which one wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Something we inferred rather than read.
    Inferred,
    /// A width found in a stylesheet's media queries.
    Css,
    /// A framework convention, not written down in this project.
    Framework,
    /// Written down deliberately: Tailwind's screens, a Drupal breakpoints file.
    Configured,
}

/// Which side of a media query boundary a width describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    /// `min-width: 768px`, so the panel renders at 768.
    Min,
    /// `max-width: 767px`, so the panel renders at 767.
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakpointDiscovery {
    pub width: f64,
    /// The framework's own name for it, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Human-readable provenance, e.g. "tailwind.config.js theme.screens".
    pub source: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub confidence: f64,
    pub kind: Kind,
    pub edge: Edge,
    /// How many files this width appears in. A width used in nine files is a
    /// real breakpoint; one used once is probably a tweak.
    pub file_count: usize,
}

impl BreakpointDiscovery {
    pub fn configured(
        width: f64,
        name: impl Into<String>,
        source: impl Into<String>,
        file: impl Into<String>,
        line: Option<usize>,
    ) -> Self {
        Self {
            width,
            name: Some(name.into()),
            source: source.into(),
            source_file: file.into(),
            line,
            confidence: 0.95,
            kind: Kind::Configured,
            edge: Edge::Min,
            file_count: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameworkDetection {
    pub framework: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub confidence: f64,
    pub source_file: String,
    pub breakpoints: Vec<BreakpointDiscovery>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServerDiscovery {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub source: String,
    pub confidence: f64,
    /// Filled in after the reachability check. `None` means not checked yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responding: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictSide {
    pub label: String,
    pub width: f64,
    pub source_file: String,
    pub detail: String,
}

/// Two sources disagreeing about the same named or nearby breakpoint. Surfaced,
/// never resolved silently.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub name: String,
    pub left: ConflictSide,
    pub right: ConflictSide,
}

/// A parse failure or a discarded value, worded for the one-line warning row in
/// the scan sheet. `detail` is what the "why?" expander shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanWarning {
    pub file: String,
    pub message: String,
    #[serde(default)]
    pub detail: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub project_root: String,
    pub project_name: String,
    pub frameworks: Vec<FrameworkDetection>,
    /// Merged and deduped across every detector, best source first.
    pub breakpoints: Vec<BreakpointDiscovery>,
    pub dev_servers: Vec<DevServerDiscovery>,
    pub conflicts: Vec<Conflict>,
    pub warnings: Vec<ScanWarning>,
    pub scanned_files: usize,
    pub skipped_files: usize,
    pub css_files: usize,
    pub duration_ms: u64,
    /// Hash of the extracted widths, not of the file bytes, so a save that
    /// changes nothing relevant is a no-op.
    pub source_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    /// True when the walk hit a budget and stopped early.
    pub truncated: bool,
}

/// What one detector produced. The scanner merges these.
#[derive(Debug, Default)]
pub struct DetectorOutput {
    pub frameworks: Vec<FrameworkDetection>,
    pub breakpoints: Vec<BreakpointDiscovery>,
    pub dev_servers: Vec<DevServerDiscovery>,
    pub warnings: Vec<ScanWarning>,
}
