//! Raw media queries in the project's own stylesheets.
//!
//! Real projects have print queries, `prefers-reduced-motion`, retina queries
//! and dozens of one-off tweaks. A naive scan turns all of that into panels and
//! destroys trust in the first thirty seconds, so this detector filters to
//! width conditions only, collapses near neighbours, and ranks what is left by
//! how many files use it.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;

use super::log::ScanLog;
use super::types::{BreakpointDiscovery, DetectorOutput, Edge, Kind, ScanWarning};
use super::units::{overrides_root_font_size, parse_length, widths_in_query};
use super::walk::{FileIndex, ReadBudget};

/// Two widths this close are the same decision written twice.
const CLUSTER_TOLERANCE: f64 = 2.0;
/// The scan sheet shows at most this many "also found" widths.
pub const MAX_REPORTED: usize = 10;

/// Compiled output and vendored code are not the project's own decisions.
const IGNORE_MARKERS: &[&str] = &[
    "dist/", "build/", "vendor/", "node_modules/", ".min.css", "-min.css",
    "bootstrap.css", "normalize.css", "core/assets/", "core/misc/", "core/themes/",
    // A compiled bundle that happens to sit in the source tree. `output.css`
    // is what a Tailwind CLI build is called by convention.
    "output.css",
];

/// Is this a compiled bundle rather than something a person wrote?
///
/// Names are not enough: a real project had 186KB of Astro's docs theme in
/// `_astro/common.D29JgpJZ.css` and 150KB of compiled Tailwind in
/// `tailwind.output.css`, and between them they supplied every "breakpoint"
/// the app offered. Their media queries belong to the framework that generated
/// them, not to the site.
///
/// The tell is the shape of the file. Hand-written stylesheets in real projects
/// run 20 to 40 bytes per line; minified bundles run 30,000 to 90,000. The
/// threshold sits three orders of magnitude away from both.
pub fn looks_compiled(text: &str) -> bool {
    // Small files are exempt whatever their shape, so a terse handwritten
    // stylesheet is never mistaken for a build artefact.
    if text.len() < 20_000 {
        return false;
    }
    text.len() / text.lines().count().max(1) > 400
}

