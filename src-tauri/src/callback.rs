//! The loopback server that injected panel scripts call home on.
//!
//! It exists because a script injected into someone else's page has no Tauri
//! IPC to invoke. It is not the agent bridge: it carries no project data, it
//! listens on an ephemeral port, and every message has to carry the per-session
//! nonce that was baked into the script we injected.

use std::net::TcpListener as StdListener;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use crate::canvas;
use crate::model::PanelState;
use crate::state::{ConsoleLine, Endpoint, Report, Shared};
use crate::util;

#[derive(Clone)]
struct Ctx {
    app: AppHandle,
    state: Shared,
    nonce: String,
}

#[derive(Debug, Deserialize)]
struct Scroll {
    panel: String,
    nonce: String,
    pct: f64,
}

#[derive(Debug, Deserialize)]
struct Console {
    panel: String,
    nonce: String,
    level: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct Ready {
    panel: String,
    nonce: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    width: f64,
}

/// A URL change that did not load a document: the history API, or the back
/// button inside a single page app.
#[derive(Debug, Deserialize)]
struct Nav {
    panel: String,
    nonce: String,
    #[serde(default)]
    url: String,
}

/// A problem someone picked out in a panel.
#[derive(Debug, Deserialize)]
struct ReportIn {
    panel: String,
    nonce: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    selector: String,
    #[serde(default)]
    element: String,
    #[serde(default, rename = "innerWidth")]
    inner_width: f64,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    rect: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Wheel {
    nonce: String,
    dx: f64,
}

/// Bind before anything spawns, so the port is known when the first panel's
/// script is built. Returns the endpoint the scripts should call.
pub fn start(app: AppHandle, state: Shared) -> Result<Endpoint, String> {
    let listener = StdListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let endpoint = Endpoint {
        port,
        nonce: util::random_token(16),
    };

    let ctx = Ctx {
        app,
        state,
        nonce: endpoint.nonce.clone(),
    };

    let router = Router::new()
        .route("/p/scroll", post(on_scroll))
        .route("/p/console", post(on_console))
        .route("/p/ready", post(on_ready))
        .route("/p/wheel", post(on_wheel))
        .route("/p/nav", post(on_nav))
        .route("/p/report", post(on_report))
        .with_state(Arc::new(ctx));

    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => {
                if let Err(err) = axum::serve(listener, router).await {
                    eprintln!("[breakpoints] panel callback server stopped: {err}");
                }
            }
            Err(err) => eprintln!("[breakpoints] could not adopt callback listener: {err}"),
        }
    });

    Ok(endpoint)
}

/// The page's own origin is whatever site is loaded, so every response needs to
/// say the loopback server is happy to be called from it.
fn ok() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        "access-control-allow-origin",
        HeaderValue::from_static("*"),
    );
    (StatusCode::NO_CONTENT, headers)
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Option<T> {
    serde_json::from_str(body).ok()
}

async fn on_scroll(State(ctx): State<Arc<Ctx>>, body: String) -> impl IntoResponse {
    if let Some(msg) = parse::<Scroll>(&body) {
        scroll(&ctx.state, &ctx.nonce, msg);
    }
    ok()
}

async fn on_console(State(ctx): State<Arc<Ctx>>, body: String) -> impl IntoResponse {
    if let Some(msg) = parse::<Console>(&body) {
        console(&ctx.app, &ctx.state, &ctx.nonce, msg);
    }
    ok()
}

async fn on_ready(State(ctx): State<Arc<Ctx>>, body: String) -> impl IntoResponse {
    if let Some(msg) = parse::<Ready>(&body) {
        ready(&ctx.app, &ctx.state, &ctx.nonce, msg);
    }
    ok()
}

async fn on_nav(State(ctx): State<Arc<Ctx>>, body: String) -> impl IntoResponse {
    if let Some(msg) = parse::<Nav>(&body) {
        nav(&ctx.app, &ctx.state, &ctx.nonce, msg);
    }
    ok()
}

async fn on_report(State(ctx): State<Arc<Ctx>>, body: String) -> impl IntoResponse {
    if let Some(msg) = parse::<ReportIn>(&body) {
        report(&ctx.app, &ctx.state, &ctx.nonce, msg);
    }
    ok()
}

async fn on_wheel(State(ctx): State<Arc<Ctx>>, body: String) -> impl IntoResponse {
    if let Some(msg) = parse::<Wheel>(&body) {
        wheel(&ctx.app, &ctx.state, &ctx.nonce, msg);
    }
    ok()
}

fn scroll(state: &Shared, nonce: &str, msg: Scroll) {
    if msg.nonce != nonce {
        return;
    }
    canvas::sync_scroll(state, &msg.panel, msg.pct);
}

fn console(app: &AppHandle, state: &Shared, nonce: &str, msg: Console) {
    if msg.nonce != nonce {
        return;
    }
    let line = ConsoleLine {
        level: msg.level.clone(),
        // A page can log a novel. Keep the buffer readable.
        text: truncate(&msg.text, 2000),
        at: util::now_ms(),
    };
    state.push_console(&msg.panel, line);
    if msg.level == "error" || msg.level == "warn" {
        let _ = app.emit(
            "panel:console",
            serde_json::json!({ "panel": msg.panel, "level": msg.level }),
        );
    }
}

