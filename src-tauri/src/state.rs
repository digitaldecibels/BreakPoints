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
    /// Unique to this note, so a batch that was handed over can be recognised
    /// and forgotten once the caller has clearly survived to ask again.
    #[serde(default)]
    pub id: String,
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
    /// A picture of the element, when one could be taken.
    ///
    /// A selector tells you where to look; a picture tells you what was wrong.
    /// Empty when the capture failed, which on macOS usually means screen
    /// recording permission has not been granted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
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
        if !self.image.is_empty() {
            out.push_str(&format!("\nPicture: {}", self.image));
        }
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

    /// Counts resize events, so the config write that follows one can wait
    /// for the drag to stop.
    pub resize_seq: Mutex<u64>,
    /// True while a relayout is already scheduled, so a drag produces one per
    /// frame rather than one per event.
    pub relayout_pending: Mutex<bool>,
    /// The main window's logical height, cached for the same reason as the
    /// width.
    pub window_height: Mutex<f64>,
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

    /// Notes handed to a caller that has not yet proved it received them.
    ///
    /// Collecting used to take a note out of the queue and hope. If the HTTP
    /// response never arrived, because the session was killed, the network
    /// stack dropped it, or the caller timed out, the note was gone and the
    /// person who wrote it had no way to know. A note is somebody describing a
    /// bug while looking at it, which is too expensive to lose to a dropped
    /// reply.
    ///
    /// A batch stays here until the same caller asks again, which it can only
    /// do if it received the last answer, or until it goes stale and returns
    /// to the queue.
    pub in_flight: Mutex<Vec<InFlight>>,

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

    /// Things to say to a listening session that are not problem reports.
    ///
    /// A skill someone chose from the menu, mostly. It goes down the same
    /// socket, because a session holding that socket open is already reading
    /// text frames and acting on them, so this needs no new transport.
    ///
    /// Deliberately not the report queue. A note is evidence and is kept until
    /// somebody proves they received it; asking for a skill to be run is an
    /// instruction, and an instruction nobody was there to hear should expire
    /// rather than arrive an hour later when the situation has changed.
    pub prompt_tx: OnceLock<tokio::sync::broadcast::Sender<String>>,

    /// How many sessions are currently watching for notes. Drives the steady
    /// "a session is listening" state in the toolbar, which is different from
    /// the dot that pulses while a single request is in flight.
    pub watchers: Mutex<usize>,

    /// Set while an agent request is in flight, so the toolbar dot can pulse.
    pub bridge_active: Mutex<bool>,
    /// Which tool that request is, so the status bar can name it.
    pub active_tool: Mutex<Option<String>>,

    /// Whether the window has focus. Nobody is scrolling a panel by hand while
    /// the app is behind something else, so the pump can go much quieter.
    pub window_focused: Mutex<bool>,
    /// Dropping this stops the agent bridge listener when the toggle goes off.
    pub bridge_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,

    /// Keeps the debounced file watcher alive; dropping it stops watching.
    pub watcher: Mutex<Option<crate::watcher::WatchHandle>>,

    /// The last thing a session said back about a note.
    ///
    /// Notes only ever went one way. You described a problem, the note left,
    /// and whatever the session decided about it happened in a window you were
    /// not looking at. This is the return leg: a session posts a sentence back
    /// and the status bar shows it, with a way to bring that terminal forward
    /// and read the rest.
    ///
    /// Kept in Rust rather than in the chrome because the chrome reloads, and
    /// a reply it heard as an event once would be gone.
    pub last_reply: Mutex<Option<Reply>>,

    /// The last few notes written in this sitting, newest first, with what
    /// became of each.
    ///
    /// Separate from `reports`, which is a queue and empties as notes are
    /// collected. This is a record, and it is what the status bar's list and
    /// the dots on the panel labels are drawn from. Not written to disk: it
    /// answers "what have I sent since I sat down", and yesterday's notes are
    /// not part of that question.
    pub recent_notes: Mutex<Vec<NoteRecord>>,
}

