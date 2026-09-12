//! The scanner: run every detector, merge what they found, and say where each
//! answer came from.
//!
//! Two rules matter more than the priority list. Never silently replace a
//! user's configuration with a lower-confidence discovery, and surface a
//! disagreement instead of guessing which side is right.

pub mod css;
pub mod devserver;
pub mod drupal;
pub mod jsobj;
pub mod log;
pub mod tailwind;
pub mod types;
pub mod units;
pub mod walk;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

use crate::util;
use log::ScanLog;
use types::*;
use walk::{FileIndex, ReadBudget};

/// One line in the scan sheet, emitted as its detector finishes so the sheet
/// fills in rather than appearing all at once.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRow {
    /// "ok", "warn" or "error", which is the status glyph.
    pub status: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// True when the label is a URL or a path and should be set in mono.
    pub mono: bool,
    /// Log lines for the inline "why?" expander.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<String>,
}

impl ScanRow {
    fn ok(label: impl Into<String>, source: Option<String>) -> Self {
        Self { status: "ok".into(), label: label.into(), source, mono: false, detail: Vec::new() }
    }
}

/// Same width twice from two detectors is one breakpoint.
const CROSS_TOLERANCE: f64 = 2.0;

pub struct ScanOutcome {
    pub report: ScanReport,
    pub rows: Vec<ScanRow>,
    pub log: ScanLog,
}

