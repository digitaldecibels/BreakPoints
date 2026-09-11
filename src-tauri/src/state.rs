//! Everything the app knows at runtime, in one place, behind ordinary mutexes.
//!
//! Locks are held for the length of a field read or a short mutation and never
//! across a webview call that can re-enter, which is what keeps the scroll path
//! from deadlocking against the panel callback server.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use crate::canvas::Canvas;
use crate::model::{AppConfig, UrlStatus};
use crate::scanner::types::ScanReport;

/// A problem someone pointed at in a panel, with the width it happened at.
///
/// This is the whole point of the app written down: a layout is only wrong at
/// some widths, so a note about one is worthless without the width attached.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub panel: String,
    /// The name a person uses, "Medium", not the id.
    pub panel_name: String,
    /// The declared breakpoint width, which is what the note is about.
    pub width: f64,
    /// What the page thinks it is, which should equal `width` and is worth
    /// carrying because when it does not, that is itself the bug.
    pub inner_width: f64,
    pub url: String,
    pub title: String,
    pub element: String,
    pub selector: String,
    pub rect: serde_json::Value,
    pub note: String,
    pub at: u64,
}

/// One console line captured from a panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub level: String,
    pub text: String,
    pub at: u64,
}

/// What the app currently believes about the open project. `None` means we are
/// testing a bare URL with a profile.
#[derive(Debug, Clone, Default)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub name: String,
    /// True when `breakpoints.md` or `.breakpoints.json` is driving the
    /// viewports, which is the app's one visual signal that it is reading code.
    pub project_driven: bool,
    /// The dev URL that was actually chosen, which is the first candidate that
    /// answered rather than the highest-confidence guess. Reporting the guess
    /// sent an agent to a hostname the panels were never pointed at.
    pub dev_url: Option<String>,
    pub report: Option<ScanReport>,
}

#[derive(Default)]
pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub canvas: Mutex<Canvas>,
    pub project: Mutex<Option<ProjectContext>>,
    pub url_status: Mutex<UrlStatus>,

    /// Rolling console buffer per panel, newest last, capped at 200.
    pub console: Mutex<HashMap<String, Vec<ConsoleLine>>>,

    /// Problems reported from a panel, oldest first, waiting to be collected.
    pub reports: Mutex<Vec<Report>>,

    /// Where injected scripts call home. Set once at startup.
    pub endpoint: OnceLock<Endpoint>,

    /// Set while an agent request is in flight, so the toolbar dot can pulse.
    pub bridge_active: Mutex<bool>,

    /// Whether the window has focus. Nobody is scrolling a panel by hand while
    /// the app is behind something else, so the pump can go much quieter.
    pub window_focused: Mutex<bool>,
    /// Dropping this stops the agent bridge listener when the toggle goes off.
    pub bridge_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,

    /// Keeps the debounced file watcher alive; dropping it stops watching.
    pub watcher: Mutex<Option<crate::watcher::WatchHandle>>,
}

/// The loopback address injected scripts call, plus the nonce that proves a
/// message came from a panel we spawned rather than from the page's own code.
///
/// Only an http page can actually reach it. WebKit blocks an http request made
/// by an https document and does not exempt `127.0.0.1` the way Chrome does, so
/// an https panel queues its messages and `canvas::start_pump` collects them
/// instead. The nonce is the same either way.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub port: u16,
    pub nonce: String,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn push_console(&self, panel: &str, line: ConsoleLine) {
        let mut map = self.console.lock().unwrap();
        let buf = map.entry(panel.to_string()).or_default();
        buf.push(line);
        if buf.len() > 200 {
            let overflow = buf.len() - 200;
            buf.drain(0..overflow);
        }
    }

    pub fn push_report(&self, report: Report) {
        let mut reports = self.reports.lock().unwrap();
        reports.push(report);
        // Nobody is going to read the two hundredth unclaimed note, and this
        // buffer outlives every panel in it.
        if reports.len() > 200 {
            let overflow = reports.len() - 200;
            reports.drain(0..overflow);
        }
    }

    /// Hand over everything and start again. Taking rather than reading is the
    /// default so the same note is not acted on twice.
    pub fn take_reports(&self) -> Vec<Report> {
        std::mem::take(&mut *self.reports.lock().unwrap())
    }

    pub fn clear_console(&self, panel: &str) {
        self.console.lock().unwrap().remove(panel);
    }
}
