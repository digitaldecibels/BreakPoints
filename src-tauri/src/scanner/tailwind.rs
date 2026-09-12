//! Tailwind, both configuration styles.
//!
//! v3 keeps breakpoints in `theme.screens` in a JavaScript config. v4 moved
//! them into CSS as `--breakpoint-*` custom properties in an `@theme` block,
//! which is far easier to read and cannot compute anything.

use std::collections::BTreeMap;

use regex::Regex;

use super::jsobj;
use super::log::ScanLog;
use super::types::{BreakpointDiscovery, DetectorOutput, Edge, FrameworkDetection, Kind, ScanWarning};
use super::units::parse_length;
use super::walk::{FileIndex, ReadBudget};

/// Tailwind's own screens, identical in v3 and v4. Used when a config extends
/// the defaults rather than replacing them.
pub const DEFAULT_SCREENS: &[(&str, f64)] = &[
    ("sm", 640.0),
    ("md", 768.0),
    ("lg", 1024.0),
    ("xl", 1280.0),
    ("2xl", 1536.0),
];

const CONFIG_NAMES: &[&str] = &[
    "tailwind.config.js",
    "tailwind.config.cjs",
    "tailwind.config.mjs",
    "tailwind.config.ts",
];

pub fn run(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> DetectorOutput {
    let mut out = DetectorOutput::default();

    let declared = declared_version(index, budget, log);
    let mut found_any = false;

    for name in CONFIG_NAMES {
        for rel in index.by_name(name) {
            let Some(text) = budget.read(index, rel, log) else { continue };
            let file = rel.to_string_lossy().to_string();
            if let Some(detection) = parse_v3(&text, &file, declared.as_deref(), log, &mut out.warnings) {
                found_any = true;
                out.breakpoints.extend(detection.breakpoints.clone());
                out.frameworks.push(detection);
            }
        }
    }

    // v4: the breakpoints are in CSS, so look for the import and the @theme.
    let mut declared_in: Vec<(String, String)> = Vec::new();
    let mut imported_by: Option<String> = None;

    // Any stylesheet can carry the `@theme` block, not only a `.css` one: a
    // PostCSS project writes `.pcss`.
    for rel in index.by_extension(super::css::STYLESHEETS) {
        // The deadline is checked here as well as in the walk. This detector
        // runs first and reads the whole stylesheet corpus, so it is the one
        // most likely to blow the budget and it was the one that could not see
        // it. On a network volume that is how a scan hangs.
        if index.out_of_time() {
            log.skip(&rel.to_string_lossy(), "scan timeout reached before this file");
            break;
        }
        let name = rel.to_string_lossy().to_string();
        // Anything already compiled is output, not source, and a hidden
        // directory is tooling: Tailwind's own probe writes a CSS file that
        // otherwise reads as a second config.
        if name.contains("dist/") || name.contains("build/") || super::css::in_hidden_directory(rel) {
            continue;
        }
        let Some(text) = budget.read(index, rel, log) else { continue };
        // A compiled bundle carries the framework's own `--breakpoint-*`
        // properties, so reading it would report Tailwind's defaults as if the
        // project had declared them.
        if super::css::looks_compiled(&text) {
            log.skip(&name, "one enormous line, so it is a compiled bundle rather than source");
            continue;
        }
        // Every stylesheet is looked at for `--breakpoint-*`, not only ones
        // that mention the framework.
        //
        // The documented v4 layout is an entry stylesheet that imports
        // tailwindcss and a separate theme file holding the block. The theme
        // file never mentions the framework, so requiring the marker skipped
        // it, and the entry file was then read as importing tailwindcss while
        // declaring nothing, which made the app report Tailwind's stock five
        // widths as if the project had chosen them.
        if text.contains("--breakpoint-") {
            declared_in.push((name.clone(), text.clone()));
        }
        if imports_tailwind(&text) && imported_by.is_none() {
            imported_by = Some(name.clone());
        }
    }

    // Declarations anywhere win. Only if there are none does importing the
    // framework mean the defaults are in force.
    if let Some((file, text)) = declared_in.first() {
        if let Some(detection) = parse_v4(text, file, log) {
            found_any = true;
            out.breakpoints.extend(detection.breakpoints.clone());
            out.frameworks.push(detection);
        }
        for (other, _) in declared_in.iter().skip(1) {
            log.note(format!("{other} also declares --breakpoint-* properties"));
        }
    } else if let Some(file) = &imported_by {
        if let Some(detection) = parse_v4("", file, log) {
            found_any = true;
            out.breakpoints.extend(detection.breakpoints.clone());
            out.frameworks.push(detection);
        }
    }

    if !found_any {
        if let Some(version) = declared {
            log.note(format!(
                "tailwindcss {version} is a dependency but no screens were resolved from any config"
            ));
            out.warnings.push(ScanWarning {
                file: "package.json".into(),
                message: format!("Tailwind {version} is installed but its breakpoints did not resolve"),
                detail: vec![
                    "No tailwind.config.* with a readable theme.screens, and no @theme block with --breakpoint-* properties.".into(),
                    "The CSS media query scan below is what Break/Points fell back to.".into(),
                ],
            });
        } else {
            log.note("no Tailwind config and no tailwindcss dependency");
        }
    }

    out
}

/// The version range from package.json, reduced to a major.
fn declared_version(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> Option<String> {
    for rel in index.by_name("package.json") {
        if index.out_of_time() {
            log.skip(&rel.to_string_lossy(), "scan timeout reached before this file");
            break;
        }
        // Only the project's own manifest, not one nested in a package.
        if rel.components().count() > 1 {
            continue;
        }
        let text = budget.read(index, rel, log)?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        for section in ["dependencies", "devDependencies"] {
            if let Some(range) = json
                .get(section)
                .and_then(|d| d.get("tailwindcss"))
                .and_then(|v| v.as_str())
            {
                let major: String = range
                    .trim_start_matches(['^', '~', '>', '=', 'v', ' '])
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                log.note(format!("package.json declares tailwindcss {range}"));
                return Some(if major.is_empty() { range.to_string() } else { major });
            }
        }
    }
    None
}

fn parse_v3(
    text: &str,
    file: &str,
    declared: Option<&str>,
    log: &mut ScanLog,
    warnings: &mut Vec<ScanWarning>,
) -> Option<FrameworkDetection> {
    let blanked: Vec<char> = jsobj::blank_comments(text).chars().collect();

    // Only `theme.screens` and `theme.extend.screens` are breakpoints. A
    // `screens` key anywhere else is a different setting wearing the same
    // name, and `theme.container.screens` is the one that turns up in real
    // configs: it holds the container's max-widths, its values are often the
    // word `none`, and reading it as breakpoints leaves the project with none
    // at all and nothing said about why.
    //
    // `theme.extend.screens` adds to Tailwind's defaults; `theme.screens`
    // replaces them. Getting that wrong loses five breakpoints or invents them.
    let theme_open = theme_object(&blanked);
    let mut sites: Vec<(usize, bool)> = Vec::new();
    if let Some(theme) = theme_open {
        if let Some(colon) = jsobj::child_key(&blanked, theme, "screens") {
            sites.push((colon, false));
        }
        if let Some(extend) = jsobj::child_key(&blanked, theme, "extend")
            .and_then(|colon| jsobj::object_after_colon(&blanked, colon))
        {
            if let Some(colon) = jsobj::child_key(&blanked, extend, "screens") {
                sites.push((colon, true));
            }
        }
    }

    if sites.is_empty() && jsobj::find_key(&blanked, "screens", 0).is_some() {
        log.discarded(
            file,
            "screens",
            "the only screens key is not theme.screens or theme.extend.screens, so it is a different setting",
        );
    }

    let mut breakpoints = Vec::new();
    let mut extends = false;
    let mut resolved_any = false;
    // Whether a `theme.screens` that replaces the defaults was actually read.
    let mut replaced_defaults = false;

    for (colon, inside_extend) in sites {
        match jsobj::object_after_colon(&blanked, colon) {
            Some(open) => {
                extends |= inside_extend;
                replaced_defaults |= !inside_extend;
                resolved_any = true;
                read_screens(&blanked, open, file, log, &mut breakpoints);
            }
            None => {
                let line = jsobj::line_of(&blanked, colon);
                let snippet: String = blanked[colon..(colon + 80).min(blanked.len())].iter().collect();
                log.parse_failure(
                    file,
                    Some(line),
                    &snippet,
                    "theme.screens value",
                    "screens is not an object literal, so it cannot be resolved without running the config",
                );
                warnings.push(ScanWarning {
                    file: file.to_string(),
                    message: format!("Could not read theme.screens in {file}"),
                    detail: vec![
                        format!("Line {line}: screens is not an object literal."),
                        "It is probably imported, spread from another module, or computed. Break/Points parses configs statically and never runs them, so this one cannot be resolved.".into(),
                        "The CSS media query scan is what covered this project instead.".into(),
                    ],
                });
            }
        }
    }

    // A screens object that resolved to nothing is the same situation as no
    // screens object at all: Tailwind's own defaults are what the site is
    // really built on, and saying nothing would leave the project with zero
    // breakpoints and no explanation.
    if resolved_any && breakpoints.is_empty() {
        resolved_any = false;
        log.discarded(
            file,
            "theme.screens",
            "the screens object held no readable widths, so Tailwind's defaults apply",
        );
        warnings.push(ScanWarning {
            file: file.to_string(),
            message: format!("theme.screens in {file} produced no widths"),
            detail: vec![
                "Every entry was a value Break/Points could not read as a length.".into(),
                "Tailwind's own default screens are being used instead, which is what the site renders with.".into(),
            ],
        });
    }

    if !resolved_any {
        log.note(format!("{file} has no readable theme.screens; Tailwind's defaults apply"));
    }

    // `theme.screens` replaces the defaults; `theme.extend.screens` adds to
    // whatever is in force. A config with both replaces and then adds, so the
    // defaults are gone either way. Reading the extend flag alone put all five
    // back, so a project that had deliberately cut down to three breakpoints
    // was shown eight, five of which it does not have.
    if (extends && !replaced_defaults) || !resolved_any {
        let named: Vec<String> = breakpoints
            .iter()
            .filter_map(|b: &BreakpointDiscovery| b.name.clone())
            .collect();
        for (name, width) in DEFAULT_SCREENS {
            if named.iter().any(|n| n == name) {
                continue;
            }
            breakpoints.push(BreakpointDiscovery {
                width: *width,
                name: Some((*name).to_string()),
                source: "Tailwind default screens".into(),
                source_file: file.to_string(),
                line: None,
                confidence: 0.8,
                kind: Kind::Framework,
                edge: Edge::Min,
                file_count: 1,
            });
        }
    }

    breakpoints.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());

    let version = declared
        .map(|v| v.to_string())
        .unwrap_or_else(|| "3".to_string());
    let mut metadata = BTreeMap::new();
    metadata.insert("config".into(), file.to_string());
    metadata.insert(
        "screens".into(),
        if extends { "theme.extend.screens".into() } else { "theme.screens".into() },
    );

    Some(FrameworkDetection {
        framework: "tailwind".into(),
        version: Some(version),
        confidence: if resolved_any { 0.95 } else { 0.6 },
        source_file: file.to_string(),
        breakpoints,
        metadata,
    })
}

/// The `{` of the config's `theme` object.
///
/// The first `theme:` in the file usually is it, but a plugin's own options
/// can carry one too, so keep looking until one turns out to be an object.
fn theme_object(src: &[char]) -> Option<usize> {
    let mut from = 0usize;
    while let Some(colon) = jsobj::find_key(src, "theme", from) {
        if let Some(open) = jsobj::object_after_colon(src, colon) {
            return Some(open);
        }
        from = colon + 1;
    }
    None
}

fn read_screens(
    src: &[char],
    open: usize,
    file: &str,
    log: &mut ScanLog,
    out: &mut Vec<BreakpointDiscovery>,
) {
    for entry in jsobj::entries(src, open) {
        let line = jsobj::line_of(src, entry.offset);

        if entry.key.is_empty() {
            log.parse_failure(
                file,
                Some(line),
                &entry.value,
                "theme.screens entry",
                "not a key and value; a spread or an imported identifier cannot be resolved statically",
            );
            continue;
        }

        // A plain string: the ordinary case.
        if let Some(px) = parse_length(&entry.value) {
            out.push(BreakpointDiscovery::configured(
                px,
                entry.key.clone(),
                "tailwind.config theme.screens",
                file,
                Some(line),
            ));
            continue;
        }

        // An object: `{ min: '768px' }`, `{ max: '767px' }`, `{ raw: '...' }`.
        if entry.value.starts_with('{') {
            let inner: Vec<char> = entry.value.chars().collect();
            let get = |key: &str| -> Option<String> {
                let colon = jsobj::find_key(&inner, key, 0)?;
                let mut i = colon + 1;
                while i < inner.len() && inner[i].is_whitespace() {
                    i += 1;
                }
                let start = i;
                while i < inner.len() && inner[i] != ',' && inner[i] != '}' {
                    i += 1;
                }
                Some(inner[start..i].iter().collect::<String>().trim().to_string())
            };

            if get("raw").is_some() {
                log.discarded(file, &entry.key, "a raw media query has no width component");
                continue;
            }
            if let Some(px) = get("min").as_deref().and_then(parse_length) {
                out.push(BreakpointDiscovery::configured(
                    px,
                    entry.key.clone(),
                    "tailwind.config theme.screens (min)",
                    file,
                    Some(line),
                ));
                continue;
            }
            if let Some(px) = get("max").as_deref().and_then(parse_length) {
                // A max-only screen names the range below the boundary, so the
                // pixel worth rendering is the one above it.
                out.push(BreakpointDiscovery {
                    width: px + 1.0,
                    name: Some(entry.key.clone()),
                    source: "tailwind.config theme.screens (max only)".into(),
                    source_file: file.to_string(),
                    line: Some(line),
                    confidence: 0.7,
                    kind: Kind::Configured,
                    edge: Edge::Max,
                    file_count: 1,
                });
                log.note(format!(
                    "{file}: screen '{}' is max-only, so its boundary is {} rather than {px}",
                    entry.key,
                    px + 1.0
                ));
                continue;
            }
            log.discarded(
                file,
                &entry.key,
                "object screen with no min or max width component",
            );
            continue;
        }

        log.parse_failure(
            file,
            Some(line),
            &format!("{}: {}", entry.key, entry.value),
            "theme.screens value",
            "value is neither a length nor an object; probably an identifier or a computed expression",
        );
    }
}

/// Whether a stylesheet pulls the framework in.
fn imports_tailwind(text: &str) -> bool {
    text.contains("@import \"tailwindcss\"") || text.contains("@import 'tailwindcss'")
}

/// Read a `@theme` block, or hand back the defaults when there is nothing to
/// read and the caller has established that the framework is in use.
fn parse_v4(text: &str, file: &str, log: &mut ScanLog) -> Option<FrameworkDetection> {
    let re = Regex::new(r"--breakpoint-([A-Za-z0-9_-]+)\s*:\s*([^;]+);").unwrap();

    let mut breakpoints = Vec::new();
    for caps in re.captures_iter(text) {
        let name = caps[1].to_string();
        let raw = caps[2].trim();
        let line = text[..caps.get(0).unwrap().start()].lines().count();
        match parse_length(raw) {
            Some(px) => breakpoints.push(BreakpointDiscovery::configured(
                px,
                name,
                "@theme --breakpoint-*",
                file,
                Some(line),
            )),
            None => log.discarded(
                file,
                &format!("--breakpoint-{name}: {raw}"),
                "value is not a length that converts to px",
            ),
        }
    }

    if breakpoints.is_empty() {
        log.note(format!(
            "{file} imports tailwindcss and no stylesheet in the project declares --breakpoint-*, so the v4 defaults apply"
        ));
        for (name, width) in DEFAULT_SCREENS {
            breakpoints.push(BreakpointDiscovery {
                width: *width,
                name: Some((*name).to_string()),
                source: "Tailwind v4 default screens".into(),
                source_file: file.to_string(),
                line: None,
                confidence: 0.8,
                kind: Kind::Framework,
                edge: Edge::Min,
                file_count: 1,
            });
        }
    }

    breakpoints.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());
    let mut metadata = BTreeMap::new();
    metadata.insert("theme".into(), file.to_string());

    Some(FrameworkDetection {
        framework: "tailwind".into(),
        version: Some("4".into()),
        confidence: 0.95,
        source_file: file.to_string(),
        breakpoints,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> (Vec<(String, f64)>, Vec<ScanWarning>) {
        let mut log = ScanLog::new("test");
        let mut warnings = Vec::new();
        let detection = parse_v3(src, "tailwind.config.js", Some("3"), &mut log, &mut warnings).unwrap();
        (
            detection
                .breakpoints
                .iter()
                .map(|b| (b.name.clone().unwrap_or_default(), b.width))
                .collect(),
            warnings,
        )
    }

    #[test]
    fn a_plain_screens_object_replaces_the_defaults() {
        let (found, _) = parse(
            r#"module.exports = { theme: { screens: { sm: '600px', lg: '1100px' } } }"#,
        );
        assert_eq!(found, vec![("sm".into(), 600.0), ("lg".into(), 1100.0)]);
    }

    #[test]
    fn the_containers_own_screens_are_not_breakpoints() {
        // theme.container.screens holds max-widths, and `none` is a legal
        // value there. Reading it as breakpoints left a real project with
        // none at all and nothing in the log to say why.
        let (found, warnings) = parse(
            r#"module.exports = { theme: {
                 container: { center: true, screens: { sm: 'none', md: 'none' } },
                 extend: { colors: { brand: '#123' } },
               } }"#,
        );
        let widths: Vec<f64> = found.iter().map(|f| f.1).collect();
        assert_eq!(widths, vec![640.0, 768.0, 1024.0, 1280.0, 1536.0]);
        assert!(warnings.is_empty(), "the defaults applying is not a warning");
    }

    #[test]
    fn a_screens_object_that_reads_as_nothing_falls_back_and_says_so() {
        let (found, warnings) = parse(
            r#"module.exports = { theme: { screens: { sm: 'none', md: 'none' } } }"#,
        );
        let widths: Vec<f64> = found.iter().map(|f| f.1).collect();
        assert_eq!(widths, vec![640.0, 768.0, 1024.0, 1280.0, 1536.0]);
        assert_eq!(warnings.len(), 1, "a silent zero is the bug being fixed");
    }

    #[test]
    fn extending_keeps_the_defaults_and_adds_to_them() {
        let (found, _) = parse(
            r#"module.exports = { theme: { extend: { screens: { '3xl': '1800px' } } } }"#,
        );
        let widths: Vec<f64> = found.iter().map(|f| f.1).collect();
        assert!(widths.contains(&640.0), "kept sm");
        assert!(widths.contains(&1536.0), "kept 2xl");
        assert!(widths.contains(&1800.0), "added 3xl");
        assert_eq!(widths.len(), 6);
    }

    #[test]
    fn rem_screens_convert_at_a_sixteen_pixel_root() {
        let (found, _) = parse(r#"{ theme: { screens: { md: '48rem' } } }"#);
        assert_eq!(found, vec![("md".into(), 768.0)]);
    }

    #[test]
    fn object_screens_resolve_from_their_min() {
        let (found, _) = parse(r#"{ theme: { screens: { tablet: { min: '768px' } } } }"#);
        assert_eq!(found, vec![("tablet".into(), 768.0)]);
    }

    #[test]
    fn a_max_only_screen_renders_at_the_pixel_above_its_range() {
        let (found, _) = parse(r#"{ theme: { screens: { phone: { max: '767px' } } } }"#);
        assert_eq!(found, vec![("phone".into(), 768.0)]);
    }

    #[test]
    fn a_raw_query_is_dropped_rather_than_guessed_at() {
        let (found, _) = parse(r#"{ theme: { screens: { print: { raw: 'print' }, md: '768px' } } }"#);
        assert_eq!(found, vec![("md".into(), 768.0)]);
    }

    #[test]
    fn a_config_that_cannot_be_read_statically_produces_a_warning_not_a_silence() {
        let (_, warnings) = parse(r#"const s = require('./screens'); module.exports = { theme: { screens: s } }"#);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("theme.screens"));
        assert!(warnings[0].detail.iter().any(|d| d.contains("never runs them")));
    }

    #[test]
    fn comments_around_a_screen_do_not_break_it() {
        let (found, _) = parse(
            "module.exports = {\n  theme: {\n    // the only one we use\n    screens: { md: '768px' /* tablet */ },\n  },\n}",
        );
        assert_eq!(found, vec![("md".into(), 768.0)]);
    }

    #[test]
    fn v4_reads_breakpoints_out_of_the_theme_block() {
        let mut log = ScanLog::new("test");
        let detection = parse_v4(
            "@import \"tailwindcss\";\n@theme {\n  --breakpoint-md: 48rem;\n  --breakpoint-wide: 1600px;\n}",
            "src/styles.css",
            &mut log,
        )
        .unwrap();
        assert_eq!(detection.version.as_deref(), Some("4"));
        let widths: Vec<f64> = detection.breakpoints.iter().map(|b| b.width).collect();
        assert_eq!(widths, vec![768.0, 1600.0]);
    }

    #[test]
    fn v4_with_no_overrides_falls_back_to_the_shipped_defaults() {
        let mut log = ScanLog::new("test");
        let detection = parse_v4("@import \"tailwindcss\";", "src/app.css", &mut log).unwrap();
        assert_eq!(detection.breakpoints.len(), 5);
    }
}