/// How many notes the record keeps. Ten is about one sitting's worth, and a
/// list longer than that is a log rather than something you read at a glance.
pub const RECENT_NOTES: usize = 10;

/// One note, and what became of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRecord {
    pub id: String,
    /// The panel it was written in, so its label can carry a mark.
    pub panel: String,
    pub panel_name: String,
    pub width: f64,
    /// What was typed, without the standing instruction or the selector.
    pub note: String,
    /// How to find the element again. Kept here as well as in the queued note,
    /// because the queue empties and this is what a session is handed when it
    /// asks the app to explain a note it was given earlier.
    pub selector: String,
    pub element: String,
    pub url: String,
    pub at: u64,
    /// True once a session holding a socket open has been handed it.
    pub delivered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_to: Option<String>,
    /// What the session said back about this note, when it said anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
}

/// What a session said back about a note.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    /// The note it answers, when the session named one. A session that just
    /// wants to say something leaves it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_id: Option<String>,
    /// Who said it, as the toolbar names them.
    pub from: String,
    pub text: String,
    pub at: u64,
}

/// One batch of notes handed over and not yet acknowledged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InFlight {
    pub client: String,
    pub at: u64,
    pub reports: Vec<Report>,
}

/// How long a handed-over batch waits for the caller to ask again before it is
/// treated as lost and put back.
///
/// Generous on purpose. Asking again is what acknowledges it, and a session
/// can reasonably spend a few minutes acting on a note before it comes back
/// for the next one.
const IN_FLIGHT_TIMEOUT_MS: u64 = 5 * 60 * 1000;

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

    /// Attach a picture to a note that is already queued.
    ///
    /// The capture happens after the note is stored, so the note is never
    /// delayed by it, and a note that has already been collected is simply not
    /// found here.
    pub fn attach_image(&self, id: &str, path: &str) {
        {
            let mut reports = self.reports.lock().unwrap();
            let Some(report) = reports.iter_mut().find(|r| r.id == id) else {
                return;
            };
            report.image = path.to_string();
            report.text = report.describe();
        }
        self.persist_reports();
    }

    /// Subscribe to notes as they are written. Every watcher gets every note;
    /// filtering by who a note is addressed to is the subscriber's job.
    /// Say something to every listening session.
    ///
    /// Returns how many were listening, which is the difference between the
    /// button having done something and the button having done nothing.
    pub fn say_to_sessions(&self, text: String) -> usize {
        let tx = self
            .prompt_tx
            .get_or_init(|| tokio::sync::broadcast::channel(16).0);
        tx.send(text).unwrap_or(0)
    }

    pub fn subscribe_prompts(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.prompt_tx
            .get_or_init(|| tokio::sync::broadcast::channel(16).0)
            .subscribe()
    }

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
        // Nothing is held in flight here. This is the window's own copy
        // button, so the notes are on somebody's clipboard by the time this
        // returns and there is no reply to lose.
        let had_in_flight = {
            let mut in_flight = self.in_flight.lock().unwrap();
            let had = !in_flight.is_empty();
            in_flight.clear();
            had
        };
        let taken = std::mem::take(&mut *self.reports.lock().unwrap());
        // Persisted whenever anything changed, not only when something was
        // handed over. Clearing an in-flight batch and not writing that down
        // left it on disk to be restored to the queue at the next start.
        if !taken.is_empty() || had_in_flight {
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
        // This caller asking again is proof it received the last batch, so
        // that batch can be forgotten. Anything else that has been waiting too
        // long goes back in the queue to be handed over a second time, because
        // a note delivered twice is a nuisance and a note lost is not.
        self.settle_in_flight(Some(client));

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
            self.in_flight.lock().unwrap().push(InFlight {
                client: client.to_string(),
                at: crate::util::now_ms(),
                reports: mine.clone(),
            });
            self.persist_reports();
        }
        mine
    }

    /// Forget what `acknowledged_by` was given, and put back anything that has
    /// been waiting too long.
    pub fn settle_in_flight(&self, acknowledged_by: Option<&str>) {
        let now = crate::util::now_ms();
        let mut returning: Vec<Report> = Vec::new();
        {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight.retain(|batch| {
                if Some(batch.client.as_str()) == acknowledged_by {
                    return false;
                }
                if now.saturating_sub(batch.at) >= IN_FLIGHT_TIMEOUT_MS {
                    returning.extend(batch.reports.iter().cloned());
                    return false;
                }
                true
            });
        }
        let returning_any = !returning.is_empty();
        self.requeue_reports(returning);
        // `requeue_reports` writes only when it puts something back, and
        // settling can change the in-flight list without returning anything.
        if !returning_any {
            self.persist_reports();
        }
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
    /// Record what a session said back, and hand it out for the event.
    ///
    /// The reply is also filed against a note, which is what clears the dot on
    /// that panel's label. A session that named a note answers that one; one
    /// that named nothing answers the newest note still waiting for an answer,
    /// because a session that says something without naming a note is nearly
    /// always talking about the one it was just handed.
    pub fn set_last_reply(&self, reply: Reply) -> Reply {
        *self.last_reply.lock().unwrap() = Some(reply.clone());

        let mut recent = self.recent_notes.lock().unwrap();
        let target = match &reply.report_id {
            Some(id) => recent.iter_mut().find(|note| &note.id == id),
            None => recent.iter_mut().rev().find(|note| note.reply.is_none()),
        };
        if let Some(note) = target {
            note.reply = Some(reply.clone());
        }
        reply
    }

    /// Write a note into the record, newest last, keeping only the last few.
    pub fn remember_note(&self, report: &Report) {
        let mut recent = self.recent_notes.lock().unwrap();
        recent.push(NoteRecord {
            id: report.id.clone(),
            panel: report.panel.clone(),
            panel_name: report.panel_name.clone(),
            width: report.width,
            note: report.note.clone(),
            selector: report.selector.clone(),
            element: report.element.clone(),
            url: report.url.clone(),
            at: report.at,
            delivered: false,
            delivered_to: None,
            reply: None,
        });
        // Oldest out of the front, so the newest `RECENT_NOTES` survive.
        let over = recent.len().saturating_sub(RECENT_NOTES);
        recent.drain(..over);
    }

    /// Mark a note as having reached a listening session.
    pub fn note_delivered(&self, id: &str, to: Option<&str>) {
        let mut recent = self.recent_notes.lock().unwrap();
        if let Some(note) = recent.iter_mut().find(|note| note.id == id) {
            note.delivered = true;
            note.delivered_to = to.map(|s| s.to_string());
        }
    }

    /// The record, newest first, which is the order it is read in.
    pub fn recent_notes(&self) -> Vec<NoteRecord> {
        let mut recent = self.recent_notes.lock().unwrap().clone();
        recent.reverse();
        recent
    }

    pub fn last_reply(&self) -> Option<Reply> {
        self.last_reply.lock().unwrap().clone()
    }

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
            in_flight: self.in_flight.lock().unwrap().clone(),
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
                // Anything that was in flight when the app stopped can never
                // be acknowledged now, so it goes back in the queue rather
                // than disappearing with the session it was handed to.
                for batch in saved.in_flight {
                    reports.extend(batch.reports);
                }
                reports.sort_by_key(|report| report.at);
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
    #[serde(default)]
    in_flight: Vec<InFlight>,
}

