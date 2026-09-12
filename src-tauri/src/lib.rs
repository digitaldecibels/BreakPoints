//! Break/Points: read a codebase, build the responsive test environment it
//! actually uses, and hand that environment to an agent.
//!
//! The window holds two kinds of webview. One is the chrome, which fills the
//! window and draws the toolbar, the label strip and the sheets. The others are
//! panels, one per viewport, positioned by Rust at absolute pixel coordinates
//! and composited on top of the chrome.

pub mod audit;
pub mod bridge;
pub mod browser;
pub mod callback;
pub mod canvas;
pub mod commands;
pub mod config;
pub mod generate;
pub mod model;
pub mod project;
pub mod project_file;
pub mod references;
pub mod scanner;
pub mod shots;
pub mod state;
pub mod tools;
pub mod util;
pub mod watcher;

use std::sync::Arc;

use serde::Serialize;
use tauri::{
    webview::WebviewBuilder, DragDropEvent, Emitter, LogicalPosition, LogicalSize, Manager,
    WebviewUrl, WindowEvent,
};

use state::{AppState, Shared};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Boot {
    pub snapshot: commands::Snapshot,
    /// The project reopened from last time, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored: Option<project::ProjectOpened>,
    pub fallback_viewports: Vec<model::Viewport>,
}

/// Called by the chrome once it is listening, so nothing is emitted into a void.
#[tauri::command]
async fn boot(app: tauri::AppHandle, state: tauri::State<'_, Shared>) -> Result<Boot, String> {
    let shared = (*state).clone();
    let restored = project::restore(&app, &shared).await;
    Ok(Boot {
        snapshot: commands::app_state(app.clone(), state),
        restored,
        fallback_viewports: model::fallback_viewports(),
    })
}

/// Write the window's current position and size to the config.
///
/// Called on move and on resize, so quitting is not the only way it gets
/// saved: an app killed by a rebuild would otherwise never remember anything.
fn remember_window(app: &tauri::AppHandle, state: &state::Shared) {
    let Some(window) = app.get_window("main") else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    let geometry = model::WindowGeometry {
        x: position.x as f64 / scale,
        y: position.y as f64 / scale,
        width: size.width as f64 / scale,
        height: size.height as f64 / scale,
    };

    let mut config = state.config.lock().unwrap();
    if config.window == Some(geometry) {
        return;
    }
    config.window = Some(geometry);
    let _ = config::save(app, &config);
}