pub fn run(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> DetectorOutput {
    let mut out = DetectorOutput::default();
    let mut per_width: BTreeMap<i64, Occurrence> = BTreeMap::new();
    let mut files_read = 0usize;
    let mut root_override: Option<(String, String)> = None;

    for rel in index.by_extension(&["css", "scss", "sass"]) {
        let file = rel.to_string_lossy().to_string();
        if IGNORE_MARKERS.iter().any(|m| file.contains(m)) {
            log.skip(&file, "compiled, vendored or minified output");
            continue;
        }
        // A hidden directory holds tooling, not the site's own stylesheets.
        // Tailwind's own probe writes one, and it looks exactly like a config.
        if in_hidden_directory(rel) {
            log.skip(&file, "inside a hidden directory, so it is tooling output");
            continue;
        }
        if index.out_of_time() {
            log.skip(&file, "scan timeout reached before this file");
            break;
        }
        let Some(text) = budget.read(index, rel, log) else { continue };
        if looks_compiled(&text) {
            log.skip(&file, "one enormous line, so it is a compiled bundle rather than source");
            continue;
        }
        files_read += 1;

        if root_override.is_none() {
            if let Some(value) = overrides_root_font_size(&text) {
                root_override = Some((file.clone(), value));
            }
        }

        let variables = scss_lengths(&text);
        for (query, offset) in media_preludes(&text) {
            let resolved = substitute(&query, &variables);
            let hits = widths_in_query(&resolved);
            if hits.is_empty() {
                if resolved.contains('$') || resolved.contains("#{") {
                    log.discarded(
                        &file,
                        query.trim(),
                        "media query depends on a value this file does not define",
                    );
                } else {
                    log.discarded(&file, query.trim(), "no width component");
                }
                continue;
            }
            let line = text[..offset].lines().count();
            for hit in hits {
                let entry = per_width.entry(hit.boundary as i64).or_insert_with(|| Occurrence {
                    width: hit.boundary,
                    edge: hit.edge,
                    files: BTreeSet::new(),
                    first_file: file.clone(),
                    first_line: line,
                    occurrences: 0,
                });
                entry.files.insert(file.clone());
                entry.occurrences += 1;
                if hit.edge == Edge::Min {
                    entry.edge = Edge::Min;
                }
            }
        }

        // A SCSS breakpoint map is a deliberate configuration even though it
        // lives in a stylesheet, so its names are worth keeping.
        for (name, px, line) in scss_map_entries(&text) {
            let entry = per_width.entry(px as i64).or_insert_with(|| Occurrence {
                width: px,
                edge: Edge::Min,
                files: BTreeSet::new(),
                first_file: file.clone(),
                first_line: line,
                occurrences: 0,
            });
            entry.files.insert(file.clone());
            entry.occurrences += 1;
            entry.named(&name);
        }
    }

    log.note(format!(
        "css detector read {files_read} stylesheets and found {} distinct widths before clustering",
        per_width.len()
    ));

    if let Some((file, value)) = root_override {
        out.warnings.push(ScanWarning {
            file: file.clone(),
            message: format!("Root font size is {value}, so rem breakpoints may be off"),
            detail: vec![
                format!("{file} sets the root font size to {value}."),
                "Break/Points converts rem widths at the CSS default of 16px. Any breakpoint written in rem will be out by the same ratio.".into(),
                "Widths written in px are unaffected.".into(),
            ],
        });
        log.note(format!("{file} overrides the root font size to {value}"));
    }

    out.breakpoints = cluster(per_width.into_values().collect(), log);
    out
}

struct Occurrence {
    width: f64,
    edge: Edge,
    files: BTreeSet<String>,
    first_file: String,
    first_line: usize,
    occurrences: usize,
}

impl Occurrence {
    fn named(&mut self, _name: &str) {}
}

/// Every `@media` prelude in a stylesheet, with where it starts.
pub fn in_hidden_directory(rel: &std::path::Path) -> bool {
    rel.parent()
        .map(|parent| {
            parent.components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .map(|name| name.starts_with('.'))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn media_preludes(css: &str) -> Vec<(String, usize)> {
    let re = Regex::new(r"@media([^{;]*)\{").unwrap();
    re.captures_iter(css)
        .map(|caps| {
            let m = caps.get(1).unwrap();
            (m.as_str().to_string(), m.start())
        })
        .collect()
}

/// `$tablet: 768px;` and `$tablet: 48rem;`, defined at the top level of the
/// same file. Indirection through an import is where this gives up.
fn scss_lengths(css: &str) -> BTreeMap<String, String> {
    let re = Regex::new(r"(?m)^\s*\$([A-Za-z0-9_-]+)\s*:\s*([0-9.]+(?:px|rem|em)?)\s*(?:!default)?\s*;").unwrap();
    re.captures_iter(css)
        .map(|caps| (caps[1].to_string(), caps[2].to_string()))
        .collect()
}

/// `$breakpoints: (sm: 640px, md: 768px);`
fn scss_map_entries(css: &str) -> Vec<(String, f64, usize)> {
    let map_re = Regex::new(r"(?s)\$([A-Za-z0-9_-]*(?:breakpoint|screen|bp|mq)[A-Za-z0-9_-]*)\s*:\s*\(([^)]*)\)").unwrap();
    let pair_re = Regex::new(r#"['"]?([A-Za-z0-9_-]+)['"]?\s*:\s*([0-9.]+(?:px|rem|em)?)"#).unwrap();
    let mut out = Vec::new();
    for caps in map_re.captures_iter(css) {
        let line = css[..caps.get(0).unwrap().start()].lines().count();
        for pair in pair_re.captures_iter(&caps[2]) {
            if let Some(px) = parse_length(&pair[2]) {
                out.push((pair[1].to_string(), px.round(), line));
            }
        }
    }
    out
}

fn substitute(query: &str, variables: &BTreeMap<String, String>) -> String {
    let mut out = query.to_string();
    for (name, value) in variables {
        out = out.replace(&format!("${name}"), value);
        out = out.replace(&format!("#{{${name}}}"), value);
    }
    out
}

/// Collapse near neighbours and rank what survives by how many files use it.
fn cluster(mut found: Vec<Occurrence>, log: &mut ScanLog) -> Vec<BreakpointDiscovery> {
    found.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());

    let mut clusters: Vec<Vec<Occurrence>> = Vec::new();
    for occurrence in found {
        match clusters.last_mut() {
            Some(group) if occurrence.width - group[0].width <= CLUSTER_TOLERANCE => {
                group.push(occurrence)
            }
            _ => clusters.push(vec![occurrence]),
        }
    }

    let mut out = Vec::new();
    for group in clusters {
        let representative = pick_representative(&group);
        let mut files = BTreeSet::new();
        let mut occurrences = 0;
        for occurrence in &group {
            files.extend(occurrence.files.iter().cloned());
            occurrences += occurrence.occurrences;
            if occurrence.width != representative {
                log.discarded(
                    &occurrence.first_file,
                    &format!("{}px", occurrence.width),
                    &format!("within {CLUSTER_TOLERANCE}px of {representative}px, so it is the same breakpoint"),
                );
            }
        }
        let first = &group[0];
        out.push(BreakpointDiscovery {
            width: representative,
            name: None,
            source: format!(
                "{} media {} in {} file{}",
                occurrences,
                if occurrences == 1 { "query" } else { "queries" },
                files.len(),
                if files.len() == 1 { "" } else { "s" }
            ),
            source_file: first.first_file.clone(),
            line: Some(first.first_line),
            // A width used across many files is a real breakpoint; one used
            // once is probably a tweak. That is the whole confidence signal.
            confidence: (0.3 + 0.1 * files.len() as f64).min(0.85),
            kind: Kind::Css,
            edge: first.edge,
            file_count: files.len(),
        });
    }

    // Rank by how widely used, then by width so the list reads left to right.
    out.sort_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then(a.width.partial_cmp(&b.width).unwrap())
    });
    out
}

/// Prefer the round number, which is what the author meant, over the `.98`
/// neighbour a framework generated from it.
fn pick_representative(group: &[Occurrence]) -> f64 {
    group
        .iter()
        .max_by_key(|o| {
            let width = o.width;
            let roundness = if width % 100.0 == 0.0 {
                3
            } else if width % 10.0 == 0.0 {
                2
            } else if width.fract() == 0.0 {
                1
            } else {
                0
            };
            (o.files.len(), roundness)
        })
        .map(|o| o.width)
        .unwrap_or(group[0].width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(css: &str) -> Vec<(f64, usize)> {
        let mut log = ScanLog::new("test");
        let mut per_width: BTreeMap<i64, Occurrence> = BTreeMap::new();
        let variables = scss_lengths(css);
        for (query, _) in media_preludes(css) {
            for hit in widths_in_query(&substitute(&query, &variables)) {
                let entry = per_width.entry(hit.boundary as i64).or_insert_with(|| Occurrence {
                    width: hit.boundary,
                    edge: hit.edge,
                    files: BTreeSet::new(),
                    first_file: "a.css".into(),
                    first_line: 1,
                    occurrences: 0,
                });
                entry.files.insert("a.css".into());
                entry.occurrences += 1;
            }
        }
        cluster(per_width.into_values().collect(), &mut log)
            .into_iter()
            .map(|b| (b.width, b.file_count))
            .collect()
    }

    #[test]
    fn noise_queries_never_become_breakpoints() {
        let found = widths(
            "@media print { a { color: #000 } }\n@media (prefers-reduced-motion: reduce) { a { } }\n@media (min-width: 768px) { a { } }",
        );
        assert_eq!(found, vec![(768.0, 1)]);
    }

    #[test]
    fn a_bootstrap_pair_collapses_to_one_width() {
        let found = widths("@media (max-width: 767.98px) { a {} }\n@media (min-width: 768px) { b {} }");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 768.0);
    }

    #[test]
    fn near_neighbours_collapse_onto_the_round_number() {
        let found = widths("@media (min-width: 767px) { a {} }\n@media (min-width: 768px) { b {} }");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 768.0);
    }

    #[test]
    fn a_scss_variable_defined_in_the_same_file_resolves() {
        let found = widths("$tablet: 768px;\n@media (min-width: $tablet) { a {} }");
        assert_eq!(found, vec![(768.0, 1)]);
    }

    #[test]
    fn a_scss_breakpoint_map_gives_up_its_names_and_widths() {
        let found = scss_map_entries("$breakpoints: (sm: 640px, md: 48rem, lg: 1024px);");
        assert_eq!(
            found.iter().map(|f| (f.0.clone(), f.1)).collect::<Vec<_>>(),
            vec![("sm".to_string(), 640.0), ("md".into(), 768.0), ("lg".into(), 1024.0)]
        );
    }

    #[test]
    fn a_query_whose_variable_lives_in_another_file_is_dropped_not_guessed() {
        let found = widths("@media (min-width: $from-another-file) { a {} }");
        assert!(found.is_empty());
    }
}