/// Run every detector against a project root.
///
/// `on_row` is called as each detector finishes, so the chrome can stream the
/// results rather than waiting for the whole scan.
pub fn scan(root: &Path, mut on_row: impl FnMut(ScanRow)) -> ScanOutcome {
    let started = std::time::Instant::now();
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string();

    let mut log = ScanLog::new(&name);
    let index = FileIndex::build(root, &mut log);
    let mut budget = ReadBudget::new();

    let mut rows = Vec::new();
    let emit = |row: ScanRow, rows: &mut Vec<ScanRow>, on_row: &mut dyn FnMut(ScanRow)| {
        rows.push(row.clone());
        on_row(row);
    };

    let mut frameworks = Vec::new();
    let mut breakpoints = Vec::new();
    let mut dev_servers = Vec::new();
    let mut warnings = Vec::new();

    // Framework configuration first, because it outranks everything the CSS
    // scan can find and changes how the CSS results are presented.
    for (label, detector) in [
        ("tailwind", tailwind::run as fn(&FileIndex, &mut ReadBudget, &mut ScanLog) -> DetectorOutput),
        ("drupal", drupal::run),
    ] {
        let at = std::time::Instant::now();
        log.detector_start(label);
        let output = detector(&index, &mut budget, &mut log);
        log.detector_end(label, output.breakpoints.len(), at.elapsed().as_millis() as u64);

        let mut deduped = output.frameworks.clone();
        // Several stylesheets can each look like a Tailwind v4 config. They are
        // one framework, and the one that resolved the most breakpoints is the
        // real one.
        deduped.sort_by(|a, b| {
            a.framework
                .cmp(&b.framework)
                .then(b.breakpoints.len().cmp(&a.breakpoints.len()))
                .then(a.source_file.len().cmp(&b.source_file.len()))
        });
        deduped.dedup_by(|a, b| a.framework == b.framework && a.version == b.version);

        for framework in &deduped {
            let title = match (framework.framework.as_str(), framework.version.as_deref()) {
                ("tailwind", Some(v)) => format!("Tailwind CSS v{v}"),
                ("tailwind", None) => "Tailwind CSS".into(),
                ("drupal", _) => "Drupal breakpoints".into(),
                (other, _) => other.to_string(),
            };
            emit(
                ScanRow::ok(title, Some(framework.source_file.clone())),
                &mut rows,
                &mut on_row,
            );
            let configured = framework
                .breakpoints
                .iter()
                .filter(|b| b.kind == Kind::Configured)
                .count();
            if configured > 0 {
                emit(
                    ScanRow::ok(
                        format!("{configured} configured breakpoint{}", plural(configured)),
                        None,
                    ),
                    &mut rows,
                    &mut on_row,
                );
            }
        }
        for warning in &output.warnings {
            emit(
                ScanRow {
                    status: "warn".into(),
                    label: warning.message.clone(),
                    source: Some(warning.file.clone()),
                    mono: false,
                    detail: warning.detail.clone(),
                },
                &mut rows,
                &mut on_row,
            );
        }
        frameworks.extend(deduped);
        breakpoints.extend(output.breakpoints);
        warnings.extend(output.warnings);
    }

    // Stylesheets.
    let at = std::time::Instant::now();
    log.detector_start("css");
    let css_output = css::run(&index, &mut budget, &mut log);
    log.detector_end("css", css_output.breakpoints.len(), at.elapsed().as_millis() as u64);
    let css_files: usize = index.by_extension(&css::readable_extensions()).len();
    if !css_output.breakpoints.is_empty() {
        let queries: usize = css_output.breakpoints.len();
        emit(
            ScanRow::ok(
                format!("{queries} CSS breakpoint{}", plural(queries)),
                Some(format!("{css_files} file{}", plural(css_files))),
            ),
            &mut rows,
            &mut on_row,
        );
    }
    for warning in &css_output.warnings {
        emit(
            ScanRow {
                status: "warn".into(),
                label: warning.message.clone(),
                source: Some(warning.file.clone()),
                mono: false,
                detail: warning.detail.clone(),
            },
            &mut rows,
            &mut on_row,
        );
    }
    breakpoints.extend(css_output.breakpoints);
    warnings.extend(css_output.warnings);

    // Dev server.
    let at = std::time::Instant::now();
    log.detector_start("devserver");
    let dev_output = devserver::run(&index, &mut budget, &mut log);
    log.detector_end("devserver", dev_output.dev_servers.len(), at.elapsed().as_millis() as u64);
    if let Some(best) = dev_output.dev_servers.first() {
        emit(
            ScanRow::ok(
                friendly_source(&best.source),
                Some(best.source.clone()),
            ),
            &mut rows,
            &mut on_row,
        );
        emit(
            ScanRow {
                status: "ok".into(),
                label: best.url.clone(),
                source: None,
                mono: true,
                detail: Vec::new(),
            },
            &mut rows,
            &mut on_row,
        );
    }
    dev_servers.extend(dev_output.dev_servers);

    // Merge.
    let (merged, conflicts) = merge(breakpoints, &mut log);
    let widths: Vec<f64> = merged.iter().map(|b| b.width).collect();
    log.merged(&widths, conflicts.len());

    if merged.is_empty() {
        emit(
            ScanRow {
                status: "warn".into(),
                label: "No breakpoints found".into(),
                source: Some(format!("{} file{} read", index.scanned, plural(index.scanned))),
                mono: false,
                detail: vec![
                    "No framework config resolved and no width media queries in any stylesheet.".into(),
                    "The standard device sizes are what Break/Points offers instead.".into(),
                ],
            },
            &mut rows,
            &mut on_row,
        );
    }

    let source_hash = util::short_hash(
        &widths.iter().map(|w| format!("{w}")).collect::<Vec<_>>(),
    );
    let breakpoint_source_file = breakpoint_source_file(&merged, &frameworks);

    let report = ScanReport {
        project_root: root.to_string_lossy().to_string(),
        project_name: name,
        frameworks,
        breakpoints: merged,
        dev_servers,
        conflicts,
        warnings,
        scanned_files: index.scanned,
        skipped_files: index.skipped,
        css_files,
        duration_ms: started.elapsed().as_millis() as u64,
        source_hash,
        breakpoint_source_file,
        log_path: None,
        truncated: index.truncated,
    };

    ScanOutcome { report, rows, log }
}

