//! The scan diagnostics log.
//!
//! Detection will fail on real projects. The point is not to prevent that, it
//! is to make every failure legible enough to fix the detector instead of
//! guessing why a project came up empty. So a detector that finds nothing still
//! has to say what it looked at and why nothing qualified.
//!
//! Nothing outside the project root is ever logged, and no file content beyond
//! the snippet that failed to parse.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::util;

pub struct ScanLog {
    detector: Option<String>,
    started: std::time::Instant,
    entries: Vec<Value>,
}

impl ScanLog {
    pub fn new(project: &str) -> Self {
        let mut log = Self {
            detector: None,
            started: std::time::Instant::now(),
            entries: Vec::new(),
        };
        log.push(json!({ "event": "scan.start", "project": project, "at": util::now_iso() }));
        log
    }

    fn push(&mut self, mut value: Value) {
        if let Some(detector) = &self.detector {
            value["detector"] = json!(detector);
        }
        value["ms"] = json!(self.started.elapsed().as_millis() as u64);
        self.entries.push(value);
    }

    pub fn detector_start(&mut self, name: &str) {
        self.detector = None;
        self.push(json!({ "event": "detector.start", "name": name }));
        self.detector = Some(name.to_string());
    }

    pub fn detector_end(&mut self, name: &str, found: usize, took_ms: u64) {
        self.push(json!({
            "event": "detector.end",
            "name": name,
            "breakpointsFound": found,
            "tookMs": took_ms,
        }));
        self.detector = None;
    }

    pub fn read(&mut self, file: &str, bytes: usize) {
        self.push(json!({ "event": "file.read", "file": file, "bytes": bytes }));
    }

    pub fn skip(&mut self, file: &str, reason: &str) {
        self.push(json!({ "event": "file.skip", "file": file, "reason": reason }));
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.push(json!({ "event": "note", "message": message.into() }));
    }

    /// A parse failure, in full: where it was, what it looked like, and which
    /// step gave up. This is the entry that makes the detector improvable.
    pub fn parse_failure(
        &mut self,
        file: &str,
        line: Option<usize>,
        snippet: &str,
        step: &str,
        reason: &str,
    ) {
        self.push(json!({
            "event": "parse.failure",
            "file": file,
            "line": line,
            "snippet": trim_snippet(snippet),
            "step": step,
            "reason": reason,
        }));
    }

    /// A value we found and threw away, with why. Half of a detector's job.
    pub fn discarded(&mut self, file: &str, value: &str, reason: &str) {
        self.push(json!({
            "event": "value.discarded",
            "file": file,
            "value": value,
            "reason": reason,
        }));
    }

    pub fn candidate(&mut self, url: &str, source: &str, confidence: f64) {
        self.push(json!({
            "event": "devurl.candidate",
            "url": url,
            "source": source,
            "confidence": confidence,
        }));
    }

    pub fn merged(&mut self, widths: &[f64], conflicts: usize) {
        self.push(json!({
            "event": "merge.result",
            "widths": widths,
            "conflicts": conflicts,
        }));
    }

    pub fn entries(&self) -> &[Value] {
        &self.entries
    }

    /// The lines a scan warning's "why?" expander should show for one file.
    pub fn lines_for_file(&self, file: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.get("file").and_then(Value::as_str) == Some(file))
            .filter(|e| {
                matches!(
                    e.get("event").and_then(Value::as_str),
                    Some("parse.failure") | Some("value.discarded") | Some("file.skip")
                )
            })
            .map(render_line)
            .collect()
    }

    /// Write the log next to the app's other data and prune to the last 20 for
    /// this project.
    pub fn write(&self, dir: &Path, project_slug: &str) -> Option<PathBuf> {
        let name = format!("{project_slug}-{}.log", util::now_ms());
        let path = dir.join(name);
        let body: String = self
            .entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, body).ok()?;
        prune(dir, project_slug, 20);
        Some(path)
    }
}

/// A JSON line rendered the way the scan sheet shows it.
pub fn render_line(entry: &Value) -> String {
    let get = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or("");
    match entry.get("event").and_then(Value::as_str) {
        Some("parse.failure") => {
            let line = entry
                .get("line")
                .and_then(Value::as_u64)
                .map(|l| format!(":{l}"))
                .unwrap_or_default();
            format!(
                "{}{} — {} ({})\n    {}",
                get("file"),
                line,
                get("reason"),
                get("step"),
                get("snippet")
            )
        }
        Some("value.discarded") => {
            format!("{} — discarded {}: {}", get("file"), get("value"), get("reason"))
        }
        Some("file.skip") => format!("{} — skipped: {}", get("file"), get("reason")),
        Some("devurl.candidate") => format!(
            "{} from {} (confidence {})",
            get("url"),
            get("source"),
            entry.get("confidence").and_then(Value::as_f64).unwrap_or(0.0)
        ),
        Some("note") => get("message").to_string(),
        _ => serde_json::to_string(entry).unwrap_or_default(),
    }
}

fn trim_snippet(snippet: &str) -> String {
    let one_line: String = snippet
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    one_line
}

fn prune(dir: &Path, project_slug: &str, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut mine: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&format!("{project_slug}-")) && n.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    if mine.len() <= keep {
        return;
    }
    mine.sort();
    for path in mine.iter().take(mine.len() - keep) {
        let _ = std::fs::remove_file(path);
    }
}
