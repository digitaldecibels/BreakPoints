//! The chrome's side of the wall. Thin wrappers: every one of these delegates
//! to `tools`, `canvas` or `project` so the agent bridge and the toolbar run
//! the same code.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::browser;
use crate::canvas::{self, CanvasInfo};
use crate::model::{AppConfig, FitMode, HeightStrategy, Profile, Viewport};
use crate::state::Shared;
use crate::tools::{self, ProjectInfo};
use crate::{bridge, config, project, project_file, scanner, util};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub config: AppConfig,
    pub canvas: CanvasInfo,
    pub project: ProjectInfo,
    pub bridge: bridge::Status,
    /// Where this project's screenshots are written, resolved. Carried here
    /// rather than in `ProjectInfo` because it has a value even with no project
    /// open, when it is the Downloads folder.
    pub shot_dir: String,
    /// The standing instruction sent with every note, resolved to the default
    /// when none has been written, so the settings textarea always has real
    /// text in it rather than a placeholder.
    pub report_prompt: String,
    /// Which agent session notes are addressed to. Carried here as well as
    /// emitted on `reports:owner`, because the chrome reloads and an event it
    /// missed is an event it never hears about.
    pub report_owner: Option<crate::state::ClientSession>,
    /// How many notes are waiting. Same reason: the chrome reloads, and a
    /// badge it counted itself would start again at nothing while the queue
    /// still held work.
    pub report_count: usize,
}

#[tauri::command]
pub fn app_state(app: AppHandle, state: State<'_, Shared>) -> Snapshot {
    // Each lock is taken and released on its own line. A struct literal keeps
    // every temporary alive until the whole expression finishes, so locking
    // twice inside one would deadlock against itself.
    let config = state.config.lock().unwrap().clone();
    let canvas = canvas::info(&state);
    let project = tools::get_project_info(&state);
    let bridge = bridge::status(&app, &state);
    let report_owner = state.report_owner();
    let report_count = state.report_count();
    let report_prompt = config
        .report_prompt
        .clone()
        .unwrap_or_else(|| crate::model::DEFAULT_REPORT_PROMPT.to_string());
    let shot_dir = config::shot_dir(&app, &state)
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();

    Snapshot {
        config,
        canvas,
        project,
        bridge,
        report_owner,
        report_count,
        shot_dir,
        report_prompt,
    }
}

/// Pick a folder for this project's screenshots.
///
/// Per project on purpose. A screenshot is evidence about one site, and filing
/// every project's into one folder makes them useless the moment you have two
/// open in a week.
#[tauri::command]
pub async fn choose_shot_dir(app: AppHandle, state: State<'_, Shared>) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Where should this project's screenshots go?")
        .pick_folder(move |picked| {
            let _ = tx.send(picked.map(|p| p.to_string()));
        });

    let Some(chosen) = rx.await.ok().flatten() else {
        // Cancelled. Report what it still is rather than an error.
        return config::shot_dir(&app, &state).map(|d| d.to_string_lossy().to_string());
    };

    set_project_shot_dir(&app, &state, Some(chosen))
}

/// Put this project's screenshots back in the Downloads folder.
#[tauri::command]
pub fn reset_shot_dir(app: AppHandle, state: State<'_, Shared>) -> Result<String, String> {
    set_project_shot_dir(&app, &state, None)
}

/// Write the choice against the open project and report where shots now go.
fn set_project_shot_dir(
    app: &AppHandle,
    state: &Shared,
    dir: Option<String>,
) -> Result<String, String> {
    let key = state
        .project
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.root.to_string_lossy().to_string())
        .ok_or("open a project first: the folder is remembered per project")?;

    {
        let mut config = state.config.lock().unwrap();
        config.projects.entry(key).or_default().shot_dir = dir;
        let _ = config::save(app, &config);
    }

    config::shot_dir(app, state).map(|d| d.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn apply_viewports(
    app: AppHandle,
    state: State<'_, Shared>,
    viewports: Vec<Viewport>,
    url: String,
) -> Result<f64, String> {
    let shared = (*state).clone();
    project::apply(&app, &shared, viewports, url).await
}

#[tauri::command]
pub fn navigate(app: AppHandle, state: State<'_, Shared>, url: String) -> Result<(), String> {
    canvas::navigate_all(&state, &url);
    {
        let mut config = state.config.lock().unwrap();
        config.last_url = Some(url.clone());
        let _ = config::save(&app, &config);
    }
    canvas::emit_canvas(&app, &state);

    // A URL that has been associated with a folder reconnects that project.
    let shared = (*state).clone();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let status = project::probe(&url).await;
        *shared.url_status.lock().unwrap() = status.clone();
        let _ = handle.emit("url:status", &status);
    });
    Ok(())
}

