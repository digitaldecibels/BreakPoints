//! The agent bridge: a local HTTP API with an MCP endpoint on top.
//!
//! This is a server that runs JavaScript in whatever page you have loaded and
//! reads files out of your project, so it is off by default, binds to loopback
//! only, and can require a token. The toolbar shows a dot whenever it is live.
//!
//! The HTTP API is the real surface. Both transports are thin wrappers over
//! `call_tool`, which means you can curl it while debugging.

use std::collections::HashMap;
use std::net::TcpListener as StdListener;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as UrlPath, Query, State as AxumState,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::state::Shared;
use crate::{access, audit, canvas, config, model, project, references, shots, tools, util};

pub const PORT: u16 = 7333;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub url: String,
    pub mcp_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// True while an agent request is in flight, which is what makes the
    /// toolbar dot pulse rather than sit still.
    pub active: bool,
    /// True while a session is holding a socket or a long poll open, waiting
    /// for the next note. Steady rather than pulsing, because it is a state
    /// and not an event, and it is the difference between a note being
    /// delivered and a note sitting in a queue.
    pub watching: bool,
}

pub fn status(_app: &AppHandle, state: &Shared) -> Status {
    let config = state.config.lock().unwrap();
    Status {
        enabled: config.agent_bridge,
        url: format!("http://127.0.0.1:{PORT}"),
        mcp_url: format!("http://127.0.0.1:{PORT}/mcp"),
        token: config.bridge_token.clone(),
        active: *state.bridge_active.lock().unwrap(),
        watching: state.watching(),
    }
}

#[derive(Clone)]
struct Ctx {
    app: AppHandle,
    state: Shared,
    token: Option<String>,
}

pub async fn set_enabled(app: &AppHandle, state: &Shared, on: bool) -> Result<Status, String> {
    if !on {
        if let Some(stop) = state.bridge_stop.lock().unwrap().take() {
            let _ = stop.send(());
        }
        let mut config = state.config.lock().unwrap();
        config.agent_bridge = false;
        let _ = config::save(app, &config);
        drop(config);
        let status = status(app, state);
        let _ = app.emit("bridge:status", &status);
        return Ok(status);
    }

    if state.bridge_stop.lock().unwrap().is_some() {
        return Ok(status(app, state));
    }

    let token = {
        let mut config = state.config.lock().unwrap();
        if config.bridge_token.is_none() {
            config.bridge_token = Some(util::random_token(16));
        }
        config.agent_bridge = true;
        let _ = config::save(app, &config);
        config.bridge_token.clone()
    };

    // Loopback only. Never 0.0.0.0, whatever else changes here.
    let listener = StdListener::bind(("127.0.0.1", PORT))
        .map_err(|e| format!("could not listen on 127.0.0.1:{PORT}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let ctx = Arc::new(Ctx {
        app: app.clone(),
        state: state.clone(),
        token,
    });

    let router = Router::new()
        .route("/health", get(health))
        .route("/tools", get(tool_list))
        .route("/api/:tool", post(http_tool))
        .route("/ws/reports", get(reports_ws))
        .route("/mcp", post(mcp))
        .with_state(ctx);

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    *state.bridge_stop.lock().unwrap() = Some(stop_tx);

    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => {
                let served = axum::serve(listener, router).with_graceful_shutdown(async {
                    let _ = stop_rx.await;
                });
                if let Err(err) = served.await {
                    eprintln!("[breakpoints] agent bridge stopped: {err}");
                }
            }
            Err(err) => eprintln!("[breakpoints] could not adopt bridge listener: {err}"),
        }
    });

    let status = status(app, state);
    let _ = app.emit("bridge:status", &status);
    Ok(status)
}

fn authorised(ctx: &Ctx, headers: &HeaderMap) -> bool {
    let Some(token) = &ctx.token else { return true };
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_start_matches("Bearer ").trim() == token)
        .unwrap_or(false)
        || headers
            .get("x-breakpoints-token")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == token)
            .unwrap_or(false)
}

async fn health(AxumState(ctx): AxumState<Arc<Ctx>>) -> impl IntoResponse {
    Json(json!({
        "name": "breakpoints",
        "version": env!("CARGO_PKG_VERSION"),
        "panels": tools::list_panels(&ctx.state).len(),
        "project": tools::get_project_info(&ctx.state).name,
    }))
}

