//! The chrome's side of the wall. Thin wrappers: every one of these delegates
//! to `tools`, `canvas` or `project` so the agent bridge and the toolbar run
//! the same code.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

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

    Snapshot {
        config,
        canvas,
        project,
        bridge,
    }
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
#[tauri::command]
pub fn take_reports(state: State<'_, Shared>) -> Vec<crate::state::Report> {
    state.take_reports()
}

#[tauri::command]
pub fn inspect_panel(state: State<'_, Shared>, panel: String) -> Result<(), String> {
    let id = canvas::resolve_id(&state, &panel).unwrap_or(panel);
    canvas::inspect_panel(&state, &id)
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

/// The window handed a folder by drag and drop.
#[tauri::command]
pub fn dropped_path(app: AppHandle, path: String) {
    let _ = app.emit("project:dropped", path);
}
