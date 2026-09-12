//! The operations, written once.
//!
//! Both the chrome's Tauri commands and the agent bridge call these, so a tool
//! an agent can reach is the same code the toolbar button runs. There is no
//! second implementation to drift.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::canvas::{self, PanelInfo};
use crate::model::Viewport;
use crate::state::{ConsoleLine, Shared};
use crate::{project, project_file, scanner};

pub fn list_panels(state: &Shared) -> Vec<PanelInfo> {
    canvas::info(state).panels
}

/// Tell the window how many notes are waiting, and whether anything is
/// listening for the next one.
///
/// The chrome used to keep this number itself, counting up on every new note
/// and down only when its own copy button was pressed. A session collecting
/// over the bridge never told it, so the badge sat there claiming notes that
/// had already been handled, and pressing it said "Nothing reported yet"
/// without clearing the number. Emitting the real count from the one place
/// that knows it means the two can never disagree.
pub fn emit_report_count(app: &AppHandle, state: &Shared) {
    let _ = app.emit(
        "reports:changed",
        json!({
            "count": state.report_count(),
            "watching": state.watching(),
        }),
    );
}

fn require_panel(state: &Shared, needle: &str) -> Result<String, String> {
    canvas::resolve_id(state, needle).ok_or_else(|| {
        let names: Vec<String> = list_panels(state)
            .into_iter()
            .map(|p| format!("{} ({})", p.name, p.source))
            .collect();
        // With nothing open, "Open panels: none" reads as a list that happens
        // to be empty and leaves an agent guessing what to do about it.
        if names.is_empty() {
            return "no panels are open, so there is nothing to address. Open a project or navigate to a URL first.".to_string();
        }
        format!("no panel matches \"{needle}\". Open panels: {}", names.join(", "))
    })
}

/// Wrap a script so an exception comes back as a message rather than
/// vanishing, and so a promise says so instead of serialising to `{}`.
///
/// A promise cannot be waited for here: the webview hands back whatever the
/// expression evaluated to. Saying so beats returning an empty object and
/// letting the caller conclude the page had nothing to say.
pub fn wrap_script(script: &str) -> String {
    format!(
        "(function () {{ try {{ var value = (function () {{ {script} }})(); if (value && typeof value.then === \"function\") {{ return JSON.stringify({{ ok: false, error: \"eval_js cannot wait for a promise. Assign the result to a window property inside .then(), then read that property with a second eval_js.\" }}); }} return JSON.stringify({{ ok: true, value: value }}); }} catch (e) {{ return JSON.stringify({{ ok: false, error: String((e && e.message) || e) }}); }} }})()"
    )
}

/// Run JavaScript in one panel and get the value back.
///
/// Takes the panel's id, already resolved. The pump calls this several hundred
/// times a second at its fastest rate, and resolving a name it was already
/// given meant cloning every viewport in the row to match one id.
pub async fn eval_wrapped(state: &Shared, id: &str, wrapped: &str) -> Result<Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();

    {
        let canvas = state.canvas.lock().unwrap();
        let target = canvas
            .panels
            .iter()
            .find(|p| p.viewport.id == id)
            .ok_or("panel disappeared")?;

        // The callback is Fn, so the sender has to be takeable from inside it.
        let slot = Mutex::new(Some(tx));
        target
            .webview
            .eval_with_callback(wrapped.to_string(), move |result| {
                if let Some(tx) = slot.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
            })
            .map_err(|e| e.to_string())?;
    }

    let raw = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| format!("panel {id} did not answer within 10 seconds"))?
        .map_err(|_| "panel closed before it answered".to_string())?;

    unwrap_envelope(&raw)
}

/// The same, for a caller that has a panel's name or index rather than its id.
pub async fn eval_js(state: &Shared, panel: &str, script: &str) -> Result<Value, String> {
    let id = require_panel(state, panel)?;
    eval_wrapped(state, &id, &wrap_script(script)).await
}

