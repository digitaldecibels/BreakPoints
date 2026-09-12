//! Drupal's `*.breakpoints.yml`.
//!
//! This is the best data any detector gets: a human wrote the label and the
//! media query on purpose, in a format meant to be read. So the label becomes
//! the panel name directly, skipping the friendly-name band lookup that every
//! other source goes through.

use std::collections::BTreeMap;

use yaml_rust2::{Yaml, YamlLoader};

use super::log::ScanLog;
use super::types::{BreakpointDiscovery, DetectorOutput, Edge, FrameworkDetection, Kind};
use super::units::widths_in_query;
use super::walk::{FileIndex, ReadBudget};

pub fn run(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> DetectorOutput {
    let mut out = DetectorOutput::default();

    let Some(evidence) = looks_like_drupal(index, budget, log) else {
        log.note("no composer.json with drupal/core and no core/ under a docroot");
        return out;
    };
    log.note(format!("Drupal recognised from {evidence}"));

    let files = index.by_suffix(".breakpoints.yml");
    if files.is_empty() {
        log.note("Drupal project has no *.breakpoints.yml in any theme or module");
        return out;
    }

    let mut breakpoints = Vec::new();
    let mut themes = Vec::new();

    for rel in files {
        if index.out_of_time() {
            log.skip(&rel.to_string_lossy(), "scan timeout reached before this file");
            break;
        }
        let file = rel.to_string_lossy().to_string();
        let Some(text) = budget.read(index, rel, log) else { continue };
        let docs = match YamlLoader::load_from_str(&text) {
            Ok(docs) => docs,
            Err(err) => {
                log.parse_failure(&file, None, &text.chars().take(120).collect::<String>(), "yaml", &err.to_string());
                continue;
            }
        };
        let Some(Yaml::Hash(root)) = docs.first() else {
            log.discarded(&file, "<document>", "top level is not a mapping");
            continue;
        };

        if let Some(stem) = rel.file_name().and_then(|n| n.to_str()) {
            let theme = stem.trim_end_matches(".breakpoints.yml").to_string();
            if !themes.contains(&theme) {
                themes.push(theme);
            }
        }

        for (key, value) in root.iter() {
            let key_name = key.as_str().unwrap_or("<key>").to_string();
            let Yaml::Hash(entry) = value else {
                log.discarded(&file, &key_name, "entry is not a mapping");
                continue;
            };
            let get = |name: &str| {
                entry
                    .get(&Yaml::String(name.to_string()))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let query = get("mediaQuery").unwrap_or_default();
            if query.trim().is_empty() {
                // The base breakpoint in a Drupal theme usually has none, and
                // it means "everything below the first one", not a width.
                log.discarded(&file, &key_name, "mediaQuery is empty, so it names no width");
                continue;
            }
            let hits = widths_in_query(&query);
            if hits.is_empty() {
                log.discarded(
                    &file,
                    &format!("{key_name}: {query}"),
                    "media query has no min-width or max-width component",
                );
                continue;
            }
            // The label is written for people, so it beats anything generated.
            let label = get("label").unwrap_or_else(|| {
                key_name.rsplit('.').next().unwrap_or(&key_name).to_string()
            });
            // Drupal's own documented form is two-sided: `all and (min-width:
            // 560px) and (max-width: 850px)` is one named breakpoint with two
            // boundaries. Giving both the same label produced two panels with
            // identical names, and made the merge report a conflict between a
            // file and itself.
            let two_sided = hits.len() > 1;
            for hit in hits {
                let name = if two_sided && hit.edge == Edge::Max {
                    format!("{label} upper")
                } else {
                    label.clone()
                };
                breakpoints.push(BreakpointDiscovery {
                    width: hit.boundary,
                    name: Some(name),
                    source: format!("Drupal breakpoint {key_name}"),
                    source_file: file.clone(),
                    line: None,
                    confidence: 0.95,
                    kind: Kind::Configured,
                    edge: hit.edge,
                    file_count: 1,
                });
            }
        }
    }

    if breakpoints.is_empty() {
        return out;
    }

    breakpoints.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());
    breakpoints.dedup_by(|a, b| a.width == b.width && a.name == b.name);

    let mut metadata = BTreeMap::new();
    metadata.insert("themes".into(), themes.join(", "));

    out.breakpoints = breakpoints.clone();
    out.frameworks.push(FrameworkDetection {
        framework: "drupal".into(),
        version: None,
        confidence: 0.95,
        source_file: breakpoints[0].source_file.clone(),
        breakpoints,
        metadata,
    });
    out
}

/// Two independent signals, because a composer.json can be missing from a
/// sub-tree and a docroot can be named either of two things.
fn looks_like_drupal(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> Option<String> {
    for rel in index.by_name("composer.json") {
        if index.out_of_time() {
            log.skip(&rel.to_string_lossy(), "scan timeout reached before this file");
            break;
        }
        if rel.components().count() > 1 {
            continue;
        }
        if let Some(text) = budget.read(index, rel, log) {
            if text.contains("\"drupal/core") {
                return Some("composer.json requiring drupal/core".into());
            }
        }
    }
    for docroot in ["web", "docroot"] {
        if index.absolute(std::path::Path::new(docroot)).join("core").is_dir() {
            return Some(format!("{docroot}/core"));
        }
    }
    if index.absolute(std::path::Path::new("core")).join("lib").is_dir() {
        return Some("core/lib".into());
    }
    None
}

/// Drupal names its own breakpoints well enough that the friendly-name band
/// lookup should be skipped for them.
pub fn is_drupal_source(source: &str) -> bool {
    source.starts_with("Drupal breakpoint")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(yaml: &str) -> Vec<(String, f64)> {
        let docs = YamlLoader::load_from_str(yaml).unwrap();
        let Some(Yaml::Hash(root)) = docs.first() else { return vec![] };
        let mut out = Vec::new();
        for (key, value) in root.iter() {
            let Yaml::Hash(entry) = value else { continue };
            let query = entry
                .get(&Yaml::String("mediaQuery".into()))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let label = entry
                .get(&Yaml::String("label".into()))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| key.as_str().unwrap_or("").to_string());
            for hit in widths_in_query(query) {
                out.push((label.clone(), hit.boundary));
            }
        }
        out
    }

    #[test]
    fn a_theme_breakpoints_file_yields_its_own_labels() {
        let found = parse_one(
            "mytheme.narrow:\n  label: narrow\n  mediaQuery: 'all and (min-width: 560px)'\n  weight: 1\nmytheme.wide:\n  label: wide\n  mediaQuery: 'all and (min-width: 851px)'\n",
        );
        assert_eq!(
            found,
            vec![("narrow".into(), 560.0), ("wide".into(), 851.0)]
        );
    }

    #[test]
    fn the_base_breakpoint_with_no_media_query_is_not_a_width() {
        let found = parse_one("mytheme.mobile:\n  label: mobile\n  mediaQuery: ''\n");
        assert!(found.is_empty());
    }
}