/// Would a remembered position still put the window somewhere visible?
///
/// A saved position outlives the display it was saved on, and a window
/// restored onto a monitor that is no longer plugged in looks exactly like the
/// app failing to launch.
fn fits_on_a_screen(app: &tauri::App, geometry: &model::WindowGeometry) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return false;
    };
    monitors.iter().any(|monitor| {
        let scale = monitor.scale_factor();
        let position = monitor.position();
        let size = monitor.size();
        let left = position.x as f64 / scale;
        let top = position.y as f64 / scale;
        let right = left + size.width as f64 / scale;
        let bottom = top + size.height as f64 / scale;
        geometry.x >= left - 1.0
            && geometry.y >= top - 1.0
            && geometry.fits_within(right, bottom)
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(AppState::default()) as Shared)
        .setup(|app| {
            let handle = app.handle().clone();
            let shared = app.state::<Shared>().inner().clone();

            let loaded = config::load(&handle);
            {
                let mut canvas = shared.canvas.lock().unwrap();
                canvas.zoom_to_fit = loaded.zoom_to_fit;
                canvas.sync_on = loaded.scroll_sync;
                canvas.follow.on = loaded.follow_links;
                canvas.fit_mode = loaded.fit_mode;
            }
            *shared.config.lock().unwrap() = loaded;

            // Notes nobody collected last time. The app restarts on every
            // Rust edit under `tauri dev` and the chrome restarts on a Vite
            // reload, and either one used to take the whole queue with it.
            match config::reports_path(&handle) {
                Ok(path) => shared.restore_reports(path),
                Err(err) => eprintln!("[breakpoints] reports cannot be persisted: {err}"),
            }

            // Injected scripts need somewhere to call home, and the port has to
            // be known before the first panel is built.
            match callback::start(handle.clone(), shared.clone()) {
                Ok(endpoint) => {
                    let _ = shared.endpoint.set(endpoint);
                }
                Err(err) => eprintln!("[breakpoints] panel callbacks are unavailable: {err}"),
            }

            // An https panel cannot reach that server, so its messages are
            // collected instead. One pump for the life of the app, because it
            // looks at whatever panels happen to exist on each tick.
            // Assume focus until macOS says otherwise: the pump only goes
            // quiet on a real Focused(false), never on a default.
            *shared.window_focused.lock().unwrap() = true;
            canvas::start_pump(handle.clone(), shared.clone());

            // Come back where you were left. Without this the window lands in
            // the middle of the screen on every launch, which is a nuisance in
            // ordinary use and worse while an agent is restarting the app.
            let remembered = shared.config.lock().unwrap().window;
            let mut builder = tauri::window::WindowBuilder::new(app, "main")
                .title("Break/Points")
                .inner_size(1400.0, 900.0)
                .min_inner_size(900.0, 600.0)
                // Launching does not take focus. This is a measuring
                // instrument you drive from somewhere else, and a window that
                // jumps in front of what you were typing is a nuisance.
                .focused(false);

            if let Some(geometry) = remembered.filter(|g| fits_on_a_screen(app, g)) {
                builder = builder
                    .position(geometry.x, geometry.y)
                    .inner_size(geometry.width, geometry.height);
            }
            let window = builder.build()?;

            let size = window.inner_size()?;
            let scale = window.scale_factor().unwrap_or(1.0);

            // The chrome fills the window. Panels spawn after it, so they
            // composite above it, which is what lets a sheet be drawn by
            // hiding the panels rather than by fighting the z-order.
            window.add_child(
                WebviewBuilder::new("chrome", WebviewUrl::App("index.html".into()))
                    .auto_resize()
                    .zoom_hotkeys_enabled(false),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(size.width as f64 / scale, size.height as f64 / scale),
            )?;

            // Resizing changes every panel's scale and position.
            let resize_handle = handle.clone();
            let resize_state = shared.clone();
            window.on_window_event(move |event| match event {
                WindowEvent::Resized(_) => {
                    canvas::relayout(&resize_handle, &resize_state);
                    remember_window(&resize_handle, &resize_state);
                }
                WindowEvent::Moved(_) => {
                    remember_window(&resize_handle, &resize_state);
                }
                // Losing focus is the cheapest signal there is that nobody is
                // about to scroll a panel by hand.
                WindowEvent::Focused(has_focus) => {
                    *resize_state.window_focused.lock().unwrap() = *has_focus;
                    // Coming back is the cheapest signal that the Web Inspector
                    // is done with a panel. It resizes whatever webview it
                    // attaches to and never puts it back, so this is the moment
                    // to undo that. Cheap, idempotent, and harmless on the many
                    // focus events that have nothing to do with the inspector.
                    if *has_focus {
                        canvas::restore_frames(&resize_state);
                    }
                }
                WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                    if let Some(path) = paths.iter().find(|p| p.is_dir()) {
                        let _ = resize_handle
                            .emit("project:dropped", path.to_string_lossy().to_string());
                    } else {
                        let _ = resize_handle.emit(
                            "project:dropped-not-a-folder",
                            paths
                                .iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect::<Vec<_>>(),
                        );
                    }
                }
                WindowEvent::DragDrop(DragDropEvent::Enter { .. }) => {
                    let _ = resize_handle.emit("project:drag", true);
                }
                WindowEvent::DragDrop(DragDropEvent::Leave) => {
                    let _ = resize_handle.emit("project:drag", false);
                }
                _ => {}
            });

            // The bridge is off by default. If it was on last time, it comes
            // back on, because that was a deliberate choice.
            if shared.config.lock().unwrap().agent_bridge {
                let bridge_handle = handle.clone();
                let bridge_state = shared.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = bridge::set_enabled(&bridge_handle, &bridge_state, true).await
                    {
                        eprintln!("[breakpoints] agent bridge did not start: {err}");
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            boot,
            commands::app_state,
            commands::apply_viewports,
            commands::navigate,
            commands::reload_all,
            commands::reload_panel,
            commands::set_scroll,
            commands::set_zoom_to_fit,
            commands::set_full_height,
            commands::set_scroll_sync,
            commands::set_follow_links,
            commands::set_picking,
            commands::take_reports,
            commands::inspect_panel,
            commands::stop_inspecting,
            commands::inspect_chrome,
            commands::open_panel_in_browser,
            commands::list_browsers,
            commands::choose_shot_dir,
            commands::reset_shot_dir,
            commands::set_sheet_open,
            commands::relayout,
            commands::choose_project,
            commands::open_project,
            commands::rescan,
            commands::write_project_file,
            commands::open_project_file,
            commands::set_preferences,
            commands::list_profiles,
            commands::select_profile,
            commands::save_profile,
            commands::delete_profile,
            commands::scan_log,
            commands::breakpoint_sources,
            commands::set_bridge,
            commands::eval_js,
            commands::panel_console,
            commands::screenshot_panel,
            commands::audit_all,
            commands::dropped_path,
        ])
        .run(tauri::generate_context!())
        .expect("error running Break/Points");
}
