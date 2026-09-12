//! Raw media queries in the project's own stylesheets.
//!
//! Real projects have print queries, `prefers-reduced-motion`, retina queries
//! and dozens of one-off tweaks. A naive scan turns all of that into panels and
//! destroys trust in the first thirty seconds, so this detector filters to
//! width conditions only, collapses near neighbours, and ranks what is left by
//! how many files use it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;

/// Every extension this detector treats as a stylesheet.
///
/// `pcss` and `postcss` are the convention in PostCSS and Tailwind setups, and
/// `less` is what a lot of older Drupal and WordPress themes are written in.
pub const STYLESHEETS: &[&str] = &["css", "scss", "sass", "pcss", "postcss", "less"];

/// Formats that keep their styles in a `<style>` block inside something else.
pub const COMPONENTS: &[&str] = &["vue", "svelte", "astro"];

/// Everything the CSS detector will open.
pub fn readable_extensions() -> Vec<&'static str> {
    STYLESHEETS.iter().chain(COMPONENTS.iter()).copied().collect()
}

/// Blank out comments, keeping every other character in place.
///
/// A query inside a comment used to be matched as a real one, which inflated
/// the file count, and the file count is the only confidence signal the CSS
/// side has. Offsets and newlines are preserved so reported line numbers stay
/// true.
///
/// `//` starts a comment in SCSS, Sass and Less but not in plain CSS, and it
/// also appears in the middle of an unquoted `url(http://...)`. So a line
/// comment is only recognised when the two slashes do not follow a colon,
/// which is what tells a protocol from a comment.
pub fn blank_comments(css: &str) -> String {
    let chars: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut quote: Option<char> = None;

    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            quote = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            while i < chars.len() {
                out.push(if chars[i] == '\n' { '\n' } else { ' ' });
                let ended = i > 0 && chars[i - 1] == '*' && chars[i] == '/';
                i += 1;
                if ended {
                    break;
                }
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' && chars.get(i.wrapping_sub(1)) != Some(&':') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The CSS out of a single-file component, or the whole text for a plain
/// stylesheet.
///
/// A component's template is markup, not CSS, and feeding it to the media
/// query parser would be noise at best. Only what is inside `<style>` counts.
pub fn stylesheet_part(rel: &std::path::Path, text: &str) -> String {
    let is_component = rel
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| COMPONENTS.contains(&ext))
        .unwrap_or(false);
    if !is_component {
        return text.to_string();
    }

    // Everything outside a `<style>` block becomes blank lines rather than
    // being cut out, so a line number in the result is the line number in the
    // file. A reported line that points at the wrong line is worse than no
    // line at all: it sends somebody to the wrong place with confidence.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>(.*?)</style>").unwrap());

    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    for caps in re.captures_iter(text) {
        let block = caps.get(1).unwrap();
        out.extend(text[at..block.start()].chars().map(blank_but_newlines));
        out.push_str(block.as_str());
        at = block.end();
    }
    out.extend(text[at..].chars().map(blank_but_newlines));
    out
}

/// Keep the newlines, drop everything else.
fn blank_but_newlines(c: char) -> char {
    if c == '\n' {
        '\n'
    } else {
        ' '
    }
}

use super::log::ScanLog;
use super::types::{BreakpointDiscovery, DetectorOutput, Edge, Kind};
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

/// Whether a path names something that is output, vendored or minified.
pub fn is_ignored_by_name(file: &str) -> bool {
    IGNORE_MARKERS.iter().any(|m| file.contains(m))
}

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
    // Tells that do not depend on the file being minified.
    //
    // The line-length test below only catches output that was minified, and a
    // build run without minification, which is every dev build and a plain
    // Tailwind CLI run, reads as source. For Tailwind that meant the generated
    // `--breakpoint-*` block was read back as if the project had declared it;
    // for everything else the compiled file and its own source both counted,
    // doubling the confidence of every width in the project.
    if text.contains("/*! tailwindcss v") || text.contains("sourceMappingURL=") {
        return true;
    }
    // Tailwind's output is full of its own internal custom properties. A
    // handwritten file might mention one or two; it does not contain dozens.
    if text.matches("--tw-").count() > 20 {
        return true;
    }

    // Small files are exempt whatever their shape, so a terse handwritten
    // stylesheet is never mistaken for a build artefact.
    if text.len() < 20_000 {
        return false;
    }
    text.len() / text.lines().count().max(1) > 400
}

