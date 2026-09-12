//! App configuration: preferences, profiles, and the index of known projects.
//!
//! Viewport data for a project lives in that project's `breakpoints.md`, not
//! here. This file holds what is true of the app, not of your code.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::state::Shared;

use crate::model::AppConfig;

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("breakpoints.json"))
}

/// A config we cannot parse is kept, not replaced. Losing someone's profiles
/// because a field moved is worse than starting from defaults for one session.
pub fn load(app: &AppHandle) -> AppConfig {
    let Ok(path) = config_path(app) else {
        return AppConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    match serde_json::from_str::<AppConfig>(&text) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("[breakpoints] config at {} did not parse: {err}", path.display());
            let backup = path.with_extension("json.unreadable");
            let _ = std::fs::rename(&path, &backup);
            AppConfig::default()
        }
    }
}

pub fn save(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    // Write beside the target and rename, so a crash mid-write cannot leave a
    // half-written config behind.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Where scan logs go: `<app data>/scans/`.
pub fn scan_log_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("no data directory: {e}"))?
        .join("scans");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Where cropped panel screenshots go: `<app data>/shots/`.
pub fn shot_dir(app: &AppHandle, state: &Shared) -> Result<PathBuf, String> {
    // The open project's own folder wins, then Downloads. Screenshots used to
    // go to the app's data directory, which is somewhere nobody can find
    // without being told the path, and a screenshot you cannot find is not
    // evidence of anything.
    let chosen = {
        let config = state.config.lock().unwrap();
        state
            .project
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.root.to_string_lossy().to_string())
            .and_then(|key| config.projects.get(&key).and_then(|r| r.shot_dir.clone()))
    };

    let dir = match chosen {
        Some(path) => PathBuf::from(path),
        None => downloads_dir(app)?,
    };

    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not write to {}: {e}", dir.display()))?;
    Ok(dir)
}

/// The user's Downloads folder, with the app's own data directory as a last
/// resort. `app_data_dir` is always available; Downloads is not, if the platform
/// has no such concept or the lookup fails.
pub fn downloads_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(dir) = app.path().download_dir() {
        return Ok(dir);
    }
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("shots"))
        .map_err(|e| format!("no data directory: {e}"))
}