#[cfg(test)]
mod reply_tests {
    use super::*;

    fn note(id: &str, panel: &str) -> Report {
        Report {
            id: id.into(),
            panel: panel.into(),
            panel_name: panel.into(),
            width: 375.0,
            inner_width: 375.0,
            url: String::new(),
            title: String::new(),
            element: String::new(),
            selector: String::new(),
            rect: serde_json::Value::Null,
            note: "something".into(),
            at: 1,
            client: None,
            prompt: String::new(),
            text: String::new(),
            image: String::new(),
        }
    }

    fn reply(report_id: Option<&str>) -> Reply {
        Reply {
            report_id: report_id.map(|s| s.to_string()),
            from: "a session".into(),
            text: "looking".into(),
            at: 2,
        }
    }

    /// A session that says something without naming a note is talking about the
    /// note it was just handed, not the oldest one still open.
    #[test]
    fn an_unaddressed_reply_answers_the_newest_note_waiting() {
        let state = AppState::default();
        state.remember_note(&note("first", "sm"));
        state.remember_note(&note("second", "md"));
        state.set_last_reply(reply(None));

        let recent = state.recent_notes();
        assert_eq!(recent[0].id, "second", "newest first");
        assert!(recent[0].reply.is_some(), "the newest note was answered");
        assert!(recent[1].reply.is_none(), "the older one was left alone");
    }