#[tauri::command]
pub fn reload_all(state: State<'_, Shared>) {
    canvas::reload_all(&state);
}

#[tauri::command]
pub fn reload_panel(state: State<'_, Shared>, panel: String) -> Result<(), String> {
    let id = canvas::resolve_id(&state, &panel).ok_or("no such panel")?;
    state.clear_console(&id);
    canvas::reload_panel(&state, &id);
    Ok(())
}

#[tauri::command]
pub fn set_scroll(state: State<'_, Shared>, offset: f64) {
    canvas::set_scroll(&state, offset);
}

#[tauri::command]
pub fn set_zoom_to_fit(app: AppHandle, state: State<'_, Shared>, on: bool) {
    state.canvas.lock().unwrap().zoom_to_fit = on;
    {
        let mut config = state.config.lock().unwrap();
        config.zoom_to_fit = on;
        let _ = config::save(&app, &config);
    }
    canvas::relayout(&app, &state);
}

/// Every panel as tall as the canvas allows. Overrides zoom to fit, which is
/// the opposite instruction.
#[tauri::command]
pub fn set_full_height(app: AppHandle, state: State<'_, Shared>, on: bool) {
    state.canvas.lock().unwrap().full_height = on;
    {
        let mut config = state.config.lock().unwrap();
        config.full_height = on;
        let _ = config::save(&app, &config);
    }
    canvas::relayout(&app, &state);
}

#[tauri::command]
pub fn set_scroll_sync(app: AppHandle, state: State<'_, Shared>, on: bool) {
    state.canvas.lock().unwrap().sync_on = on;
    let mut config = state.config.lock().unwrap();
    config.scroll_sync = on;
    let _ = config::save(&app, &config);
}

/// Sheets are chrome, and chrome composites under the panels, so opening one
/// has to move the panels out of the way.
/// The Web Inspector on one panel. Its own window, and it takes focus: WebKit
/// gives no way to open it in the background.
/// Arm the element picker in every panel. Not saved to the config: it is a
/// thing you are doing right now, not a preference.
#[tauri::command]
pub fn set_picking(state: State<'_, Shared>, on: bool) {
    canvas::set_picking(&state, on);
}

/// Everything reported since the last time anyone asked, and clears the list.
///
/// This is the chrome's copy button, so it takes every note whoever it was
/// addressed to. That is what makes a note written while a session that has
/// since gone away owned reports still reachable by hand.
#[tauri::command]
pub fn take_reports(app: AppHandle, state: State<'_, Shared>) -> Vec<crate::state::Report> {
    let taken = state.take_reports();
    let shared = (*state).clone();
    tools::emit_report_count(&app, &shared);
    taken
}

#[tauri::command]
pub fn inspect_panel(app: AppHandle, state: State<'_, Shared>, panel: String) -> Result<(), String> {
    let id = canvas::resolve_id(&state, &panel).unwrap_or(panel);

    // Pressing Inspect on the panel already being inspected leaves the mode.
    // The inspector itself cannot be closed from here, but the row coming back
    // is the part that was in the way.
    if state.canvas.lock().unwrap().inspecting.as_deref() == Some(id.as_str()) {
        canvas::set_inspecting(&app, &state, None);
        return Ok(());
    }

    canvas::inspect_panel(&app, &state, &id)
}

/// Leave the inspect mode and put the row back.
#[tauri::command]
pub fn stop_inspecting(app: AppHandle, state: State<'_, Shared>) {
    canvas::set_inspecting(&app, &state, None);
}

/// Open one panel's page in a real browser, at that panel's width.
///
/// The counterpart to Inspect, and the honest answer to "can I use Chrome
/// DevTools on a panel". You cannot: a panel is a WKWebView and Chrome DevTools
/// speaks a protocol it does not. So the page goes to Chrome instead, at the
/// same width, and you use Chrome's tools there.
#[tauri::command]
pub fn open_panel_in_browser(
    app: AppHandle,
    state: State<'_, Shared>,
    panel: String,
) -> Result<String, String> {
    let chosen = state.config.lock().unwrap().browser.clone();
    let browser = chosen
        .as_deref()
        .and_then(browser::Browser::from_id)
        .filter(|b| b.installed())
        .or_else(|| browser::installed().first().copied())
        .ok_or("no browser found in /Applications")?;

    // `resolve_id` takes the canvas lock itself, so it has to finish before the
    // block below takes it again. Calling it inside would deadlock against
    // itself, which is the same trap the `Snapshot` comment above describes.
    let id = canvas::resolve_id(&state, &panel).ok_or("no such panel")?;

    // Read the panel out under its own lock and drop it before launching. A
    // process spawn while holding the canvas lock would block every scroll and
    // callback for as long as the launch took.
    let (url, width, height) = {
        let canvas = state.canvas.lock().unwrap();
        let target = canvas
            .panels
            .iter()
            .find(|p| p.viewport.id == id)
            .ok_or("no such panel")?;
        // The declared width, never the on-screen one. A zoomed panel is drawn
        // smaller but the page inside it is at the breakpoint, and the
        // breakpoint is what the browser has to be opened at.
        (
            canvas.url.clone(),
            target.viewport.width,
            target.viewport.height,
        )
    };

    let profile_root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("browser-profiles");

    browser::open(browser, &url, width, height, &profile_root)?;
    Ok(browser.app_name().to_string())
}

