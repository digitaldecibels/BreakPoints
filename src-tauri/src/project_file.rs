//! `breakpoints.md`: the normalised, human-readable, Git-friendly project
//! config. It lives in the project root, commits with the repo, and once it
//! exists it is the source of truth. Detection results become a suggestion.
//!
//! Two rules govern writing it. Prose is preserved, because people document
//! their reasoning around the table. And a file that will not parse is kept
//! exactly as it is, because losing someone's config to a parser bug is worse
//! than showing a warning chip.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::Viewport;
use crate::util;

pub const MARKDOWN_NAME: &str = "breakpoints.md";
pub const JSON_NAME: &str = ".breakpoints.json";

const BEGIN: &str = "<!-- breakpoints:config";
const END: &str = "-->";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub viewports: Vec<Viewport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom_to_fit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<String>,
}

/// Read whichever project file exists, Markdown first.
pub fn read(root: &Path) -> Option<Result<ProjectFile, String>> {
    let markdown = root.join(MARKDOWN_NAME);
    if markdown.is_file() {
        return Some(match std::fs::read_to_string(&markdown) {
            Ok(text) => parse_markdown(&text),
            Err(err) => Err(format!("could not read {MARKDOWN_NAME}: {err}")),
        });
    }
    let json = root.join(JSON_NAME);
    if json.is_file() {
        return Some(match std::fs::read_to_string(&json) {
            Ok(text) => serde_json::from_str::<ProjectFile>(&text)
                .map_err(|e| format!("{JSON_NAME} did not parse: {e}")),
            Err(err) => Err(format!("could not read {JSON_NAME}: {err}")),
        });
    }
    None
}

pub fn exists(root: &Path) -> bool {
    root.join(MARKDOWN_NAME).is_file() || root.join(JSON_NAME).is_file()
}

pub fn parse_markdown(text: &str) -> Result<ProjectFile, String> {
    let mut file = ProjectFile::default();

    if let Some((start, end)) = table_range(text) {
        for line in text.lines().skip(start + 2).take(end - start - 1) {
            let cells = row_cells(line);
            if cells.len() < 3 {
                continue;
            }
            let name = cells[0].clone();
            let (Ok(width), Ok(height)) = (parse_number(&cells[1]), parse_number(&cells[2])) else {
                continue;
            };
            let source = cells.get(3).cloned().unwrap_or_else(|| "custom".into());
            let mut viewport = Viewport::new(name, width, height, source);
            if let Some(reference) = cells.get(4) {
                if !reference.is_empty() && reference != "-" {
                    viewport.reference = Some(reference.clone());
                }
            }
            file.viewports.push(viewport);
        }
    }

    if file.viewports.is_empty() {
        return Err("no table with Name, Width and Height columns".into());
    }

    for (key, value) in config_block(text) {
        match key.as_str() {
            "url" => file.url = Some(value),
            "zoomToFit" => file.zoom_to_fit = Some(value == "true"),
            "scrollSync" => file.scroll_sync = Some(value == "true"),
            "source" => file.source = Some(value),
            "sourceHash" => file.source_hash = Some(value),
            "generated" => file.generated = Some(value),
            _ => {}
        }
    }

    Ok(file)
}