/// Run JavaScript in Break/Points' own chrome and hand back the value.
///
/// The twin of `eval_js`, for the one surface that was not reachable. Every
/// page in a panel could be measured from outside the app while the toolbar,
/// the label strip and the sheets could only be looked at, so a question like
/// "where is the label strip actually drawn" had no answer that did not involve
/// asking a person to read it off their own screen.
///
/// This is the app's own code, not somebody's page, so there is no nonce and no
/// callback server: the chrome is a webview we built and serve.
pub async fn eval_chrome(app: &AppHandle, script: &str) -> Result<Value, String> {
    let chrome = app.get_webview("chrome").ok_or("no chrome webview")?;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();

    // Same envelope as `eval_js`, and for the same reason: a bare expression
    // comes back as a doubly encoded string and a rejected promise comes back
    // as an empty object.
    let wrapped = format!(
        "(function () {{ try {{ var value = (function () {{ {script} }})(); if (value && typeof value.then === \"function\") {{ return JSON.stringify({{ ok: false, error: \"eval_chrome cannot wait for a promise. Assign the result to a window property inside .then(), then read that property with a second call.\" }}); }} return JSON.stringify({{ ok: true, value: value }}); }} catch (e) {{ return JSON.stringify({{ ok: false, error: String((e && e.message) || e) }}); }} }})()"
    );

    let slot = Mutex::new(Some(tx));
    chrome
        .eval_with_callback(wrapped, move |result| {
            if let Some(tx) = slot.lock().unwrap().take() {
                let _ = tx.send(result);
            }
        })
        .map_err(|e| e.to_string())?;

    let raw = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| "the chrome did not answer within 10 seconds".to_string())?
        .map_err(|_| "the chrome closed before it answered".to_string())?;

    unwrap_envelope(&raw)
}

/// Unpack what the webview handed back.
///
/// Tauri serialises the result and the injected wrapper already produced JSON,
/// so the string arrives encoded twice. Getting this wrong turns every value
/// into a quoted string, which is the kind of thing that looks fine until a
/// caller tries to read a number out of it.
pub fn unwrap_envelope(raw: &str) -> Result<Value, String> {
    let once: Value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
    let envelope: Value = match &once {
        Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| once.clone()),
        other => other.clone(),
    };

    if envelope.get("ok") == Some(&Value::Bool(false)) {
        return Err(envelope
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("script threw")
            .to_string());
    }
    Ok(envelope.get("value").cloned().unwrap_or(envelope))
}

pub fn get_console(state: &Shared, panel: &str) -> Result<Vec<ConsoleLine>, String> {
    let id = require_panel(state, panel)?;
    Ok(state
        .console
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap_or_default())
}

pub async fn get_dom(
    state: &Shared,
    panel: &str,
    selector: &str,
    max_chars: usize,
) -> Result<String, String> {
    let script = format!(
        "var el = document.querySelector({}); if (!el) return null; var html = el.outerHTML; return html.length > {max} ? html.slice(0, {max}) + '\\n… truncated' : html;",
        serde_json::to_string(selector).unwrap(),
        max = max_chars
    );
    let value = eval_js(state, panel, &script).await?;
    match value {
        Value::Null => Err(format!("nothing matches {selector} in {panel}")),
        Value::String(html) => Ok(html),
        other => Ok(other.to_string()),
    }
}

