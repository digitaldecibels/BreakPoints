//! Everything the app knows at runtime, in one place, behind ordinary mutexes.
//!
//! Locks are held for the length of a field read or a short mutation and never
//! across a webview call that can re-enter, which is what keeps the scroll path
//! from deadlocking against the panel callback server.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::canvas::Canvas;
use crate::model::{AppConfig, UrlStatus};
use crate::scanner::types::ScanReport;

/// A problem someone pointed at in a panel, with the width it happened at.
///
/// This is the whole point of the app written down: a layout is only wrong at
/// some widths, so a note about one is worthless without the width attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Which agent session this note was addressed to, if one had claimed
    /// reports when it was written. `None` means nobody had, and only the
    /// chrome's copy button will ever collect it.
    pub client: Option<String>,
    /// The standing instruction, captured when the note was written rather than
    /// when it is collected, so editing it later does not rewrite the meaning
    /// of notes already sitting in the queue.
    pub prompt: String,
    /// The note as prose, built once here rather than by whoever collects it.
    ///
    /// This used to be assembled in the chrome's JavaScript for the clipboard
    /// and left to each agent to invent for itself over the bridge, so the same
    /// note read differently depending on how it arrived. One formatter means
    /// the clipboard, the bridge and a watching session all say the same thing.
    #[serde(default)]
    pub text: String,
}

impl Report {
    /// The instruction leads, because whoever reads this needs to know what
    /// they are being asked to do before they read what is wrong.
    pub fn describe(&self) -> String {
        let drawn = if (self.inner_width - self.width).abs() > 1.0 {
            format!(
                " (the page reports {}px, which is itself wrong)",
                self.inner_width.round()
            )
        } else {
            String::new()
        };
        let mut out = String::new();
        let lead = self.prompt.trim();
        if !lead.is_empty() {
            out.push_str(lead);
            out.push_str("\n\n---\n\n");
        }
        out.push_str(&format!(
            "{}\n\nThis problem exists at the {}px breakpoint{}, in the {} panel.\nElement: {}\nSelector: {}\nPage: {}",
            self.note,
            self.width.round(),
            drawn,
            self.panel_name,
            self.element,
            self.selector,
            self.url,
        ));
        out
    }
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

    /// The agent session notes are currently addressed to.
    ///
    /// Exactly one, and the last session to claim wins. Rick runs several
    /// Claude Code sessions at once, and the old behaviour was that whichever
    /// one asked first swallowed every note including the ones meant for
    /// another. Addressing a note fixes that without asking a person to choose
    /// a destination every time they write one.
    pub report_owner: Mutex<Option<ClientSession>>,

    /// Where injected scripts call home. Set once at startup.
    pub endpoint: OnceLock<Endpoint>,

    /// The main window's logical width, kept up to date from the resize
    /// handler.
    ///
    /// Asking the window for its size sends a message to the main thread and
    /// blocks until it answers, with no timeout, so every hot path that wanted
    /// it was parking a worker thread behind a main thread that is laying out
    /// seven pages. Reading it from here costs a mutex.
    pub window_width: Mutex<f64>,

    /// Held for the whole of `canvas::spawn`, so two row rebuilds cannot
    /// interleave.
    ///
    /// `spawn` empties the row, then fills it one panel at a time with a 20ms
    /// pause between each. Two overlapping calls used to mean the second
    /// emptied a list the first was still filling, while the first kept
    /// pushing panels positioned by a different layout: home positions from
    /// two generations, a total width describing neither, labels offset from
    /// the panels they name, and webviews nobody closed. An async mutex,
    /// because `spawn` awaits.
    pub spawning: tokio::sync::Mutex<()>,

    /// Panels whose first navigation was reported before the panel itself had
    /// been added to the row.
    ///
    /// `add_child` returns a webview that is already loading, and the `Panel`
    /// record is pushed after it returns, so the load report can arrive first
    /// and find nothing to write to. That report is what marks a panel safe to
    /// ask questions of, and losing it left the panel silent for the whole
    /// session: no scroll sync, no console capture, and no problem reports.
    /// Ids land here instead and are claimed by the push.
    pub committed_early: Mutex<std::collections::HashSet<String>>,

    /// Where the queue and the claim are written, so neither is lost when the
    /// app restarts. Set once at startup; nothing is persisted until it is.
    pub reports_file: OnceLock<PathBuf>,

    /// Notes go out here the moment they are written, to anything waiting on
    /// one: a WebSocket, or a long poll holding a request open. This is what
    /// makes delivery a push rather than a two second poll.
    pub report_tx: OnceLock<tokio::sync::broadcast::Sender<Report>>,

    /// How many sessions are currently watching for notes. Drives the steady
    /// "a session is listening" state in the toolbar, which is different from
    /// the dot that pulses while a single request is in flight.
    pub watchers: Mutex<usize>,

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

/// An agent session that has claimed reports.
///
/// Claude Code hands every session a stable id in `CLAUDE_CODE_SESSION_ID`, so
/// a session can name itself without the app inventing an identity for it. The
/// name is for the toolbar; the id is what a note is addressed to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSession {
    pub id: String,
    pub name: String,
    /// When it claimed. Only used to say "since 14:02" in the toolbar.
    pub at: u64,
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