/// Whether a `.css` file sits beside a source file it was plainly built from.
///
/// `app.css` next to `app.scss` is output, however it is formatted, and
/// reading both counts every width in it twice.
pub fn has_a_source_sibling(rel: &std::path::Path, index: &FileIndex) -> bool {
    if rel.extension().and_then(|e| e.to_str()) != Some("css") {
        return false;
    }
    let Some(stem) = rel.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    index.files.iter().any(|other| {
        other.file_stem().and_then(|s| s.to_str()) == Some(stem)
            && matches!(
                other.extension().and_then(|e| e.to_str()),
                Some("scss") | Some("sass") | Some("less") | Some("pcss") | Some("postcss")
            )
    })
}

pub fn run(index: &FileIndex, budget: &mut ReadBudget, log: &mut ScanLog) -> DetectorOutput {
    let mut out = DetectorOutput::default();
    let mut per_width: BTreeMap<i64, Occurrence> = BTreeMap::new();
    let mut files_read = 0usize;
    let mut root_override: Option<(String, String)> = None;

    // Read once, understand the whole project, then resolve.
    //
    // Resolution used to happen per file with only that file's own variables,
    // and the universal layout is one file that defines the widths and fifty
    // that use them. The defining file has no media queries at all, and every
    // file that does was discarded as depending on a value it did not define,
    // so those projects reported no breakpoints and fell back to generic
    // device sizes.
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut project_vars: BTreeMap<String, String> = BTreeMap::new();
    let mut project_maps: BTreeMap<String, f64> = BTreeMap::new();

    for rel in index.by_extension(&readable_extensions()) {
        let file = rel.to_string_lossy().to_string();
        if is_ignored_by_name(&file) {
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
        let Some(raw) = budget.read(index, rel, log) else { continue };
        // A component is markup with a `<style>` block in it, so the CSS is
        // pulled out before anything looks at it. Checking the whole file for
        // compiled output would reject a component with one long template
        // line.
        let text = blank_comments(&stylesheet_part(rel, &raw));
        if text.trim().is_empty() {
            continue;
        }
        if looks_compiled(&text) {
            log.skip(&file, "it reads as build output rather than something a person wrote");
            continue;
        }
        if has_a_source_sibling(rel, index) {
            log.skip(&file, "there is a source file of the same name beside it, so this is what was built from it");
            continue;
        }
        files_read += 1;

        if root_override.is_none() {
            if let Some(value) = overrides_root_font_size(&text) {
                root_override = Some((file.clone(), value));
            }
        }

        for (name, value) in scss_lengths(&text) {
            project_vars.entry(name).or_insert(value);
        }
        for (key, px, _) in scss_map_entries(&text) {
            project_maps.entry(key).or_insert(px);
        }
        sources.push((file, text));
    }

    for (file, text) in &sources {
        let file = file.clone();
        let text = text.as_str();
        // The file's own definitions win over the project's, which is what
        // SCSS itself does.
        let mut variables = project_vars.clone();
        variables.extend(scss_lengths(text));
        for (query, offset) in media_preludes(text) {
            let resolved = substitute_maps(&substitute(&query, &variables), &project_maps);
            let hits = widths_in_query(&resolved);
            if hits.is_empty() {
                if resolved.contains('$') || resolved.contains("#{") {
                    log.discarded(
                        &file,
                        query.trim(),
                        "media query depends on a value no file in the project defines",
                    );
                } else if resolved.contains("calc(") {
                    log.discarded(
                        &file,
                        query.trim(),
                        "the width is a calc() and is only known once the browser works it out",
                    );
                } else if resolved.contains("var(") {
                    log.discarded(
                        &file,
                        query.trim(),
                        "the width is a custom property, which a media query cannot resolve anyway",
                    );
                } else if resolved.contains("@container") || resolved.contains("container") {
                    log.discarded(
                        &file,
                        query.trim(),
                        "a container query is about an element, not the viewport",
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
                    name: None,
                    declared: false,
                });
                entry.files.insert(file.clone());
                entry.occurrences += 1;
                if hit.edge == Edge::Min {
                    entry.edge = Edge::Min;
                }
            }
        }

        // How a breakpoint map is actually used: through a mixin, by key.
        //
        // `@include media-breakpoint-up(md)` is Bootstrap, and every sass-mq
        // derivative has its own spelling of it. What each mixin does cannot be
        // known statically, but the key can: it is one the project itself
        // declared in its own map. Counting these is what gives a Bootstrap
        // project a real usage count instead of one occurrence from inside the
        // file that defines the mixin.
        for key in mixin_keys(text) {
            let Some(px) = project_maps.get(&key) else { continue };
            let entry = per_width.entry(*px as i64).or_insert_with(|| Occurrence {
                width: *px,
                edge: Edge::Min,
                files: BTreeSet::new(),
                first_file: file.clone(),
                first_line: 1,
                occurrences: 0,
                name: Some(key.clone()),
                declared: true,
            });
            entry.files.insert(file.clone());
            entry.occurrences += 1;
            entry.named(&key);
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
                name: None,
                declared: false,
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

    // No warning, on purpose.
    //
    // This used to warn that rem breakpoints might be wrong wherever a project
    // set the root font size, and the warning was itself wrong. Media Queries
    // Level 4 resolves relative units in a query against the initial font
    // size, never against anything a stylesheet declares, so `html {
    // font-size: 62.5% }` does not move `(min-width: 48rem)`: it stays at 768.
    // Converting at 16px is correct for every query boundary.
    //
    // The 62.5% technique is widespread, so this fired on a large share of real
    // projects and told people the numbers on screen might be wrong when they
    // were not, which is the most expensive thing a measuring instrument can
    // say. It is still worth noting in the log, because it does affect rem
    // lengths everywhere else in the CSS.
    if let Some((file, value)) = root_override {
        log.note(format!(
            "{file} sets the root font size to {value}. Breakpoint widths are unaffected: a media query resolves rem against the initial font size, not against this."
        ));
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
    /// The name the project gave this width, when it gave it one.
    name: Option<String>,
    /// True when the width came from a declared map of breakpoints rather than
    /// from counting media queries.
    declared: bool,
}

impl Occurrence {
    /// A width that appears in a breakpoint map was chosen, not observed.
    ///
    /// This used to be an empty function, so the name was taken and thrown
    /// away. Five deliberate, named breakpoints then arrived as five anonymous
    /// widths at the confidence of a one-off tweak, and the recommender, which
    /// ranks by how many files a width appears in, opened the three narrowest
    /// and merely offered the two that designs most often break at.
    fn named(&mut self, name: &str) {
        if self.name.is_none() && !name.is_empty() {
            self.name = Some(name.to_string());
        }
        self.declared = true;
    }
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
        // Lines are counted from one, the way an editor does. Counting the
        // lines before the match gives zero for a map at the top of a file.
        let line = css[..caps.get(0).unwrap().start()].lines().count() + 1;
        for pair in pair_re.captures_iter(&caps[2]) {
            if let Some(px) = parse_length(&pair[2]) {
                out.push((pair[1].to_string(), px.round(), line));
            }
        }
    }
    out
}

/// Every key a mixin was called with, such as the `md` in
/// `@include media-breakpoint-up(md)`.
///
/// The caller decides whether a key means anything, by looking it up in the
/// project's own breakpoint map. A key that is not in the map is somebody
/// else's mixin and is ignored.
fn mixin_keys(css: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"@include\s+[A-Za-z0-9_.-]+\s*\(\s*['"]?([A-Za-z0-9_-]+)['"]?\s*\)"#)
            .unwrap()
    });
    re.captures_iter(css)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// Resolve `map-get($breakpoints, md)` and `map.get($breakpoints, md)`.
///
/// How a project with a breakpoint map actually consumes it. Without this the
/// map was parsed, its widths recorded, and every query that used one was
/// thrown away as depending on something unknown.
fn substitute_maps(query: &str, maps: &BTreeMap<String, f64>) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r#"map[-.]get\s*\(\s*\$[A-Za-z0-9_-]+\s*,\s*['"]?([A-Za-z0-9_-]+)['"]?\s*\)"#)
            .unwrap()
    });
    let mut out = query.to_string();
    for caps in re.captures_iter(query) {
        if let Some(px) = maps.get(&caps[1]) {
            out = out.replace(&caps[0], &format!("{px}px"));
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
        let name = group.iter().find_map(|o| o.name.clone());
        let declared = group.iter().any(|o| o.declared);

        out.push(BreakpointDiscovery {
            width: representative,
            name: name.clone(),
            source: if declared {
                match &name {
                    Some(name) => format!("breakpoint map entry {name}"),
                    None => "breakpoint map entry".to_string(),
                }
            } else {
                format!(
                    "{} media {} in {} file{}",
                    occurrences,
                    if occurrences == 1 { "query" } else { "queries" },
                    files.len(),
                    if files.len() == 1 { "" } else { "s" }
                )
            },
            source_file: first.first_file.clone(),
            line: Some(first.first_line),
            // A width used across many files is a real breakpoint; one used
            // once is probably a tweak. That is the whole confidence signal,
            // and it does not apply to a width somebody wrote down on purpose
            // in a map of breakpoints: that one was chosen, not observed.
            confidence: if declared {
                0.9
            } else {
                (0.3 + 0.1 * files.len() as f64).min(0.85)
            },
            kind: if declared { Kind::Configured } else { Kind::Css },
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

    /// A query inside a comment is not a breakpoint, and it used to be counted
    /// as one, which inflated the only confidence signal this detector has.
    #[test]
    fn a_commented_out_query_is_not_a_query() {
        let css = "/* @media (min-width: 900px) { } */\n@media (min-width: 700px) { .a { color: red; } }\n// @media (min-width: 1100px) { }\n";
        let blanked = blank_comments(css);
        let found: Vec<f64> = media_preludes(&blanked)
            .into_iter()
            .flat_map(|(q, _)| widths_in_query(&q))
            .map(|hit| hit.boundary)
            .collect();
        assert_eq!(found, vec![700.0], "only the live one");
        assert_eq!(blanked.lines().count(), css.lines().count(), "line for line");
    }

    /// An unquoted URL has two slashes in it and is not a comment.
    #[test]
    fn a_url_survives_the_comment_blanker() {
        let css = "@font-face { src: url(http://example.test/f.woff2); }\n@media (min-width: 820px) { .a { color: red; } }\n";
        let blanked = blank_comments(css);
        assert!(blanked.contains("example.test"), "{blanked}");
        let found: Vec<f64> = media_preludes(&blanked)
            .into_iter()
            .flat_map(|(q, _)| widths_in_query(&q))
            .map(|hit| hit.boundary)
            .collect();
        assert_eq!(found, vec![820.0]);
    }

    /// A component is markup with a style block in it. Only the block is CSS,
    /// and a line number has to still mean the line in the file.
    #[test]
    fn a_component_gives_up_its_style_block_and_keeps_its_line_numbers() {
        let vue = "<template>\n  <div>hi</div>\n</template>\n\n<script>\n// @media (min-width: 999px) in a string\n</script>\n\n<style>\n@media (min-width: 760px) {\n  .a { display: flex; }\n}\n</style>\n";
        let css = stylesheet_part(std::path::Path::new("Card.vue"), vue);
        assert!(css.contains("min-width: 760px"));
        assert!(!css.contains("999px"), "the script block is not CSS");
        assert_eq!(
            css.lines().count(),
            vue.lines().count(),
            "line for line, so a reported line number is the real one"
        );
        let media_line = css.lines().position(|l| l.contains("760px")).unwrap();
        let real_line = vue.lines().position(|l| l.contains("760px")).unwrap();
        assert_eq!(media_line, real_line);
    }

    #[test]
    fn a_plain_stylesheet_is_returned_whole() {
        let css = "@media (min-width: 900px) { .a { color: red; } }";
        assert_eq!(stylesheet_part(std::path::Path::new("app.scss"), css), css);
    }

    /// The layout that used to find nothing: one file defines the widths,
    /// every other file uses them, and the defining file has no queries of its
    /// own to be found by.
    #[test]
    fn a_variable_defined_in_another_file_resolves() {
        let mut project = BTreeMap::new();
        project.insert("tablet-up".to_string(), "900px".to_string());
        let resolved = substitute("(min-width: $tablet-up)", &project);
        assert_eq!(widths_in_query(&resolved).len(), 1);
        assert_eq!(widths_in_query(&resolved)[0].boundary, 900.0);
    }

    /// How a project with a breakpoint map actually consumes it.
    #[test]
    fn a_map_lookup_resolves() {
        let mut maps = BTreeMap::new();
        maps.insert("lg".to_string(), 992.0);
        for query in [
            "(min-width: map-get($grid-breakpoints, lg))",
            "(min-width: map.get($grid-breakpoints, lg))",
            "(min-width: map-get($grid-breakpoints, 'lg'))",
        ] {
            let resolved = substitute_maps(query, &maps);
            let hits = widths_in_query(&resolved);
            assert_eq!(hits.len(), 1, "{query} -> {resolved}");
            assert_eq!(hits[0].boundary, 992.0, "{query}");
        }
        // A key the project never declared stays unresolved rather than
        // becoming a made-up width.
        let untouched = substitute_maps("(min-width: map-get($other, zz))", &maps);
        assert!(widths_in_query(&untouched).is_empty());
    }

    #[test]
    fn a_mixin_call_gives_up_its_key() {
        let keys = mixin_keys("@include media-breakpoint-up(md) { .a { display: flex; } }");
        assert_eq!(keys, vec!["md".to_string()]);
        assert_eq!(mixin_keys("@include mq('tablet') { }"), vec!["tablet".to_string()]);
        assert!(mixin_keys("@include button-variant($primary, $secondary) { }").is_empty(),
            "two arguments is not a breakpoint call");
    }

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
                    name: None,
                    declared: false,
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