/// Rewrite the table and the config block, leaving every other line alone.
pub fn render_markdown(existing: Option<&str>, file: &ProjectFile) -> String {
    let table = render_table(&file.viewports);
    let block = render_config_block(file);

    let Some(existing) = existing else {
        return format!("# Breakpoints\n\n{table}\n{block}\n");
    };

    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();

    // The config block first, because replacing the table shifts line numbers.
    if let Some((start, end)) = block_range(existing) {
        lines.splice(start..=end, block.lines().map(str::to_string));
    } else {
        lines.push(String::new());
        lines.extend(block.lines().map(str::to_string));
    }

    let rebuilt = lines.join("\n");
    let mut lines: Vec<String> = rebuilt.lines().map(str::to_string).collect();
    if let Some((start, end)) = table_range(&rebuilt) {
        lines.splice(start..=end, table.lines().map(str::to_string));
    } else {
        let insert_at = lines
            .iter()
            .position(|l| l.trim_start().starts_with(BEGIN))
            .unwrap_or(lines.len());
        let mut block_lines: Vec<String> = table.lines().map(str::to_string).collect();
        block_lines.push(String::new());
        lines.splice(insert_at..insert_at, block_lines);
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Write `breakpoints.md`, keeping whatever prose is already in it.
pub fn write(root: &Path, file: &ProjectFile) -> Result<std::path::PathBuf, String> {
    let path = root.join(MARKDOWN_NAME);
    let existing = std::fs::read_to_string(&path).ok();
    let body = render_markdown(existing.as_deref(), file);
    std::fs::write(&path, body).map_err(|e| format!("could not write {MARKDOWN_NAME}: {e}"))?;
    Ok(path)
}

fn render_table(viewports: &[Viewport]) -> String {
    let with_references = viewports.iter().any(|v| v.reference.is_some());
    let mut out = String::new();
    if with_references {
        out.push_str("| Name | Width | Height | Source | Reference |\n");
        out.push_str("|------|------:|-------:|--------|-----------|\n");
    } else {
        out.push_str("| Name | Width | Height | Source |\n");
        out.push_str("|------|------:|-------:|--------|\n");
    }
    for viewport in viewports {
        out.push_str(&format!(
            "| {} | {} | {} | {} |",
            viewport.name,
            viewport.width as i64,
            viewport.height as i64,
            viewport.source
        ));
        if with_references {
            out.push_str(&format!(
                " {} |",
                viewport.reference.clone().unwrap_or_else(|| "-".into())
            ));
        }
        out.push('\n');
    }
    out
}

fn render_config_block(file: &ProjectFile) -> String {
    let mut out = String::from(BEGIN);
    out.push('\n');
    if let Some(url) = &file.url {
        out.push_str(&format!("url: {url}\n"));
    }
    out.push_str(&format!(
        "zoomToFit: {}\n",
        file.zoom_to_fit.unwrap_or(true)
    ));
    out.push_str(&format!(
        "scrollSync: {}\n",
        file.scroll_sync.unwrap_or(true)
    ));
    if let Some(source) = &file.source {
        out.push_str(&format!("source: {source}\n"));
    }
    if let Some(hash) = &file.source_hash {
        out.push_str(&format!("sourceHash: {hash}\n"));
    }
    out.push_str(&format!(
        "generated: {}\n",
        file.generated.clone().unwrap_or_else(util::today_iso)
    ));
    out.push_str(END);
    out
}

/// Line indices of the first table whose header names Name, Width and Height.
fn table_range(text: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let header: Vec<String> = row_cells(line)
            .into_iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let names = ["name", "width", "height"];
        if !names.iter().all(|n| header.iter().any(|h| h == n)) {
            continue;
        }
        // The separator row has to be next, or this is prose that happens to
        // use pipes.
        if lines
            .get(index + 1)
            .map(|l| !l.contains('-') || !l.trim_start().starts_with('|'))
            .unwrap_or(true)
        {
            continue;
        }
        let mut end = index + 1;
        while lines
            .get(end + 1)
            .map(|l| l.trim_start().starts_with('|'))
            .unwrap_or(false)
        {
            end += 1;
        }
        return Some((index, end));
    }
    None
}

fn block_range(text: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| l.trim_start().starts_with(BEGIN))?;
    let end = lines
        .iter()
        .skip(start)
        .position(|l| l.trim() == END)
        .map(|offset| start + offset)?;
    Some((start, end))
}

fn config_block(text: &str) -> Vec<(String, String)> {
    let Some((start, end)) = block_range(text) else {
        return Vec::new();
    };
    text.lines()
        .skip(start + 1)
        .take(end - start - 1)
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn row_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_start_matches('|').trim_end_matches('|');
    trimmed.split('|').map(|c| c.trim().to_string()).collect()
}