    /// Queue a note, write it down, and hand it to anything already waiting.
    ///
    /// The broadcast is what makes delivery immediate. A session holding a
    /// WebSocket or a long poll gets the note at the moment it is written; the
    /// queue is what a session that was not connected collects later.
    pub fn push_report(&self, report: Report) {
        {
            let mut reports = self.reports.lock().unwrap();
            reports.push(report.clone());
            // Nobody is going to read the two hundredth unclaimed note, and
            // this buffer outlives every panel in it.
            if reports.len() > 200 {
                let overflow = reports.len() - 200;
                reports.drain(0..overflow);
            }
        }
        self.persist_reports();
        if let Some(tx) = self.report_tx.get() {
            // An error here only means nothing is listening, which is the
            // ordinary case and not a failure: the note is in the queue.
            let _ = tx.send(report);
        }
    }

    /// Put notes back at the front of the queue, oldest first.
    ///
    /// Delivery takes a note out of the queue, so a send that fails half way
    /// through would destroy the rest. A note nobody received is not a note
    /// that was handled. Nothing is broadcast again: whatever was listening
    /// has gone, which is why we are here.
    pub fn requeue_reports(&self, mut notes: Vec<Report>) {
        if notes.is_empty() {
            return;
        }
        {
            let mut reports = self.reports.lock().unwrap();
            notes.append(&mut reports);
            *reports = notes;
        }
        self.persist_reports();
    }

    /// Subscribe to notes as they are written. Every watcher gets every note;
    /// filtering by who a note is addressed to is the subscriber's job.
    pub fn subscribe_reports(&self) -> tokio::sync::broadcast::Receiver<Report> {
        self.report_tx
            .get_or_init(|| tokio::sync::broadcast::channel(64).0)
            .subscribe()
    }

    /// Hand over everything and start again. Taking rather than reading is the
    /// default so the same note is not acted on twice.
    ///
    /// This is the chrome's copy button and the escape hatch: it collects every
    /// note whoever it was addressed to, which is what makes a note written
    /// while a dead session owned reports still reachable.
    pub fn take_reports(&self) -> Vec<Report> {
        let taken = std::mem::take(&mut *self.reports.lock().unwrap());
        if !taken.is_empty() {
            self.persist_reports();
        }
        taken
    }

    /// Hand over the notes addressed to one session, plus any addressed to
    /// nobody, and leave other sessions' notes alone.
    ///
    /// This is what an agent calls. Several sessions poll at once, so a drain
    /// that ignored the address would mean the fastest poller swallowed notes
    /// meant for a different window.
    ///
    /// An unaddressed note belongs to whoever asks first, and that is what
    /// stops a note being stranded. The claim lives in memory, so a rebuild,
    /// a reload or a crash loses it, and every note written after that is
    /// addressed to nobody. Matching only the exact id left those notes in the
    /// queue forever while the session that wanted them polled past them.
    pub fn take_reports_for(&self, client: &str) -> Vec<Report> {
        let mut reports = self.reports.lock().unwrap();
        let mut mine = Vec::new();
        let mut theirs = Vec::new();
        for report in std::mem::take(&mut *reports) {
            match report.client.as_deref() {
                Some(owner) if owner != client => theirs.push(report),
                _ => mine.push(report),
            }
        }
        *reports = theirs;
        drop(reports);
        if !mine.is_empty() {
            self.persist_reports();
        }
        mine
    }

    /// How many notes are waiting, whoever they are addressed to. The toolbar
    /// count comes from here rather than from the chrome counting events, so
    /// a note collected over the bridge is reflected in the window.
    pub fn report_count(&self) -> usize {
        self.reports.lock().unwrap().len()
    }

    /// Whether anything is listening for a note right now.
    pub fn watching(&self) -> bool {
        *self.watchers.lock().unwrap() > 0
    }

    pub fn watcher_joined(&self) {
        *self.watchers.lock().unwrap() += 1;
    }

    pub fn watcher_left(&self) {
        let mut watchers = self.watchers.lock().unwrap();
        *watchers = watchers.saturating_sub(1);
    }

    /// Claim reports for a session. The last one to ask wins, deliberately:
    /// running the command in a window is how you say "send them here now".
    pub fn claim_reports(&self, session: ClientSession) -> ClientSession {
        {
            let mut owner = self.report_owner.lock().unwrap();
            *owner = Some(session.clone());
        }
        self.persist_reports();
        session
    }

    /// Which session notes are going to, if any.
    pub fn report_owner(&self) -> Option<ClientSession> {
        self.report_owner.lock().unwrap().clone()
    }

    pub fn clear_console(&self, panel: &str) {
        self.console.lock().unwrap().remove(panel);
    }