/// Which file to name when talking about where the breakpoints came from.
///
/// The file that produced the most of them wins. A tie breaks on the name, so
/// the answer does not move between runs of the same scan.
fn breakpoint_source_file(
    merged: &[types::BreakpointDiscovery],
    frameworks: &[types::FrameworkDetection],
) -> String {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for found in merged {
        if found.source_file.is_empty() {
            continue;
        }
        *counts.entry(found.source_file.as_str()).or_default() += 1;
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    ranked
        .first()
        .map(|(file, _)| file.to_string())
        .or_else(|| frameworks.first().map(|f| f.source_file.clone()))
        .unwrap_or_else(|| "css media queries".into())
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn friendly_source(source: &str) -> String {
    if source.starts_with(".lando") {
        "Lando".into()
    } else if source.contains("ddev") {
        "DDEV".into()
    } else if source.starts_with("vite.config") {
        "Vite".into()
    } else if source.contains("compose") {
        "Docker Compose".into()
    } else if source.starts_with("package.json") {
        "package.json script".into()
    } else {
        source.split(' ').next().unwrap_or(source).to_string()
    }
}

/// Collapse the same width found by several detectors down to one entry, best
/// source winning, and record where two sources disagree.
fn merge(found: Vec<BreakpointDiscovery>, log: &mut ScanLog) -> (Vec<BreakpointDiscovery>, Vec<Conflict>) {
    let mut sorted = found;
    sorted.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());

    let mut groups: Vec<Vec<BreakpointDiscovery>> = Vec::new();
    for entry in sorted {
        match groups.last_mut() {
            Some(group) if (entry.width - group[0].width).abs() <= CROSS_TOLERANCE => {
                group.push(entry)
            }
            _ => groups.push(vec![entry]),
        }
    }

    let mut merged: Vec<BreakpointDiscovery> = Vec::new();
    for group in groups {
        let mut best = group
            .iter()
            .max_by(|a, b| {
                a.kind
                    .cmp(&b.kind)
                    .then(a.confidence.partial_cmp(&b.confidence).unwrap())
            })
            .cloned()
            .unwrap();
        // Frequency is a CSS fact, and it is useful even when a framework name
        // won the entry, because it says how much the codebase leans on it.
        best.file_count = group.iter().map(|b| b.file_count).max().unwrap_or(1);
        if best.name.is_none() {
            best.name = group.iter().find_map(|b| b.name.clone());
        }
        merged.push(best);
    }

    let conflicts = find_conflicts(&merged, log);
    (merged, conflicts)
}

/// A conflict is two sources disagreeing about the same named breakpoint, or a
/// stylesheet that lays out at a width close to but not the same as the one the
/// framework declares. Both are worth a question, neither is worth a guess.
fn find_conflicts(merged: &[BreakpointDiscovery], log: &mut ScanLog) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    let mut by_name: BTreeMap<String, Vec<&BreakpointDiscovery>> = BTreeMap::new();
    for entry in merged {
        if let Some(name) = &entry.name {
            by_name.entry(name.to_ascii_lowercase()).or_default().push(entry);
        }
    }
    for (name, entries) in by_name {
        if entries.len() < 2 {
            continue;
        }
        for pair in entries.windows(2) {
            if (pair[0].width - pair[1].width).abs() <= CROSS_TOLERANCE {
                continue;
            }
            // One declaration with two edges is not two sources disagreeing.
            // Drupal's documented form, `(min-width: 560px) and (max-width:
            // 850px)`, is a single named breakpoint with a lower and an upper
            // bound, and reporting it asked people to resolve a disagreement
            // between a file and itself. Two entries from the same declaration
            // that sit on opposite edges are that case; two on the same edge
            // are a real disagreement and still count.
            let same_declaration =
                pair[0].source_file == pair[1].source_file && pair[0].source == pair[1].source;
            if same_declaration && pair[0].edge != pair[1].edge {
                continue;
            }
            conflicts.push(Conflict {
                name: name.clone(),
                left: side(pair[0]),
                right: side(pair[1]),
            });
        }
    }

    // A near miss is worth a question; a different decision is not.
    //
    // Five percent of 1536 is 76 pixels, so a deliberate 1470 used in two
    // files was flagged as disagreeing with the configured 1536. The absolute
    // cap keeps the case this was written for, a 992 laid out against a
    // configured 1024, and loses the ones that were never in doubt.
    const NEAR_MISS_CAP: f64 = 32.0;

    let mut already_flagged: Vec<f64> = Vec::new();
    for configured in merged.iter().filter(|b| b.kind == Kind::Configured) {
        for css in merged.iter().filter(|b| b.kind == Kind::Css) {
            let gap = (configured.width - css.width).abs();
            let window = (configured.width * 0.05).min(NEAR_MISS_CAP);
            // One CSS width could sit near two configured ones and produce a
            // card for each, all saying the same thing.
            if already_flagged.contains(&css.width) {
                continue;
            }
            if gap > CROSS_TOLERANCE && gap <= window && css.file_count >= 2 {
                already_flagged.push(css.width);
                log.note(format!(
                    "css lays out at {}px where the config says {}px",
                    css.width, configured.width
                ));
                conflicts.push(Conflict {
                    name: configured.name.clone().unwrap_or_else(|| format!("{}px", configured.width)),
                    left: side(configured),
                    right: side(css),
                });
            }
        }
    }

    conflicts
}