/// Which browsers are installed, for the settings list.
#[tauri::command]
pub fn list_browsers(state: State<'_, Shared>) -> Value {
    let chosen = state.config.lock().unwrap().browser.clone();
    let installed = browser::installed();
    let active = chosen
        .as_deref()
        .and_then(browser::Browser::from_id)
        .filter(|b| b.installed())
        .or_else(|| installed.first().copied());

    serde_json::json!({
        "browsers": installed
            .iter()
            .map(|b| serde_json::json!({
                "id": b.id(),
                "name": b.app_name(),
                "canSize": b.can_size(),
            }))
            .collect::<Vec<_>>(),
        "active": active.map(|b| b.id()),
    })
}

/// The Web Inspector on Break/Points' own chrome, for when the app itself is
/// what is misbehaving. Without this the only console in the app is one nobody
/// can read, which is how a permissions failure once looked like a dead UI.
#[tauri::command]
pub fn inspect_chrome(app: AppHandle) -> Result<(), String> {
    app.get_webview("chrome")
        .ok_or("no chrome webview")?
        .open_devtools();
    Ok(())
}

#[tauri::command]
pub fn set_follow_links(app: AppHandle, state: State<'_, Shared>, on: bool) {
    canvas::set_follow(&state, on);
    let mut config = state.config.lock().unwrap();
    config.follow_links = on;
    let _ = config::save(&app, &config);
}

#[tauri::command]
pub fn set_sheet_open(app: AppHandle, state: State<'_, Shared>, open: bool) {
    canvas::set_panels_hidden(&app, &state, open);
}

#[tauri::command]
pub fn relayout(app: AppHandle, state: State<'_, Shared>) {
    canvas::relayout(&app, &state);
}

#[tauri::command]
pub async fn choose_project(app: AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a project folder")
        .pick_folder(move |picked| {
            let _ = tx.send(picked.map(|p| p.to_string()));
        });
    rx.await.ok().flatten()
}

#[tauri::command]
pub async fn open_project(
    app: AppHandle,
    state: State<'_, Shared>,
    path: String,
) -> Result<project::ProjectOpened, String> {
    let shared = (*state).clone();
    project::open(&app, &shared, PathBuf::from(path)).await
}

#[tauri::command]
pub async fn rescan(
    app: AppHandle,
    state: State<'_, Shared>,
) -> Result<scanner::types::ScanReport, String> {
    let shared = (*state).clone();
    tools::scan_breakpoints(&app, &shared).await
}

#[tauri::command]
pub fn write_project_file(
    app: AppHandle,
    state: State<'_, Shared>,
    viewports: Option<Vec<Viewport>>,
) -> Result<String, String> {
    tools::write_project_file(&app, &state, viewports)
}

/// Open `breakpoints.md` in whatever the user edits Markdown with.
#[tauri::command]
pub fn open_project_file(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let root = state
        .project
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or("no project is open")?;
    let path = root.join(project_file::MARKDOWN_NAME);
    if !path.is_file() {
        return Err(format!("{} has not been written yet", project_file::MARKDOWN_NAME));
    }
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub height_strategy: Option<HeightStrategy>,
    pub fixed_height: Option<f64>,
    pub fit_mode: Option<FitMode>,
    pub edge_testing: Option<bool>,
    /// Browser id for "Open in browser". Only this preference does not touch
    /// the layout, so it is the one that does not need a relayout after.
    pub browser: Option<String>,
    /// The standing instruction sent with every reported problem. An empty
    /// string means "back to the default", which is how the Reset button in
    /// the settings sheet works without needing a command of its own.
    pub report_prompt: Option<String>,
}

#[tauri::command]
pub fn set_preferences(app: AppHandle, state: State<'_, Shared>, prefs: Preferences) -> AppConfig {
    let updated = {
        let mut config = state.config.lock().unwrap();
        if let Some(value) = prefs.height_strategy {
            config.height_strategy = value;
        }
        if let Some(value) = prefs.fixed_height {
            config.fixed_height = value;
        }
        if let Some(value) = prefs.fit_mode {
            config.fit_mode = value;
            state.canvas.lock().unwrap().fit_mode = value;
        }
        if let Some(value) = prefs.edge_testing {
            config.edge_testing = value;
        }
        if let Some(value) = prefs.browser {
            config.browser = Some(value);
        }
        if let Some(value) = prefs.report_prompt {
            config.report_prompt = if value.trim().is_empty() {
                None
            } else {
                Some(value)
            };
        }
        let _ = config::save(&app, &config);
        config.clone()
    };
    canvas::relayout(&app, &state);
    updated
}