fn ready(app: &AppHandle, state: &Shared, nonce: &str, msg: Ready) {
    if msg.nonce != nonce {
        return;
    }
    // A panel that reports in has rendered, whatever the HTTP status was, so
    // this is the honest moment to call it loaded.
    canvas::set_panel_state(app, state, &msg.panel, PanelState::Loaded);

    // The page has just told us how wide it thinks it is, and that is the one
    // claim this app cannot afford to get wrong. It used to be forwarded to the
    // window and dropped. A disagreement here is checked once more before it is
    // believed, because a first load can report the pre-zoom width.
    if msg.width > 0.0 && canvas::set_reported_width(app, state, &msg.panel, msg.width) {
        canvas::confirm_width(app.clone(), state.clone(), msg.panel.clone());
    }

    let _ = app.emit(
        "panel:ready",
        serde_json::json!({
            "panel": msg.panel,
            "url": msg.url,
            "title": msg.title,
            "innerWidth": msg.width,
        }),
    );
    // A loaded document is also how a followed link announces itself. Every
    // panel we pushed reports here too, which is exactly what
    // `follow_navigation` has to tell apart from someone clicking.
    if !msg.url.is_empty() {
        canvas::follow_navigation(app, state, &msg.panel, &msg.url);
    }
}

fn nav(app: &AppHandle, state: &Shared, nonce: &str, msg: Nav) {
    if msg.nonce != nonce {
        return;
    }
    if msg.url.is_empty() {
        return;
    }
    canvas::follow_navigation(app, state, &msg.panel, &msg.url);
}

fn report(app: &AppHandle, state: &Shared, nonce: &str, msg: ReportIn) {
    if msg.nonce != nonce || msg.note.trim().is_empty() {
        return;
    }
    // The panel's declared width comes from the panel, never from the page. A
    // page can say anything about itself; what the note is about is the
    // breakpoint we put it at.
    let (name, width) = {
        let canvas = state.canvas.lock().unwrap();
        match canvas.panels.iter().find(|p| p.viewport.id == msg.panel) {
            Some(panel) => (panel.viewport.name.clone(), panel.viewport.width),
            None => (msg.panel.clone(), msg.inner_width),
        }
    };

    let mut report = Report {
        id: util::random_token(8),
        panel: msg.panel.clone(),
        panel_name: name,
        width,
        inner_width: msg.inner_width,
        url: msg.url.clone(),
        title: truncate(&msg.title, 300),
        element: truncate(&msg.element, 300),
        selector: truncate(&msg.selector, 600),
        rect: msg.rect.clone(),
        note: truncate(&msg.note, 4000),
        at: util::now_ms(),
        // Addressed at the moment it is written, not at the moment it is
        // collected. Switching sessions after writing a note should not
        // redirect the note.
        client: state.report_owner().map(|owner| owner.id),
        prompt: state
            .config
            .lock()
            .unwrap()
            .report_prompt
            .clone()
            .unwrap_or_else(|| crate::model::DEFAULT_REPORT_PROMPT.to_string()),
        text: String::new(),
        image: String::new(),
    };
    // Written once, here, so the clipboard, the bridge and a watching session
    // all hand over the same words.
    report.text = report.describe();
    state.push_report(report.clone());
    let _ = app.emit("report:new", &report);
    crate::tools::emit_report_count(app, state);

    // The picture is taken after the note is queued, not before, so a capture
    // that fails or takes a second cannot delay or lose the note itself. When
    // it arrives the note is updated in place.
    let app_for_shot = app.clone();
    let state_for_shot = state.clone();
    let panel = msg.panel.clone();
    let selector = report.selector.clone();
    let id = report.id.clone();
    tauri::async_runtime::spawn(async move {
        if selector.is_empty() {
            return;
        }
        match crate::shots::capture_element(&app_for_shot, &state_for_shot, &panel, &selector).await
        {
            Ok(path) => {
                state_for_shot.attach_image(&id, &path);
                crate::tools::emit_report_count(&app_for_shot, &state_for_shot);
            }
            Err(err) => {
                eprintln!("[breakpoints] no picture for the note on {selector}: {err}");
            }
        }
    });
}

fn wheel(app: &AppHandle, state: &Shared, nonce: &str, msg: Wheel) {
    if msg.nonce != nonce {
        return;
    }
    canvas::nudge_scroll(app, state, msg.dx);
}