fn side(entry: &BreakpointDiscovery) -> ConflictSide {
    ConflictSide {
        label: match entry.kind {
            Kind::Configured | Kind::Framework => entry
                .name
                .clone()
                .unwrap_or_else(|| entry.source.clone()),
            _ => "CSS".into(),
        },
        width: entry.width,
        source_file: entry.source_file.clone(),
        detail: entry.source.clone(),
    }
}

/// Reopening a project should be instant, so a report is kept against the
/// mtimes of the files that could change it.
static CACHE: Mutex<Option<(String, String, ScanReport)>> = Mutex::new(None);

pub fn cache_key(root: &Path) -> String {
    let mut parts = Vec::new();
    for name in [
        "tailwind.config.js",
        "tailwind.config.ts",
        "tailwind.config.cjs",
        "tailwind.config.mjs",
        "package.json",
        ".lando.yml",
        "vite.config.js",
        "vite.config.ts",
        "breakpoints.md",
        ".breakpoints.json",
    ] {
        let path = root.join(name);
        if let Ok(meta) = std::fs::metadata(&path) {
            let stamp = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            parts.push(format!("{name}:{stamp}:{}", meta.len()));
        }
    }
    util::short_hash(&parts)
}

pub fn cached(root: &Path) -> Option<ScanReport> {
    let key = cache_key(root);
    let root = root.to_string_lossy().to_string();
    let cache = CACHE.lock().unwrap();
    match &*cache {
        Some((cached_root, cached_key, report)) if *cached_root == root && *cached_key == key => {
            Some(report.clone())
        }
        _ => None,
    }
}