async fn tool_list() -> impl IntoResponse {
    Json(json!({ "tools": tool_definitions() }))
}

async fn http_tool(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    UrlPath(tool): UrlPath<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> impl IntoResponse {
    if !authorised(&ctx, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "bad or missing token" })));
    }
    let args = body.map(|Json(value)| value).unwrap_or_else(|| json!({}));
    match dispatch(&ctx, &tool, &args).await {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(err) => (StatusCode::BAD_REQUEST, Json(json!({ "error": err }))),
    }
}

/// A session watching for notes, pushed rather than polled.
///
/// The app used to be unable to reach into a session at all, so the skill that
/// drives it ran a shell loop asking every two seconds. That is no longer
/// necessary: a watching agent can hold a socket open and be handed each note
/// at the moment it is written.
///
/// The token rides in the query string rather than a header, because the tool
/// that opens this socket cannot set headers. That is only acceptable because
/// the listener is loopback-only; never expose this beyond 127.0.0.1.
async fn reports_ws(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    Query(params): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Some(token) = &ctx.token {
        if params.get("token").map(|given| given != token).unwrap_or(true) {
            return (StatusCode::UNAUTHORIZED, "bad or missing token").into_response();
        }
    }
    let client = params.get("client").cloned();
    let name = params.get("name").cloned().or_else(|| client.clone());
    ws.on_upgrade(move |socket| watch_reports(ctx, socket, client, name))
}

/// Hand over everything waiting for this session, one note per frame.
///
/// The frame is the note's own prose, built in Rust, so a session does not have
/// to know the shape of anything to read it. Returns false once the socket has
/// gone.
///
/// Delivery takes notes out of the queue, so a send that fails has to put the
/// rest back. Without that, a note written in the seconds after a watcher went
/// away was drained for a socket that could not carry it and then dropped,
/// which is the stranding this whole change exists to stop.
async fn send_waiting<S>(sink: &mut S, app: &AppHandle, state: &Shared, client: Option<&str>) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    let waiting = drain_reports(state, client);
    if waiting.is_empty() {
        return true;
    }
    for (sent, report) in waiting.iter().enumerate() {
        if sink.send(Message::Text(report.text.clone())).await.is_err() {
            // Settle first, then put back only what did not go. Without the
            // settle, the unsent notes would be in the queue and in flight at
            // once, and the caller would eventually be handed them twice.
            if let Some(client) = client {
                state.settle_in_flight(Some(client));
            }
            state.requeue_reports(waiting[sent..].to_vec());
            tools::emit_report_count(app, state);
            return false;
        }
    }
    // A send that returned is delivery, so there is nothing left to hold. The
    // in-flight list exists for a reply that might never arrive, which is a
    // problem the socket does not have.
    if let Some(client) = client {
        state.settle_in_flight(Some(client));
    }
    tools::emit_report_count(app, state);
    true
}