    /// Naming a note answers that one wherever it is in the list.
    #[test]
    fn a_named_reply_answers_the_note_it_names() {
        let state = AppState::default();
        state.remember_note(&note("first", "sm"));
        state.remember_note(&note("second", "md"));
        state.set_last_reply(reply(Some("first")));

        let recent = state.recent_notes();
        assert!(recent[1].reply.is_some(), "the one it named");
        assert!(recent[0].reply.is_none(), "and only that one");
    }

    /// The record is a window on the sitting, not a log that grows for ever.
    #[test]
    fn the_record_keeps_only_the_last_few() {
        let state = AppState::default();
        for i in 0..(RECENT_NOTES + 5) {
            state.remember_note(&note(&format!("n{i}"), "sm"));
        }
        let recent = state.recent_notes();
        assert_eq!(recent.len(), RECENT_NOTES);
        assert_eq!(recent[0].id, format!("n{}", RECENT_NOTES + 4), "newest kept");
    }

    /// Delivery is recorded against the note it happened to.
    #[test]
    fn delivery_is_written_against_the_right_note() {
        let state = AppState::default();
        state.remember_note(&note("first", "sm"));
        state.remember_note(&note("second", "md"));
        state.note_delivered("first", Some("a session"));

        let recent = state.recent_notes();
        assert!(!recent[0].delivered, "the one that did not go");
        assert!(recent[1].delivered, "the one that did");
        assert_eq!(recent[1].delivered_to.as_deref(), Some("a session"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(client: Option<&str>) -> Report {
        let mut report = Report {
            id: "n1".into(),
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
            image: String::new(),
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

    /// A batch handed over is kept until the caller proves it arrived, which it
    /// does by asking again. Collecting used to take notes out of the queue and
    /// hope the reply reached anybody.
    #[test]
    fn a_note_handed_over_is_kept_until_the_caller_asks_again() {
        let state = AppState::default();
        state.push_report(note(None));

        let first = state.take_reports_for("session-a");
        assert_eq!(first.len(), 1);
        assert_eq!(state.report_count(), 0, "out of the queue");
        assert_eq!(state.in_flight.lock().unwrap().len(), 1, "but not forgotten");

        // Asking again is the acknowledgement.
        let second = state.take_reports_for("session-a");
        assert!(second.is_empty());
        assert!(
            state.in_flight.lock().unwrap().is_empty(),
            "the first batch is settled once the caller comes back"
        );
    }

    /// A batch nobody ever came back for goes back in the queue rather than
    /// vanishing with the session it was handed to.
    #[test]
    fn a_batch_nobody_acknowledged_comes_back() {
        let state = AppState::default();
        state.push_report(note(None));
        state.take_reports_for("session-a");

        // Pretend it was handed over long enough ago to have been lost.
        state.in_flight.lock().unwrap()[0].at = 1;
        state.settle_in_flight(Some("session-b"));

        assert_eq!(state.report_count(), 1, "back in the queue");
        assert_eq!(state.take_reports_for("session-b").len(), 1);
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