    /// Write the queue and the claim to disk.
    ///
    /// `tauri dev` rebuilds on every Rust edit, a Vite reload restarts the
    /// chrome, and either one used to take every uncollected note with it. A
    /// note is somebody describing a bug while looking at it, so losing one
    /// costs more than the few milliseconds this takes.
    pub fn persist_reports(&self) {
        let Some(path) = self.reports_file.get() else {
            return;
        };
        let saved = SavedReports {
            reports: self.reports.lock().unwrap().clone(),
            owner: self.report_owner.lock().unwrap().clone(),
        };
        let Ok(text) = serde_json::to_string_pretty(&saved) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// Read back what the last run left waiting.
    ///
    /// The queue comes back and the claim deliberately does not. A claim names
    /// a session that may well have ended while the app was down, and restoring
    /// it would address every new note to a session nobody is watching. Nobody
    /// owning reports is the honest state, and it is also the collectable one,
    /// because an unaddressed note goes to whoever asks.
    pub fn restore_reports(&self, path: PathBuf) {
        let _ = self.reports_file.set(path.clone());
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        match serde_json::from_str::<SavedReports>(&text) {
            Ok(saved) => {
                let mut reports = self.reports.lock().unwrap();
                *reports = saved.reports;
            }
            Err(err) => {
                eprintln!("[breakpoints] waiting reports at {} did not parse: {err}", path.display());
            }
        }
    }
}

/// The on-disk shape of what is waiting. Its own struct so the file can gain a
/// field later without the queue becoming unreadable.
#[derive(Debug, Serialize, Deserialize)]
struct SavedReports {
    #[serde(default)]
    reports: Vec<Report>,
    #[serde(default)]
    owner: Option<ClientSession>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(client: Option<&str>) -> Report {
        let mut report = Report {
            panel: "p1".into(),
            panel_name: "Medium".into(),
            width: 768.0,
            inner_width: 768.0,
            url: "https://example.test/about".into(),
            title: "About".into(),
            element: "div.card".into(),
            selector: "#main > div.card".into(),
            rect: serde_json::json!({}),
            note: "The heading wraps".into(),
            at: 1,
            client: client.map(|c| c.to_string()),
            prompt: "Fix this.".into(),
            text: String::new(),
        };
        report.text = report.describe();
        report
    }

    /// The one that matters. The claim lives in memory, so a rebuild or a
    /// reload leaves every later note addressed to nobody. Matching only the
    /// exact id left those notes in the queue while the session that wanted
    /// them polled straight past.
    #[test]
    fn a_note_addressed_to_nobody_goes_to_whoever_asks() {
        let state = AppState::default();
        state.push_report(note(None));
        assert_eq!(state.take_reports_for("session-a").len(), 1);
        assert_eq!(state.report_count(), 0);
    }

    #[test]
    fn a_note_addressed_to_another_session_is_left_where_it_is() {
        let state = AppState::default();
        state.push_report(note(Some("session-b")));
        assert!(state.take_reports_for("session-a").is_empty());
        assert_eq!(state.report_count(), 1, "it is still there for session-b");
        assert_eq!(state.take_reports_for("session-b").len(), 1);
    }

    #[test]
    fn the_width_is_in_the_prose_because_that_is_the_whole_point() {
        let text = note(None).describe();
        assert!(text.contains("768px breakpoint"), "{text}");
        assert!(text.starts_with("Fix this."), "the instruction leads: {text}");
        assert!(text.contains("The heading wraps"), "{text}");
        assert!(
            !text.contains("which is itself wrong"),
            "the page agreed about its width, so there is nothing to say"
        );
    }

    /// When the panel and the page disagree about the width, that disagreement
    /// is itself the bug and has to reach whoever reads the note.
    #[test]
    fn a_page_that_disagrees_about_its_own_width_says_so() {
        let mut report = note(None);
        report.inner_width = 751.0;
        assert!(report.describe().contains("reports 751px"));
    }

    #[test]
    fn a_restart_keeps_the_queue_and_forgets_the_claim() {
        let path = std::env::temp_dir().join(format!(
            "breakpoints-reports-test-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let before = AppState::default();
        let _ = before.reports_file.set(path.clone());
        before.claim_reports(ClientSession {
            id: "session-a".into(),
            name: "bucknell".into(),
            at: 1,
        });
        before.push_report(note(Some("session-a")));

        // A fresh process, reading what the last one left.
        let after = AppState::default();
        after.restore_reports(path.clone());
        assert_eq!(after.report_count(), 1, "the note survived");
        assert!(
            after.report_owner().is_none(),
            "the claim did not, because the session it named may be gone"
        );
        // And because nothing owns reports, the note is still collectable by
        // the session that comes back for it.
        assert_eq!(after.take_reports_for("session-a").len(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_watcher_leaving_cannot_take_the_count_below_nothing() {
        let state = AppState::default();
        state.watcher_left();
        assert!(!state.watching());
        state.watcher_joined();
        assert!(state.watching());
        state.watcher_left();
        assert!(!state.watching());
    }
}