async fn watch_reports(
    ctx: Arc<Ctx>,
    socket: WebSocket,
    client: Option<String>,
    name: Option<String>,
) {
    let app = ctx.app.clone();
    let state = ctx.state.clone();

    // Connecting is the claim. There is no separate request to make, so the
    // toolbar names the session the moment it starts listening, and a session
    // cannot end up watching for notes that are being addressed elsewhere.
    if let (Some(id), Some(name)) = (client.clone(), name) {
        let session = state.claim_reports(crate::state::ClientSession {
            id,
            name,
            at: util::now_ms(),
        });
        let _ = app.emit("reports:owner", &session);
    }

    // Subscribe before the first drain, or a note written in between is
    // delivered to nobody.
    let mut rx = state.subscribe_reports();
    state.watcher_joined();
    let _ = app.emit("bridge:status", status(&app, &state));
    tools::emit_report_count(&app, &state);

    // Reading and writing at once, because a watcher going away has to be
    // noticed at once rather than at the next thing we try to send. The app
    // suppresses the clipboard while somebody is listening, so a stale
    // "listening" is a note that goes nowhere at all.
    let (mut sink, mut stream) = socket.split();
    let (closed, mut has_closed) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        // Nothing a watcher sends means anything. Reading is only how the
        // close is seen.
        while let Some(Ok(_)) = stream.next().await {}
        let _ = closed.send(());
    });

    // Whatever was written while this session was away goes first.
    let mut alive = send_waiting(&mut sink, &app, &state, client.as_deref()).await;

    while alive {
        tokio::select! {
            _ = &mut has_closed => break,
            received = rx.recv() => match received {
                // The broadcast is only a wake-up. The queue is what is
                // actually delivered, so two notes in quick succession are one
                // drain and the second wake-up finds nothing left to do.
                Ok(_) | Err(RecvError::Lagged(_)) => {
                    alive = send_waiting(&mut sink, &app, &state, client.as_deref()).await;
                }
                Err(RecvError::Closed) => break,
            },
            // A half-open connection, where the other end went away without
            // saying so, is only found by writing to it.
            _ = tokio::time::sleep(Duration::from_secs(20)) => {
                alive = sink.send(Message::Ping(Vec::new())).await.is_ok();
            }
        }
    }

    state.watcher_left();
    let _ = app.emit("bridge:status", status(&app, &state));
    tools::emit_report_count(&app, &state);
}

/// Tools that must not pulse the toolbar dot.
///
/// The dot means "an agent is doing something", and a watcher asking for notes
/// is not doing anything. A two second poll made it blink thirty times a
/// minute for as long as a session was open, which trained the eye to ignore
/// the one signal that says the bridge is in use.
const QUIET_TOOLS: &[&str] = &["take_reports", "await_reports", "claim_reports"];

/// MCP over HTTP, which is JSON-RPC 2.0 with three methods that matter.
async fn mcp(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> impl IntoResponse {
    if !authorised(&ctx, &headers) {
        return Json(rpc_error(request.get("id").cloned(), -32001, "bad or missing token"));
    }

    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => Json(rpc_ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "breakpoints", "version": env!("CARGO_PKG_VERSION") },
            }),
        )),
        "notifications/initialized" => Json(json!({ "jsonrpc": "2.0" })),
        "ping" => Json(rpc_ok(id, json!({}))),
        "tools/list" => Json(rpc_ok(id, json!({ "tools": tool_definitions() }))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match dispatch(&ctx, &name, &args).await {
                Ok(value) => Json(rpc_ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&value).unwrap_or_default(),
                        }],
                        "isError": false,
                    }),
                )),
                Err(err) => Json(rpc_ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": err }],
                        "isError": true,
                    }),
                )),
            }
        }
        other => Json(rpc_error(id, -32601, &format!("no method {other}"))),
    }
}

fn rpc_ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Option<Value>, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Run a tool, with the toolbar dot lit for as long as it takes.
async fn dispatch(ctx: &Ctx, tool: &str, args: &Value) -> Result<Value, String> {
    let pulse = !QUIET_TOOLS.contains(&tool);
    if pulse {
        *ctx.state.bridge_active.lock().unwrap() = true;
        let _ = ctx.app.emit("bridge:status", status(&ctx.app, &ctx.state));
    }

    let result = call_tool(&ctx.app, &ctx.state, tool, args).await;

    if pulse {
        *ctx.state.bridge_active.lock().unwrap() = false;
        let _ = ctx.app.emit("bridge:status", status(&ctx.app, &ctx.state));
    }
    result
}

/// Take the notes a caller is entitled to: its own, plus any addressed to
/// nobody. See `AppState::take_reports_for` for why the second half matters.
fn drain_reports(state: &Shared, client: Option<&str>) -> Vec<crate::state::Report> {
    match client {
        Some(client) => state.take_reports_for(client),
        None => state.take_reports(),
    }
}

fn string_arg(args: &Value, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        // `panel` is documented as "index or name", so an agent reasonably
        // sends the index as a number. Refusing that with "panel is required"
        // sends it looking for an argument it already passed.
        Some(Value::Number(number)) => Ok(number.to_string()),
        Some(other) => Err(format!(
            "{key} should be text, not {}",
            match other {
                Value::Bool(_) => "a boolean",
                Value::Array(_) => "a list",
                Value::Object(_) => "an object",
                _ => "null",
            }
        )),
        None => Err(format!("{key} is required")),
    }
}