/// One message that did not arrive over HTTP, from a panel we know the id of.
///
/// Two things reach this: `panel_event`, which is Tauri's own IPC and carries
/// the sending webview's label, and the pump, which drains a queue out of a
/// page that could not use either route. Both end at the same handlers a
/// posted message reaches, so there is no second implementation to drift.
///
/// The panel id is the caller's, not the message's. A page can put whatever it
/// likes in the body, so the one field it must not be trusted with is which
/// panel it is.
pub fn dispatch_from(
    app: &AppHandle,
    state: &Shared,
    panel: &str,
    message: &serde_json::Value,
) {
    let Some(path) = message.get("path").and_then(|p| p.as_str()) else {
        return;
    };
    let Some(body) = message.get("body") else {
        return;
    };
    let mut body = body.clone();
    if let Some(object) = body.as_object_mut() {
        object.insert("panel".into(), serde_json::Value::String(panel.to_string()));
    }
    let body = &body;

    let nonce = match state.endpoint.get() {
        Some(endpoint) => endpoint.nonce.clone(),
        None => return,
    };

    match path {
        "/p/scroll" => {
            if let Ok(msg) = serde_json::from_value::<Scroll>(body.clone()) {
                scroll(state, &nonce, msg);
            }
        }
        "/p/console" => {
            if let Ok(msg) = serde_json::from_value::<Console>(body.clone()) {
                console(app, state, &nonce, msg);
            }
        }
        "/p/ready" => {
            if let Ok(msg) = serde_json::from_value::<Ready>(body.clone()) {
                ready(app, state, &nonce, msg);
            }
        }
        "/p/wheel" => {
            if let Ok(msg) = serde_json::from_value::<Wheel>(body.clone()) {
                wheel(app, state, &nonce, msg);
            }
        }
        "/p/nav" => {
            if let Ok(msg) = serde_json::from_value::<Nav>(body.clone()) {
                nav(app, state, &nonce, msg);
            }
        }
        "/p/report" => {
            if let Ok(msg) = serde_json::from_value::<ReportIn>(body.clone()) {
                report(app, state, &nonce, msg);
            }
        }
        _ => {}
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str(" …");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A message shaped exactly as the injected script queues it.
    fn queued(path: &str, body: serde_json::Value) -> serde_json::Value {
        json!({ "path": path, "body": body })
    }

    #[test]
    fn every_path_the_injected_script_uses_is_one_dispatch_understands() {
        // This is the contract between JavaScript in someone else's page and
        // Rust, and nothing but this test holds the two halves together. A
        // path renamed on one side goes quiet rather than failing loudly.
        let script = include_str!("canvas.rs");
        for path in ["/p/scroll", "/p/console", "/p/ready", "/p/wheel", "/p/nav", "/p/report"] {
            assert!(
                script.contains(&format!("post(\"{path}\"")),
                "nothing in the injected script sends {path}"
            );
        }
    }

    #[test]
    fn a_queued_scroll_report_parses() {
        let message = queued("/p/scroll", json!({ "panel": "md-768", "nonce": "n", "pct": 0.42 }));
        let parsed: Scroll = serde_json::from_value(message["body"].clone()).unwrap();
        assert_eq!(parsed.panel, "md-768");
        assert_eq!(parsed.pct, 0.42);
    }

    #[test]
    fn a_queued_console_line_parses() {
        let message = queued(
            "/p/console",
            json!({ "panel": "md-768", "nonce": "n", "level": "error", "text": "boom" }),
        );
        let parsed: Console = serde_json::from_value(message["body"].clone()).unwrap();
        assert_eq!(parsed.level, "error");
        assert_eq!(parsed.text, "boom");
    }

    #[test]
    fn a_queued_ready_report_parses_even_with_the_optional_parts_missing() {
        let message = queued("/p/ready", json!({ "panel": "md-768", "nonce": "n" }));
        let parsed: Ready = serde_json::from_value(message["body"].clone()).unwrap();
        assert_eq!(parsed.panel, "md-768");
        assert_eq!(parsed.title, "");
        assert_eq!(parsed.width, 0.0);
    }

    #[test]
    fn a_queued_wheel_delta_parses() {
        let message = queued("/p/wheel", json!({ "panel": "md-768", "nonce": "n", "dx": -120.5 }));
        let parsed: Wheel = serde_json::from_value(message["body"].clone()).unwrap();
        assert_eq!(parsed.dx, -120.5);
    }

    #[test]
    fn a_message_without_a_nonce_is_not_from_a_panel_we_spawned() {
        // The page's own code could reach the loopback server too, so every
        // message has to carry the nonce that was baked into what we injected.
        let body = json!({ "panel": "md-768", "pct": 0.5 });
        assert!(serde_json::from_value::<Scroll>(body).is_err());
    }

    #[test]
    fn a_page_that_logs_a_novel_is_cut_down_to_something_readable() {
        let long = "x".repeat(5000);
        let cut = truncate(&long, 2000);
        assert_eq!(cut.chars().count(), 2002, "2000 characters plus the ellipsis");
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_short_line_is_left_exactly_as_it_was() {
        assert_eq!(truncate("all fine", 2000), "all fine");
    }

    #[test]
    fn truncating_counts_characters_rather_than_bytes() {
        // Slicing a multi-byte character in half panics, and a page can log
        // anything at all.
        let emoji = "🎯".repeat(50);
        let cut = truncate(&emoji, 10);
        assert_eq!(cut.chars().count(), 12);
    }
}