/// Change one panel's size without touching the others.
pub fn set_viewport(
    app: &AppHandle,
    state: &Shared,
    panel: &str,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<PanelInfo, String> {
    let id = require_panel(state, panel)?;
    {
        let mut canvas = state.canvas.lock().unwrap();
        let target = canvas
            .panels
            .iter_mut()
            .find(|p| p.viewport.id == id)
            .ok_or("panel disappeared")?;
        if let Some(width) = width {
            target.viewport.width = width.clamp(120.0, 5000.0);
        }
        if let Some(height) = height {
            target.viewport.height = height.clamp(200.0, 5000.0);
        }
    }
    canvas::relayout(app, state);

    // The page only volunteers its width when it loads, so after a deliberate
    // resize the last thing it said is stale and would read as a mismatch.
    // Ask it again rather than leaving a false alarm on the label.
    canvas::confirm_width(app.clone(), state.clone(), id.clone());

    list_panels(state)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "panel disappeared".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub project_driven: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_url: Option<String>,
    pub url: String,
    pub url_status: crate::model::UrlStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan_ms: Option<u64>,
    pub panels: Vec<PanelInfo>,
    /// How far the row is panned, and how wide it is. An agent that knows both
    /// knows which panels are actually on screen.
    pub scroll_x: f64,
    pub total_width: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_file: Option<String>,
}

pub fn get_project_info(state: &Shared) -> ProjectInfo {
    let project = state.project.lock().unwrap().clone();
    let canvas = canvas::info(state);
    let report = project.as_ref().and_then(|p| p.report.clone());

    ProjectInfo {
        root: project.as_ref().map(|p| p.root.to_string_lossy().to_string()),
        name: project.as_ref().map(|p| p.name.clone()),
        project_driven: project.as_ref().map(|p| p.project_driven).unwrap_or(false),
        framework: report.as_ref().and_then(|r| {
            r.frameworks.first().map(|f| match &f.version {
                Some(version) => format!("{}@{version}", f.framework),
                None => f.framework.clone(),
            })
        }),
        dev_url: project.as_ref().and_then(|p| p.dev_url.clone()),
        url: canvas.url.clone(),
        url_status: state.url_status.lock().unwrap().clone(),
        last_scan_ms: report.as_ref().map(|r| r.duration_ms),
        scroll_x: canvas.scroll_x,
        total_width: canvas.total_width,
        panels: canvas.panels,
        project_file: project
            .as_ref()
            .filter(|p| project_file::exists(&p.root))
            .map(|p| p.root.join(project_file::MARKDOWN_NAME).to_string_lossy().to_string()),
    }
}

/// Where each active viewport came from: file, line, confidence.
///
/// Quietly the most useful thing an agent can ask, because it turns "fix the
/// tablet view" into "edit line 12 of tailwind.config.js".
pub fn get_breakpoint_sources(state: &Shared) -> Vec<Value> {
    let project = state.project.lock().unwrap().clone();
    let report = project.as_ref().and_then(|p| p.report.clone());
    let panels = list_panels(state);

    panels
        .into_iter()
        .map(|panel| {
            let discovery = report.as_ref().and_then(|r| {
                r.breakpoints
                    .iter()
                    .find(|b| (b.width - panel.width).abs() < 2.0)
            });
            match discovery {
                Some(found) => json!({
                    "panel": panel.name,
                    "id": panel.id,
                    "width": panel.width,
                    "frameworkName": found.name,
                    "source": found.source,
                    "file": found.source_file,
                    "line": found.line,
                    "confidence": found.confidence,
                    "kind": found.kind,
                    "usedInFiles": found.file_count,
                }),
                None => json!({
                    "panel": panel.name,
                    "id": panel.id,
                    "width": panel.width,
                    "source": "set by hand",
                    "confidence": 1.0,
                }),
            }
        })
        .collect()
}

/// The structured log of the last scan, so an agent handed a project that
/// scanned badly can work out why the detector missed it.
pub fn get_scan_log(state: &Shared) -> Result<Vec<Value>, String> {
    let project = state.project.lock().unwrap().clone();
    let path = project
        .as_ref()
        .and_then(|p| p.report.as_ref())
        .and_then(|r| r.log_path.clone())
        .ok_or("no scan has been run in this session")?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read {path}: {e}"))?;
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// Turn a scan report into a viewport set without writing anything.
pub fn generate_project_config(state: &Shared) -> Result<generate_config::Output, String> {
    let project = state.project.lock().unwrap().clone();
    let report = project
        .and_then(|p| p.report)
        .ok_or("no project has been scanned")?;
    let (strategy, fixed, edge) = {
        let config = state.config.lock().unwrap();
        (config.height_strategy, config.fixed_height, config.edge_testing)
    };
    let recommendation = crate::generate::recommend(&report.breakpoints, strategy, fixed, edge);
    Ok(generate_config::Output {
        viewports: recommendation
            .recommended
            .iter()
            .map(|c| c.viewport.clone())
            .collect(),
        recommendation,
    })
}

pub mod generate_config {
    use super::*;

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Output {
        pub viewports: Vec<Viewport>,
        pub recommendation: crate::generate::Recommendation,
    }
}

/// The canvas URL, but only when it is this project's.
///
/// It counts as this project's if the project was applied with it (which is
/// what puts it in `url_to_project`) or if the scan found it as one of this
/// project's own dev servers. Anything else belongs to whatever was open
/// before, and gets left out rather than committed into someone's repo.
pub fn url_for_this_project(state: &Shared) -> Option<String> {
    let url = state.canvas.lock().unwrap().url.clone();
    if url.is_empty() || url.starts_with("about:") {
        return None;
    }
    let Some(project) = state.project.lock().unwrap().clone() else {
        return None;
    };
    let root = project.root.to_string_lossy().to_string();

    let applied_here = state
        .config
        .lock()
        .unwrap()
        .url_to_project
        .get(&url)
        .map(|owner| owner == &root)
        .unwrap_or(false);
    if applied_here {
        return Some(url);
    }

    let found_here = project
        .report
        .as_ref()
        .map(|report| {
            report
                .dev_servers
                .iter()
                .any(|server| crate::util::normalize_url(&server.url).as_str() == url)
        })
        .unwrap_or(false);
    found_here.then_some(url)
}

pub fn write_project_file(
    app: &AppHandle,
    state: &Shared,
    viewports: Option<Vec<Viewport>>,
) -> Result<String, String> {
    let viewports = viewports.unwrap_or_else(|| {
        state
            .canvas
            .lock()
            .unwrap()
            .panels
            .iter()
            .map(|p| p.viewport.clone())
            .collect()
    });
    // `breakpoints.md` commits with the repo, so the URL in it has to belong
    // to this repo. The canvas holds whatever was last loaded, which may be a
    // different project's site, and writing that hostname into someone else's
    // file is not a mistake worth making twice.
    let url = url_for_this_project(state);
    let path = project::write_file(app, state, &viewports, url)?;
    let _ = app.emit("project:file-written", json!({ "path": path }));
    Ok(path)
}

pub fn list_profiles(state: &Shared) -> Vec<Value> {
    let config = state.config.lock().unwrap();
    config
        .profiles
        .iter()
        .map(|(id, profile)| {
            json!({
                "id": id,
                "name": profile.name,
                "locked": profile.locked,
                "active": *id == config.active_profile,
                "viewports": profile.viewports,
            })
        })
        .collect()
}

/// Re-run detection and return the fresh report.
pub async fn scan_breakpoints(app: &AppHandle, state: &Shared) -> Result<scanner::types::ScanReport, String> {
    let root = state
        .project
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or("no project is open")?;
    let report = project::scan_now(app, &root).await?;
    if let Some(context) = state.project.lock().unwrap().as_mut() {
        context.report = Some(report.clone());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::types::{DevServerDiscovery, ScanReport};
    use crate::state::{AppState, ProjectContext};
    use std::path::PathBuf;
    use std::sync::Arc;

    // ---- what the webview hands back -------------------------------------

    #[test]
    fn a_value_arrives_encoded_twice_and_comes_back_once() {
        // Tauri serialises the result and the injected wrapper already made
        // JSON, so the payload is a JSON string containing JSON. Unpacking one
        // layer too few turns every number into a quoted string.
        let raw = serde_json::to_string(r#"{"ok":true,"value":42}"#).unwrap();
        assert_eq!(unwrap_envelope(&raw).unwrap(), serde_json::json!(42));
    }

    #[test]
    fn an_object_value_survives_the_round_trip() {
        let raw = serde_json::to_string(r#"{"ok":true,"value":{"title":"Home","width":1024}}"#).unwrap();
        let value = unwrap_envelope(&raw).unwrap();
        assert_eq!(value["title"], "Home");
        assert_eq!(value["width"], 1024);
    }

    #[test]
    fn a_string_the_page_returned_is_not_parsed_again() {
        let raw = serde_json::to_string(r#"{"ok":true,"value":"Bucknell Be The Ray"}"#).unwrap();
        assert_eq!(unwrap_envelope(&raw).unwrap(), serde_json::json!("Bucknell Be The Ray"));
    }

    #[test]
    fn a_script_that_threw_comes_back_as_an_error_with_its_message() {
        let raw = serde_json::to_string(r#"{"ok":false,"error":"x is not defined"}"#).unwrap();
        assert_eq!(unwrap_envelope(&raw).unwrap_err(), "x is not defined");
    }

    #[test]
    fn a_null_value_is_a_value_and_not_a_failure() {
        // get_dom leans on this: null means "nothing matched the selector".
        let raw = serde_json::to_string(r#"{"ok":true,"value":null}"#).unwrap();
        assert_eq!(unwrap_envelope(&raw).unwrap(), Value::Null);
    }

    #[test]
    fn something_that_is_not_json_at_all_is_handed_back_rather_than_lost() {
        assert_eq!(
            unwrap_envelope("undefined").unwrap(),
            serde_json::json!("undefined")
        );
    }

    // ---- whose URL is this? ----------------------------------------------

    fn state_with(url: &str, project: Option<ProjectContext>) -> Shared {
        let state: Shared = Arc::new(AppState::default());
        state.canvas.lock().unwrap().url = url.to_string();
        *state.project.lock().unwrap() = project;
        state
    }

    fn project_at(root: &str) -> ProjectContext {
        ProjectContext {
            root: PathBuf::from(root),
            name: "test".into(),
            ..Default::default()
        }
    }

    fn project_serving(root: &str, dev_url: &str) -> ProjectContext {
        let mut context = project_at(root);
        context.report = Some(ScanReport {
            dev_servers: vec![DevServerDiscovery {
                url: dev_url.into(),
                port: None,
                source: "test".into(),
                confidence: 0.9,
                responding: None,
            }],
            ..Default::default()
        });
        context
    }

    #[test]
    fn another_projects_url_is_never_written_into_this_ones_file() {
        // This is the bug. breakpoints.md commits with the repo, and opening a
        // project with no dev server of its own used to keep the previous
        // project's site on the canvas, then write that hostname into the new
        // project's file.
        let state = state_with("https://someone-elses-client.lndo.site/", Some(project_at("/p/mine")));
        assert_eq!(url_for_this_project(&state), None);
    }

    #[test]
    fn a_url_this_project_was_applied_with_is_kept() {
        let state = state_with("https://mine.lndo.site/", Some(project_at("/p/mine")));
        state
            .config
            .lock()
            .unwrap()
            .url_to_project
            .insert("https://mine.lndo.site/".into(), "/p/mine".into());
        assert_eq!(
            url_for_this_project(&state).as_deref(),
            Some("https://mine.lndo.site/")
        );
    }

    #[test]
    fn a_url_this_projects_own_scan_found_is_kept() {
        let state = state_with(
            "https://mine.lndo.site/",
            Some(project_serving("/p/mine", "https://mine.lndo.site")),
        );
        assert_eq!(
            url_for_this_project(&state).as_deref(),
            Some("https://mine.lndo.site/")
        );
    }

    #[test]
    fn a_url_applied_by_a_different_project_is_refused() {
        let state = state_with("https://theirs.lndo.site/", Some(project_at("/p/mine")));
        state
            .config
            .lock()
            .unwrap()
            .url_to_project
            .insert("https://theirs.lndo.site/".into(), "/p/theirs".into());
        assert_eq!(url_for_this_project(&state), None);
    }

    #[test]
    fn a_blank_canvas_contributes_no_url() {
        let state = state_with("about:blank", Some(project_at("/p/mine")));
        assert_eq!(url_for_this_project(&state), None);
        let state = state_with("", Some(project_at("/p/mine")));
        assert_eq!(url_for_this_project(&state), None);
    }

    #[test]
    fn with_no_project_open_there_is_nowhere_for_a_url_to_belong() {
        let state = state_with("https://mine.lndo.site/", None);
        assert_eq!(url_for_this_project(&state), None);
    }
}