/// Every tool this server answers to.
///
/// Written out rather than left implied by the match below, so a test can hold
/// it against `tool_definitions`. A tool that exists in the dispatch but not in
/// the definitions is invisible to an agent, and one that exists in the
/// definitions but not the dispatch is worse: it is advertised and then errors.
pub const TOOL_NAMES: &[&str] = &[
    "list_panels",
    "navigate",
    "reload",
    "eval_js",
    "get_console",
    "get_dom",
    "set_viewport",
    "screenshot_panel",
    "screenshot_all",
    "detect_project",
    "scan_breakpoints",
    "get_project_info",
    "get_breakpoint_sources",
    "get_scan_log",
    "generate_project_config",
    "write_project_file",
    "list_profiles",
    "load_profile",
    "audit_all",
    "audit_accessibility",
    "verify_breakpoints",
    "attach_reference",
    "get_references",
    "diff_panel",
    "take_reports",
    "await_reports",
    "claim_reports",
    "eval_chrome",
];

/// Every tool, in one place. The chrome's commands call the same functions.
pub async fn call_tool(
    app: &AppHandle,
    state: &Shared,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool {
        // Canvas
        "list_panels" => Ok(json!(tools::list_panels(state))),

        "navigate" => {
            let url = string_arg(args, "url")?;
            canvas::navigate_all(state, &url)?;
            canvas::emit_canvas(app, state);
            let status = project::probe(&url).await;
            *state.url_status.lock().unwrap() = status.clone();
            let _ = app.emit("url:status", &status);
            Ok(json!({ "url": url, "status": status }))
        }

        "reload" => {
            canvas::reload_all(state);
            Ok(json!({ "reloaded": tools::list_panels(state).len() }))
        }

        "eval_js" => {
            let panel = string_arg(args, "panel")?;
            let script = string_arg(args, "script")?;
            tools::eval_js(state, &panel, &script).await
        }

        "eval_chrome" => {
            let script = string_arg(args, "script")?;
            tools::eval_chrome(app, &script).await
        }

        "audit_accessibility" => {
            let report = access::audit_all(state).await?;
            Ok(serde_json::to_value(report).map_err(|e| e.to_string())?)
        }

        "verify_breakpoints" => {
            let panel = args
                .get("panel")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "0".to_string());
            let report = crate::verify::verify(state, &panel).await?;
            Ok(serde_json::to_value(report).map_err(|e| e.to_string())?)
        }

        "take_reports" => {
            // Addressed if the caller says who it is, which is what
            // `/run-breakpoints` sets up. Unaddressed keeps the old behaviour
            // for a single session that never claimed anything.
            let reports = drain_reports(state, args.get("client").and_then(Value::as_str));
            tools::emit_report_count(app, state);
            Ok(json!(reports))
        }

        // The same as `take_reports`, except that it waits rather than coming
        // back empty. One call replaces a polling loop, and a note arrives the
        // moment it is written instead of up to a poll interval later.
        "await_reports" => {
            let client = args
                .get("client")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let seconds = args
                .get("timeout")
                .and_then(Value::as_f64)
                .unwrap_or(25.0)
                .clamp(1.0, 60.0);

            // Subscribe before looking, or a note written between the two is
            // waited past.
            let mut rx = state.subscribe_reports();
            let mut out = drain_reports(state, client.as_deref());

            if out.is_empty() {
                state.watcher_joined();
                let _ = app.emit("bridge:status", status(app, state));
                let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(seconds);
                loop {
                    match tokio::time::timeout_at(deadline, rx.recv()).await {
                        Ok(Ok(_)) | Ok(Err(RecvError::Lagged(_))) => {
                            // A note for somebody else wakes us too, and
                            // drains to nothing, so keep waiting.
                            out = drain_reports(state, client.as_deref());
                            if !out.is_empty() {
                                break;
                            }
                        }
                        Ok(Err(RecvError::Closed)) => break,
                        Err(_past_the_deadline) => break,
                    }
                }
                state.watcher_left();
                let _ = app.emit("bridge:status", status(app, state));
            }

            if !out.is_empty() {
                tools::emit_report_count(app, state);
            }
            Ok(json!(out))
        }

        "claim_reports" => {
            let id = string_arg(args, "id")?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let session = state.claim_reports(crate::state::ClientSession {
                id,
                name,
                at: crate::util::now_ms(),
            });
            let _ = app.emit("reports:owner", &session);
            tools::emit_report_count(app, state);
            Ok(json!(session))
        }

        "get_console" => {
            let panel = string_arg(args, "panel")?;
            Ok(json!(tools::get_console(state, &panel)?))
        }

        "get_dom" => {
            let panel = string_arg(args, "panel")?;
            let selector = string_arg(args, "selector")?;
            let max = args
                .get("maxChars")
                .and_then(Value::as_u64)
                .unwrap_or(8000) as usize;
            Ok(json!({ "html": tools::get_dom(state, &panel, &selector, max).await? }))
        }

        "set_viewport" => {
            let panel = string_arg(args, "panel")?;
            let width = args.get("width").and_then(Value::as_f64);
            let height = args.get("height").and_then(Value::as_f64);
            Ok(json!(tools::set_viewport(app, state, &panel, width, height)?))
        }

        "screenshot_panel" => {
            let panel = string_arg(args, "panel")?;
            let full_page = args.get("fullPage").and_then(Value::as_bool).unwrap_or(false);
            let path = shots::capture(app, state, &panel, full_page).await?;
            Ok(json!({ "path": path }))
        }

        "screenshot_all" => {
            let paths = shots::capture_all(app, state).await?;
            Ok(json!({
                "paths": paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()
            }))
        }

        // Project intelligence
        "detect_project" => {
            let path = string_arg(args, "path")?;
            let opened = project::open(app, state, std::path::PathBuf::from(path)).await?;
            // A person gets the scan sheet and decides. An agent asked for a
            // working environment, so unless it says otherwise it gets one.
            let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(true);

            if apply {
                let viewports: Vec<model::Viewport> = match &opened.project_file {
                    // A project file exists, so it wins. Full stop.
                    Some(file) => file.viewports.clone(),
                    None => opened
                        .recommendation
                        .recommended
                        .iter()
                        .map(|candidate| candidate.viewport.clone())
                        .collect(),
                };
                // If this project has no URL of its own, the panels open
                // blank. Falling back to whatever the canvas was showing
                // pointed a freshly opened project at the *previous* one's
                // site, which then got written into its breakpoints.md.
                let url = opened
                    .project_file
                    .as_ref()
                    .and_then(|f| f.url.clone())
                    .or_else(|| opened.dev_url.clone())
                    .unwrap_or_else(|| "about:blank".to_string());

                if !viewports.is_empty() {
                    project::apply(app, state, viewports, url).await?;
                }
                let _ = app.emit("project:applied", &opened);
            } else {
                let _ = app.emit("project:opened", &opened);
            }

            let mut value = serde_json::to_value(&opened).map_err(|e| e.to_string())?;
            value["applied"] = json!(apply);
            value["panels"] = json!(tools::list_panels(state));
            Ok(value)
        }

        "scan_breakpoints" => {
            let report = tools::scan_breakpoints(app, state).await?;
            Ok(serde_json::to_value(report).map_err(|e| e.to_string())?)
        }

        "get_project_info" => {
            Ok(serde_json::to_value(tools::get_project_info(state)).map_err(|e| e.to_string())?)
        }

        "get_breakpoint_sources" => Ok(json!(tools::get_breakpoint_sources(state))),

        "get_scan_log" => Ok(json!({ "entries": tools::get_scan_log(state)? })),

        "generate_project_config" => Ok(
            serde_json::to_value(tools::generate_project_config(state)?).map_err(|e| e.to_string())?,
        ),

        "write_project_file" => {
            let viewports: Option<Vec<model::Viewport>> = args
                .get("viewports")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let path = tools::write_project_file(app, state, viewports)?;
            // Writing the file is what makes it the source of truth, so the
            // panels reload from it straight away.
            let current: Vec<model::Viewport> = state
                .canvas
                .lock()
                .unwrap()
                .panels
                .iter()
                .map(|p| p.viewport.clone())
                .collect();
            let url = state.canvas.lock().unwrap().url.clone();
            canvas::spawn(app, state, current, &url).await?;
            Ok(json!({ "path": path }))
        }

        "list_profiles" => Ok(json!(tools::list_profiles(state))),

        "load_profile" => {
            let id = string_arg(args, "id")?;
            let viewports = {
                let mut config = state.config.lock().unwrap();
                let profile = config.profiles.get(&id).ok_or("no such profile")?.clone();
                config.active_profile = id.clone();
                let _ = config::save(app, &config);
                profile.viewports
            };
            let url = state.canvas.lock().unwrap().url.clone();
            let total = canvas::spawn(app, state, viewports.clone(), &url).await?;
            Ok(json!({ "profile": id, "viewports": viewports, "totalWidth": total }))
        }

        // Composite
        "audit_all" => audit::run(app, state).await,

        // References
        "attach_reference" => {
            let panel = string_arg(args, "panel")?;
            let reference = string_arg(args, "reference")?;
            references::attach(app, state, &panel, &reference)
        }

        "get_references" => Ok(json!({
            "panels": references::list(state),
            "fromProjectFile": references::from_project_file(state),
        })),

        "diff_panel" => {
            let panel = string_arg(args, "panel")?;
            references::diff(app, state, &panel).await
        }

        other => Err(format!(
            "no tool called {other}. GET /tools lists what there is."
        )),
    }
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn text(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

pub fn tool_definitions() -> Vec<Value> {
    let panel = || text("Which panel. A name, a framework key, or a breakpoint width all work, case-insensitively and by prefix: \"Tablet\", \"md\" and \"768\" reach the same panel. A bare number under the panel count is a zero-based index; anything larger is read as a width. When someone says \"frame 2\" or \"the second one\" they mean the panel whose `position` is 2 in list_panels, which is counted from the left starting at one.");

    vec![
        json!({
            "name": "list_panels",
            "description": "Every open panel, left to right: `position` counted from one, id, name, framework key, declared size, on-screen scale and load state. Read this first when someone names a panel by where it sits rather than what it is called.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "navigate",
            "description": "Point every panel at a URL.",
            "inputSchema": schema(json!({ "url": text("The URL to load. A bare host becomes https.") }), &["url"]),
        }),
        json!({
            "name": "reload",
            "description": "Reload every panel.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "eval_js",
            "description": "Run JavaScript in one panel and return the value. The script body is wrapped in a function, so use `return`.",
            "inputSchema": schema(json!({ "panel": panel(), "script": text("JavaScript to run. Use return to send a value back.") }), &["panel", "script"]),
        }),
        json!({
            "name": "get_console",
            "description": "Recent console output and page errors from one panel, collected since it loaded.",
            "inputSchema": schema(json!({ "panel": panel() }), &["panel"]),
        }),
        json!({
            "name": "get_dom",
            "description": "Outer HTML of the first element matching a selector, truncated.",
            "inputSchema": schema(json!({ "panel": panel(), "selector": text("A CSS selector."), "maxChars": json!({ "type": "number" }) }), &["panel", "selector"]),
        }),
        json!({
            "name": "set_viewport",
            "description": "Change one panel's size without touching the others.",
            "inputSchema": schema(json!({ "panel": panel(), "width": json!({ "type": "number" }), "height": json!({ "type": "number" }) }), &["panel"]),
        }),
        json!({
            "name": "screenshot_panel",
            "description": "PNG of one panel, cropped from the app window. Returns a file path.",
            "inputSchema": schema(json!({
                "panel": panel(),
                "fullPage": json!({
                    "type": "boolean",
                    "description": "Capture the whole scroll height rather than what is on screen, by walking the page a screenful at a time and stitching. Sticky headers are hidden after the first screenful so they do not repeat. Slower, and a page that grows as you scroll stops after twenty screenfuls.",
                }),
            }), &["panel"]),
        }),
        json!({
            "name": "screenshot_all",
            "description": "One PNG per panel. Returns file paths, left to right.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "detect_project",
            "description": "Point the app at a folder, scan it, and open the panels it recommends. Returns the framework, the breakpoints with their sources, the dev URL and any conflicts. Pass apply:false to scan without changing the canvas.",
            "inputSchema": schema(json!({
                "path": text("Absolute path to the project folder."),
                "apply": json!({ "type": "boolean", "description": "Open the recommended panels. Defaults to true." }),
            }), &["path"]),
        }),
        json!({
            "name": "scan_breakpoints",
            "description": "Re-run breakpoint detection on the open project and return the report.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "get_project_info",
            "description": "The open project: root, framework, dev URL, whether breakpoints.md is driving it, and the current panels.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "get_breakpoint_sources",
            "description": "Where each active viewport came from: file, line, confidence and how many files use it. Use this to find the config line to edit.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "get_scan_log",
            "description": "Structured log of the last scan: which detectors ran, what was skipped, and every parse failure with the reason.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "generate_project_config",
            "description": "Turn the last scan into a viewport set without writing anything.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "write_project_file",
            "description": "Write breakpoints.md into the project root and reload the panels from it. Prose already in the file is preserved.",
            "inputSchema": schema(json!({ "viewports": json!({ "type": "array", "description": "Optional. Defaults to the panels currently open." }) }), &[]),
        }),
        json!({
            "name": "list_profiles",
            "description": "Saved viewport sets.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "load_profile",
            "description": "Switch to a saved viewport set.",
            "inputSchema": schema(json!({ "id": text("Profile id from list_profiles.") }), &["id"]),
        }),
        json!({
            "name": "audit_all",
            "description": "For every panel: capture console errors and run layout probes for overflow, overlap, small type, upscaled images, tap targets and viewport-hungry fixed elements. Returns one report ranked by severity. Run it again after a fix.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "attach_reference",
            "description": "Bind a Figma node URL or a local image under .breakpoints/refs/ to a viewport, and write it into breakpoints.md so the mapping commits with the repo.",
            "inputSchema": schema(json!({ "panel": panel(), "reference": text("figma:FILE?node-id=1-2, or a repo-relative image path.") }), &["panel", "reference"]),
        }),
        json!({
            "name": "get_references",
            "description": "Which reference frame belongs to which panel.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "verify_breakpoints",
            "description": "Read the media queries out of a panel's live stylesheet and compare them to the widths the row is open at. Everything else in this app infers breakpoints from source; this asks the browser that actually parsed the CSS. Says which open widths the page really changes at, and which widths the page changes at that no panel covers. Needs a panel pointed at a running site.",
            "inputSchema": schema(
                json!({
                    "panel": {
                        "type": "string",
                        "description": "Index or name. Defaults to the first panel, since every panel loads the same stylesheets.",
                    }
                }),
                &[],
            ),
        }),
        json!({
            "name": "audit_accessibility",
            "description": "Run an accessibility audit in every panel with axe-core and return the violations grouped by rule, each carrying the widths it was broken at. Rules broken at some widths and not others are marked `widthSpecific` and ranked first within their impact, because those are the ones a tool that tests one width cannot see. Ranked worst impact first. Takes a few seconds per panel.",
            "inputSchema": schema(json!({}), &[]),
        }),
        json!({
            "name": "take_reports",
            "description": "Problems a person marked in a panel by pointing at an element and describing what is wrong, each with the breakpoint width it happened at. Each note carries a `text` field that is the whole thing as prose, already formatted. This empties the list, so what comes back is only what has not been handled yet. Call it when asked about reported problems, notes, or what is broken. Pass the same `client` id you claimed with, or you will collect notes addressed to somebody else's session. To wait for the next note rather than checking for one, use await_reports.",
            "inputSchema": schema(
                json!({
                    "client": {
                        "type": "string",
                        "description": "The session id passed to claim_reports. Omit only if this is the one and only session using the app.",
                    }
                }),
                &[],
            ),
        }),
        json!({
            "name": "await_reports",
            "description": "The same as take_reports, except that it waits for the next note rather than coming back empty. Hold this open instead of polling: it returns the moment somebody writes a note, or an empty list when the timeout passes, and you call it again. Pass the same `client` id you claimed with.",
            "inputSchema": schema(
                json!({
                    "client": {
                        "type": "string",
                        "description": "The session id passed to claim_reports. Omit only if this is the one and only session using the app.",
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Seconds to wait before coming back empty. Default 25, maximum 60.",
                    }
                }),
                &[],
            ),
        }),
        json!({
            "name": "eval_chrome",
            "description": "Run JavaScript inside Break/Points' own interface, the toolbar and label strip and sheets, rather than inside a panel. Use it to measure the app's own layout when the app itself is what is misbehaving. Panels are eval_js; this is everything around them. The script is a function body, so it needs a return.",
            "inputSchema": schema(
                json!({
                    "script": {
                        "type": "string",
                        "description": "A function body. Must `return` the value you want back.",
                    }
                }),
                &["script"],
            ),
        }),
        json!({
            "name": "claim_reports",
            "description": "Address every problem reported from now on to this agent session, so several sessions can use the app at once without collecting each other's notes. The last session to claim wins. Call it once at the start of a session, then pass the same id to take_reports.",
            "inputSchema": schema(
                json!({
                    "id": {
                        "type": "string",
                        "description": "A stable id for this session. Claude Code puts one in CLAUDE_CODE_SESSION_ID.",
                    },
                    "name": {
                        "type": "string",
                        "description": "What to call this session in the toolbar, usually the project folder. Defaults to the id.",
                    }
                }),
                &["id"],
            ),
        }),
        json!({
            "name": "diff_panel",
            "description": "Screenshot a panel, compare it to its local reference image, and return a difference image and a rough figure. The figure is a pointer, not a verdict.",
            "inputSchema": schema(json!({ "panel": panel() }), &["panel"]),
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn defined_names() -> BTreeSet<String> {
        tool_definitions()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
            .collect()
    }

    #[test]
    fn every_tool_is_both_dispatched_and_advertised() {
        let declared: BTreeSet<String> = TOOL_NAMES.iter().map(|n| n.to_string()).collect();
        assert_eq!(
            declared,
            defined_names(),
            "the dispatch list and the tool definitions have drifted apart"
        );
    }

    #[test]
    fn the_tool_list_has_no_duplicates() {
        assert_eq!(TOOL_NAMES.len(), defined_names().len());
    }

    #[test]
    fn every_required_argument_is_one_the_schema_describes() {
        // A required field with no matching property tells an agent to send
        // something it has no description of, and the mistake is a typo that
        // nothing else would catch.
        for tool in tool_definitions() {
            let name = tool["name"].as_str().unwrap().to_string();
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{name} schema is not an object");
            let properties = schema["properties"].as_object().unwrap();
            for required in schema["required"].as_array().unwrap() {
                let field = required.as_str().unwrap();
                assert!(
                    properties.contains_key(field),
                    "{name} requires \"{field}\" but never describes it"
                );
            }
        }
    }

    #[test]
    fn every_tool_says_what_it_is_for() {
        for tool in tool_definitions() {
            let name = tool["name"].as_str().unwrap();
            let description = tool["description"].as_str().unwrap_or("");
            assert!(
                !description.trim().is_empty(),
                "{name} is advertised with no description, so an agent has to guess"
            );
        }
    }

    #[test]
    fn a_panel_index_sent_as_a_number_is_accepted() {
        // The schema calls it "Panel index or name", so an agent sends 0. That
        // used to come back as "panel is required", which sends it looking for
        // an argument it had already passed.
        let args = json!({ "panel": 0 });
        assert_eq!(string_arg(&args, "panel").unwrap(), "0");
        let args = json!({ "panel": 1024 });
        assert_eq!(string_arg(&args, "panel").unwrap(), "1024");
    }

    #[test]
    fn a_string_argument_comes_back_as_written() {
        let args = json!({ "panel": "Tablet" });
        assert_eq!(string_arg(&args, "panel").unwrap(), "Tablet");
    }

    #[test]
    fn a_missing_argument_and_a_wrong_typed_one_say_different_things() {
        let missing = string_arg(&json!({}), "panel").unwrap_err();
        assert!(missing.contains("required"), "{missing}");

        let wrong = string_arg(&json!({ "panel": ["a"] }), "panel").unwrap_err();
        assert!(wrong.contains("should be text"), "{wrong}");
        assert!(!wrong.contains("required"), "it was passed, just wrongly");
    }
}