pub fn remember(root: &Path, report: &ScanReport) {
    let key = cache_key(root);
    *CACHE.lock().unwrap() = Some((
        root.to_string_lossy().to_string(),
        key,
        report.clone(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(width: f64, file: &str) -> types::BreakpointDiscovery {
        types::BreakpointDiscovery {
            width,
            name: None,
            source: "test".into(),
            source_file: file.into(),
            line: None,
            confidence: 1.0,
            kind: types::Kind::Configured,
            edge: types::Edge::Min,
            file_count: 1,
        }
    }

    fn framework(file: &str) -> types::FrameworkDetection {
        types::FrameworkDetection {
            framework: "Tailwind CSS".into(),
            version: Some("4".into()),
            confidence: 1.0,
            source_file: file.into(),
            breakpoints: vec![],
            metadata: Default::default(),
        }
    }

    /// The case this exists for. Tailwind is detected first, from a stylesheet
    /// that declares no breakpoints, while every width came out of Drupal's
    /// yaml. Naming the stylesheet sent people to a file with nothing in it.
    #[test]
    fn the_file_named_is_the_one_the_widths_came_from() {
        let merged = vec![
            found(550.0, "web/themes/be_the_ray/be_the_ray.breakpoints.yml"),
            found(768.0, "web/themes/be_the_ray/be_the_ray.breakpoints.yml"),
            found(1024.0, "web/themes/be_the_ray/be_the_ray.breakpoints.yml"),
        ];
        let frameworks = vec![framework("web/themes/be_the_ray/css/styles.css")];
        assert_eq!(
            breakpoint_source_file(&merged, &frameworks),
            "web/themes/be_the_ray/be_the_ray.breakpoints.yml"
        );
    }

    #[test]
    fn the_file_that_produced_the_most_of_them_wins() {
        let merged = vec![
            found(550.0, "a.css"),
            found(768.0, "b.yml"),
            found(1024.0, "b.yml"),
        ];
        assert_eq!(breakpoint_source_file(&merged, &[]), "b.yml");
    }

    /// With nothing found, the framework is still better than nothing, and
    /// with no framework either there is still something to print.
    #[test]
    fn nothing_found_falls_back_rather_than_naming_an_empty_string() {
        assert_eq!(breakpoint_source_file(&[], &[framework("tailwind.css")]), "tailwind.css");
        assert_eq!(breakpoint_source_file(&[], &[]), "css media queries");
    }

    fn bp(width: f64, name: Option<&str>, kind: Kind, confidence: f64) -> BreakpointDiscovery {
        BreakpointDiscovery {
            width,
            name: name.map(str::to_string),
            source: "test".into(),
            source_file: "test".into(),
            line: None,
            confidence,
            kind,
            edge: Edge::Min,
            file_count: 1,
        }
    }

    #[test]
    fn a_configured_width_beats_the_same_width_found_in_css() {
        let mut log = ScanLog::new("t");
        let (merged, _) = merge(
            vec![
                bp(768.0, None, Kind::Css, 0.6),
                bp(768.0, Some("md"), Kind::Configured, 0.95),
            ],
            &mut log,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name.as_deref(), Some("md"));
        assert_eq!(merged[0].kind, Kind::Configured);
    }

    #[test]
    fn merging_keeps_how_widely_a_width_is_used() {
        let mut log = ScanLog::new("t");
        let mut css = bp(768.0, None, Kind::Css, 0.6);
        css.file_count = 9;
        let (merged, _) = merge(vec![css, bp(768.0, Some("md"), Kind::Configured, 0.95)], &mut log);
        assert_eq!(merged[0].file_count, 9);
    }

    #[test]
    fn two_sources_naming_the_same_breakpoint_differently_is_a_conflict() {
        let mut log = ScanLog::new("t");
        let (_, conflicts) = merge(
            vec![
                bp(1024.0, Some("lg"), Kind::Configured, 0.95),
                bp(992.0, Some("lg"), Kind::Css, 0.6),
            ],
            &mut log,
        );
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].name, "lg");
    }

    #[test]
    fn a_css_width_beside_a_configured_one_is_flagged_when_several_files_use_it() {
        let mut log = ScanLog::new("t");
        let mut css = bp(992.0, None, Kind::Css, 0.6);
        css.file_count = 4;
        let (_, conflicts) = merge(vec![bp(1024.0, Some("lg"), Kind::Configured, 0.95), css], &mut log);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].right.width, 992.0);
    }

    #[test]
    fn a_one_off_css_width_near_a_configured_one_is_not_worth_a_question() {
        let mut log = ScanLog::new("t");
        let (_, conflicts) = merge(
            vec![bp(1024.0, Some("lg"), Kind::Configured, 0.95), bp(992.0, None, Kind::Css, 0.6)],
            &mut log,
        );
        assert!(conflicts.is_empty());
    }
}