#[tauri::command]
pub fn list_profiles(state: State<'_, Shared>) -> Vec<Value> {
    tools::list_profiles(&state)
}

#[tauri::command]
pub fn select_profile(app: AppHandle, state: State<'_, Shared>, id: String) -> Result<Vec<Viewport>, String> {
    let mut config = state.config.lock().unwrap();
    let profile = config.profiles.get(&id).ok_or("no such profile")?.clone();
    config.active_profile = id;
    let _ = config::save(&app, &config);
    Ok(profile.viewports)
}

#[tauri::command]
pub fn save_profile(
    app: AppHandle,
    state: State<'_, Shared>,
    id: Option<String>,
    name: String,
    viewports: Vec<Viewport>,
) -> Result<String, String> {
    let mut config = state.config.lock().unwrap();
    let id = match id {
        Some(id) => {
            let profile = config.profiles.get_mut(&id).ok_or("no such profile")?;
            if profile.locked {
                // Editing the read-only Default becomes a new profile rather
                // than an error, which is what the person meant.
                let fresh = format!("p_{}", util::random_token(3));
                config.profiles.insert(
                    fresh.clone(),
                    Profile { name, locked: false, viewports },
                );
                config.active_profile = fresh.clone();
                let _ = config::save(&app, &config);
                return Ok(fresh);
            }
            profile.name = name;
            profile.viewports = viewports;
            id
        }
        None => {
            let fresh = format!("p_{}", util::random_token(3));
            config.profiles.insert(
                fresh.clone(),
                Profile { name, locked: false, viewports },
            );
            config.active_profile = fresh.clone();
            fresh
        }
    };
    let _ = config::save(&app, &config);
    Ok(id)
}

#[tauri::command]
pub fn delete_profile(app: AppHandle, state: State<'_, Shared>, id: String) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    if config.profiles.get(&id).map(|p| p.locked).unwrap_or(false) {
        return Err("the Default profile cannot be deleted".into());
    }
    config.profiles.remove(&id);
    if config.active_profile == id {
        config.active_profile = "default".into();
    }
    let _ = config::save(&app, &config);
    Ok(())
}

#[tauri::command]
pub fn scan_log(state: State<'_, Shared>) -> Result<Vec<Value>, String> {
    tools::get_scan_log(&state)
}

#[tauri::command]
pub fn breakpoint_sources(state: State<'_, Shared>) -> Vec<Value> {
    tools::get_breakpoint_sources(&state)
}

#[tauri::command]
pub async fn set_bridge(
    app: AppHandle,
    state: State<'_, Shared>,
    on: bool,
) -> Result<bridge::Status, String> {
    let shared = (*state).clone();
    bridge::set_enabled(&app, &shared, on).await
}

#[tauri::command]
pub async fn eval_js(state: State<'_, Shared>, panel: String, script: String) -> Result<Value, String> {
    let shared = (*state).clone();
    tools::eval_js(&shared, &panel, &script).await
}

#[tauri::command]
pub fn panel_console(state: State<'_, Shared>, panel: String) -> Result<Vec<crate::state::ConsoleLine>, String> {
    tools::get_console(&state, &panel)
}

#[tauri::command]
pub async fn screenshot_panel(
    app: AppHandle,
    state: State<'_, Shared>,
    panel: String,
    full_page: Option<bool>,
) -> Result<String, String> {
    let shared = (*state).clone();
    crate::shots::capture(&app, &shared, &panel, full_page.unwrap_or(false)).await
}

#[tauri::command]
pub async fn audit_all(app: AppHandle, state: State<'_, Shared>) -> Result<Value, String> {
    let shared = (*state).clone();
    crate::audit::run(&app, &shared).await
}

/// Run the accessibility checks in every panel.
///
/// Panels have to be visible while this runs, so the window shows a notice
/// rather than a sheet and only opens the results when it is done. A sheet
/// hides the panels, and a hidden webview is not a laid out one.
#[tauri::command]
pub async fn audit_accessibility(
    state: State<'_, Shared>,
) -> Result<crate::access::AccessReport, String> {
    let shared = (*state).clone();
    crate::access::audit_all(&shared).await
}

/// The window handed a folder by drag and drop.
#[tauri::command]
pub fn dropped_path(app: AppHandle, path: String) {
    let _ = app.emit("project:dropped", path);
}