fn parse_number(cell: &str) -> Result<f64, ()> {
    cell.trim()
        .trim_end_matches("px")
        .trim()
        .parse::<f64>()
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# Breakpoints

These come from the Tailwind config. Do not edit by hand unless you mean it.

| Name | Width | Height | Source |
|------|------:|-------:|--------|
| Mobile | 640 | 850 | sm |
| Tablet | 768 | 1020 | md |

Some notes underneath, which must survive a rewrite.

<!-- breakpoints:config
url: http://localhost:5173
zoomToFit: true
scrollSync: false
source: tailwind.config.js
sourceHash: 8f2a91c4
generated: 2026-09-09
-->
"#;

    #[test]
    fn the_table_and_the_config_block_both_come_back() {
        let file = parse_markdown(SAMPLE).unwrap();
        assert_eq!(file.viewports.len(), 2);
        assert_eq!(file.viewports[1].name, "Tablet");
        assert_eq!(file.viewports[1].width, 768.0);
        assert_eq!(file.viewports[1].source, "md");
        assert_eq!(file.url.as_deref(), Some("http://localhost:5173"));
        assert_eq!(file.scroll_sync, Some(false));
        assert_eq!(file.source_hash.as_deref(), Some("8f2a91c4"));
    }

    #[test]
    fn rewriting_keeps_every_line_of_prose() {
        let mut file = parse_markdown(SAMPLE).unwrap();
        file.viewports.push(Viewport::new("Laptop", 1024.0, 770.0, "lg"));
        let out = render_markdown(Some(SAMPLE), &file);
        assert!(out.contains("Do not edit by hand unless you mean it."));
        assert!(out.contains("Some notes underneath, which must survive a rewrite."));
        assert!(out.contains("| Laptop | 1024 | 770 | lg |"));
        // And it still round-trips.
        let again = parse_markdown(&out).unwrap();
        assert_eq!(again.viewports.len(), 3);
    }

    #[test]
    fn a_rewrite_does_not_duplicate_the_table_or_the_block() {
        let file = parse_markdown(SAMPLE).unwrap();
        let once = render_markdown(Some(SAMPLE), &file);
        let twice = render_markdown(Some(&once), &file);
        assert_eq!(once, twice);
        assert_eq!(twice.matches("| Name | Width").count(), 1);
        assert_eq!(twice.matches(BEGIN).count(), 1);
    }

    #[test]
    fn a_file_with_no_table_is_an_error_rather_than_an_empty_config() {
        assert!(parse_markdown("# Breakpoints\n\nNothing here yet.\n").is_err());
    }

    #[test]
    fn prose_that_happens_to_use_pipes_is_not_mistaken_for_the_table() {
        let text = "Use | Name | Width | Height | in your head.\n\n| Name | Width | Height |\n|---|---|---|\n| Mobile | 640 | 850 |\n";
        let file = parse_markdown(text).unwrap();
        assert_eq!(file.viewports.len(), 1);
    }

    #[test]
    fn a_reference_column_survives_a_round_trip() {
        let mut file = ProjectFile::default();
        let mut viewport = Viewport::new("Tablet", 768.0, 1020.0, "md");
        viewport.reference = Some("figma:Ab3x?node-id=12-88".into());
        file.viewports.push(viewport);
        let out = render_markdown(None, &file);
        assert!(out.contains("| Reference |"));
        let again = parse_markdown(&out).unwrap();
        assert_eq!(
            again.viewports[0].reference.as_deref(),
            Some("figma:Ab3x?node-id=12-88")
        );
    }

    #[test]
    fn a_brand_new_file_is_written_from_nothing() {
        let mut file = ProjectFile::default();
        file.viewports.push(Viewport::new("Mobile", 640.0, 850.0, "sm"));
        file.url = Some("https://x.lndo.site".into());
        let out = render_markdown(None, &file);
        assert!(out.starts_with("# Breakpoints"));
        assert!(parse_markdown(&out).is_ok());
    }
}
