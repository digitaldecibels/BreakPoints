//! Break/Points: read a codebase, build the responsive test environment it
//! actually uses, and hand that environment to an agent.
//!
//! The window holds two kinds of webview. One is the chrome, which fills the
//! window and draws the toolbar, the label strip and the sheets. The others are
//! panels, one per viewport, positioned by Rust at absolute pixel coordinates
//! and composited on top of the chrome.

pub mod access;
pub mod audit;
pub mod baseline;
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
pub mod verify;
pub mod watcher;

use std::sync::Arc;

use serde::Serialize;
use tauri::{
    webview::WebviewBuilder, DragDropEvent, Emitter, Listener, LogicalPosition, LogicalSize,
    Manager, WebviewUrl, WindowEvent,
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

/// One screen, in logical pixels.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Would a remembered position still put the window somewhere visible?
///
/// A saved position outlives the display it was saved on, and a window
/// restored onto a monitor that is no longer plugged in looks exactly like the
/// app failing to launch.
///
/// An empty list means nobody has told us about any screens, which is not the
/// same as the window not fitting on one. Treating those two the same is what
/// made the app forget its position on every launch: `available_monitors()`
/// answers with nothing this early in startup, so every remembered geometry
/// was thrown away and the window opened at the default size instead.
pub fn fits_on_a_screen(geometry: &model::WindowGeometry, screens: &[Screen]) -> bool {
    if screens.is_empty() {
        return true;
    }
    screens.iter().any(|screen| {
        geometry.x >= screen.left - 1.0
            && geometry.y >= screen.top - 1.0
            && geometry.fits_within(screen.right, screen.bottom)
    })
}

fn screens_of(monitors: &[tauri::window::Monitor]) -> Vec<Screen> {
    monitors
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let position = monitor.position();
            let size = monitor.size();
            let left = position.x as f64 / scale;
            let top = position.y as f64 / scale;
            Screen {
                left,
                top,
                right: left + size.width as f64 / scale,
                bottom: top + size.height as f64 / scale,
            }
        })
        .collect()
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

            // Applied first and checked afterwards. Screens cannot be
            // enumerated reliably before a window exists, and refusing a
            // geometry because nothing has told us about any screens yet is
            // how the app came to forget its position on every launch.
            if let Some(geometry) = remembered {
                builder = builder
                    .position(geometry.x, geometry.y)
                    .inner_size(geometry.width, geometry.height);
            }
            let window = builder.build()?;

            // Now the window exists, its screens can be asked about. A
            // remembered geometry that lands on a display which is no longer
            // there is put back to something visible.
            if let Some(geometry) = remembered {
                let screens = window
                    .available_monitors()
                    .map(|monitors| screens_of(&monitors))
                    .unwrap_or_default();
                if !fits_on_a_screen(&geometry, &screens) {
                    eprintln!(
                        "[breakpoints] {:.0}x{:.0} at {:.0},{:.0} is not on any screen any more, so the window is at its default size",
                        geometry.width, geometry.height, geometry.x, geometry.y
                    );
                    let _ = window.set_size(LogicalSize::new(1400.0, 900.0));
                    let _ = window.center();
                }
            }

            let size = window.inner_size()?;
            let scale = window.scale_factor().unwrap_or(1.0);

            // Before any resize event, so the pump knows what is on screen
            // from the first tick.
            *shared.window_width.lock().unwrap() = size.width as f64 / scale;

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
                    // macOS sends this continuously while the window is being
                    // dragged, and this handler runs on the main thread. Doing
                    // the work here meant, every frame, serialising the whole
                    // config to a temp file and renaming it, plus 21 inline
                    // webview calls and seven page re-layouts. So the size is
                    // recorded now and both of those are paced.
                    canvas::remember_window_size(&resize_handle, &resize_state);

                    // At most one layout per frame, and always one after the
                    // last event.
                    let already_scheduled = {
                        let mut pending = resize_state.relayout_pending.lock().unwrap();
                        std::mem::replace(&mut *pending, true)
                    };
                    if !already_scheduled {
                        let handle = resize_handle.clone();
                        let state = resize_state.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
                            *state.relayout_pending.lock().unwrap() = false;
                            canvas::relayout(&handle, &state);
                        });
                    }

                    // The config is written once, when the drag stops.
                    let seq = {
                        let mut seq = resize_state.resize_seq.lock().unwrap();
                        *seq = seq.wrapping_add(1);
                        *seq
                    };
                    let handle = resize_handle.clone();
                    let state = resize_state.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        if *state.resize_seq.lock().unwrap() == seq {
                            remember_window(&handle, &state);
                        }
                    });
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

            // Re-run the layout checks after the project's stylesheets
            // settle, when that has been asked for. The event the watcher
            // already emits is the trigger; the wait is for the dev server to
            // rebuild and the panels to catch up.
            {
                let handle = handle.clone();
                let state = shared.clone();
                handle.clone().listen_any("project:config-changed", move |_| {
                    let wanted = state.config.lock().unwrap().recheck_on_change;
                    eprintln!(
                        "[breakpoints] the project's stylesheets changed; re-running the checks: {wanted}"
                    );
                    if !wanted {
                        return;
                    }
                    let handle = handle.clone();
                    let state = state.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        match audit::run(&handle, &state).await {
                            Ok(report) => {
                                // The audit counts findings by severity, so
                                // say the numbers rather than that it finished.
                                let count = |key: &str| {
                                    report
                                        .get("summary")
                                        .and_then(|s| s.get(key))
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                };
                                let (high, medium) = (count("high"), count("medium"));
                                let line = if high == 0 && medium == 0 {
                                    "Stylesheets changed. The layout checks found nothing.".to_string()
                                } else {
                                    format!(
                                        "Stylesheets changed. The layout checks found {high} serious and {medium} worth a look."
                                    )
                                };
                                let _ = handle.emit("checks:done", line);
                            }
                            Err(err) => {
                                let _ = handle.emit("checks:done", format!("checks did not run: {err}"));
                            }
                        }
                    });
                });
            }

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
            commands::audit_accessibility,
            commands::dropped_path,
        ])
        .run(tauri::generate_context!())
        .expect("error running Break/Points");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(left: f64, top: f64, width: f64, height: f64) -> Screen {
        Screen { left, top, right: left + width, bottom: top + height }
    }

    fn geometry(x: f64, y: f64, width: f64, height: f64) -> model::WindowGeometry {
        model::WindowGeometry { x, y, width, height }
    }

    /// The one that was wrong. Nothing has told us about any screens yet, which
    /// is not the same as the window not fitting on one, and treating them the
    /// same made the app forget its position on every single launch.
    #[test]
    fn no_screens_known_is_not_a_reason_to_forget_where_the_window_was() {
        assert!(fits_on_a_screen(&geometry(61.0, 32.0, 5059.0, 1331.0), &[]));
    }

    #[test]
    fn a_window_that_fits_the_screen_is_restored() {
        let screens = [screen(0.0, 0.0, 5120.0, 1440.0)];
        assert!(fits_on_a_screen(&geometry(61.0, 32.0, 5059.0, 1331.0), &screens));
        assert!(fits_on_a_screen(&geometry(0.0, 0.0, 1400.0, 900.0), &screens));
    }

    /// The case the check exists for: a display that is no longer plugged in.
    #[test]
    fn a_window_on_a_screen_that_is_gone_is_not_restored() {
        let laptop = [screen(0.0, 0.0, 1512.0, 982.0)];
        assert!(!fits_on_a_screen(&geometry(3000.0, 100.0, 1400.0, 900.0), &laptop));
        assert!(!fits_on_a_screen(&geometry(-2000.0, 0.0, 1400.0, 900.0), &laptop));
    }

    #[test]
    fn a_second_screen_counts() {
        let both = [screen(0.0, 0.0, 1512.0, 982.0), screen(1512.0, 0.0, 2560.0, 1440.0)];
        assert!(fits_on_a_screen(&geometry(2000.0, 100.0, 1400.0, 900.0), &both));
    }
}
