//! Watching a project for the two kinds of change that matter, which behave
//! differently on purpose.
//!
//! `breakpoints.md` changed means you or your agent edited the source of truth
//! deliberately, so the panels reload immediately. A framework config changed
//! means the underlying breakpoints might have moved, so the app re-scans,
//! compares against the stored hash, and offers a review. It never silently
//! regenerates.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use tauri::{AppHandle, Emitter};

use crate::project_file;

/// Dropping this stops the watch.
pub struct WatchHandle {
    _debouncer: Debouncer<notify::RecommendedWatcher, FileIdMap>,
    pub root: PathBuf,
}

/// Files whose change means the project's own config moved.
const CONFIG_STEMS: &[&str] = &[
    "tailwind.config",
    "vite.config",
    "package.json",
    ".lando.yml",
    "config.yaml",
    "docker-compose.yml",
    "compose.yml",
];

pub fn start(app: AppHandle, root: &Path) -> Result<WatchHandle, String> {
    let root_owned = root.to_path_buf();
    let emit_root = root_owned.clone();

    // Half a second is enough that an editor's save-and-format is one event,
    // and short enough that a deliberate edit feels immediate.
    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        None,
        move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            let mut project_file_changed = false;
            let mut config_changed: Option<String> = None;

            for event in events {
                for path in &event.paths {
                    match classify(path) {
                        Some(Change::ProjectFile) => project_file_changed = true,
                        Some(Change::Config) => {
                            config_changed = Some(relative(&emit_root, path));
                        }
                        None => {}
                    }
                }
            }

            if project_file_changed {
                let _ = app.emit(
                    "project:file-changed",
                    serde_json::json!({ "root": emit_root.to_string_lossy() }),
                );
            }
            if let Some(file) = config_changed {
                let _ = app.emit(
                    "project:config-changed",
                    serde_json::json!({ "root": emit_root.to_string_lossy(), "file": file }),
                );
            }
        },
    )
    .map_err(|e| e.to_string())?;

    debouncer
        .watcher()
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    debouncer.cache().add_root(root, RecursiveMode::Recursive);

    Ok(WatchHandle {
        _debouncer: debouncer,
        root: root_owned,
    })
}

enum Change {
    ProjectFile,
    Config,
}

/// Most of what a watch reports is noise: build output, caches, a package
/// manager writing a lockfile. Only two categories are worth waking up for.
fn classify(path: &Path) -> Option<Change> {
    let name = path.file_name()?.to_str()?;
    let text = path.to_string_lossy();

    for skip in [
        "/node_modules/", "/.git/", "/dist/", "/build/", "/target/", "/vendor/",
        "/.next/", "/coverage/", "/.cache/",
    ] {
        if text.contains(skip) {
            return None;
        }
    }

    if name == project_file::MARKDOWN_NAME || name == project_file::JSON_NAME {
        return Some(Change::ProjectFile);
    }
    if name.ends_with(".breakpoints.yml") {
        return Some(Change::Config);
    }
    if CONFIG_STEMS
        .iter()
        .any(|stem| name == *stem || name.starts_with(&format!("{stem}.")))
    {
        return Some(Change::Config);
    }
    // A stylesheet can hold Tailwind v4 breakpoints or plain media queries, so
    // it counts, but only source, never compiled output.
    if name.ends_with(".css") || name.ends_with(".scss") || name.ends_with(".sass") {
        if name.ends_with(".min.css") {
            return None;
        }
        return Some(Change::Config);
    }
    None
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(path: &str) -> Option<&'static str> {
        match classify(Path::new(path)) {
            Some(Change::ProjectFile) => Some("project"),
            Some(Change::Config) => Some("config"),
            None => None,
        }
    }

    #[test]
    fn the_project_file_is_its_own_category() {
        assert_eq!(kind("/p/breakpoints.md"), Some("project"));
        assert_eq!(kind("/p/.breakpoints.json"), Some("project"));
    }

    #[test]
    fn framework_config_and_stylesheets_ask_for_a_rescan() {
        assert_eq!(kind("/p/tailwind.config.ts"), Some("config"));
        assert_eq!(kind("/p/themes/x/x.breakpoints.yml"), Some("config"));
        assert_eq!(kind("/p/src/styles.scss"), Some("config"));
        assert_eq!(kind("/p/.lando.yml"), Some("config"));
    }

    #[test]
    fn build_output_and_dependencies_never_wake_the_app_up() {
        assert_eq!(kind("/p/node_modules/x/tailwind.config.js"), None);
        assert_eq!(kind("/p/dist/assets/main.css"), None);
        assert_eq!(kind("/p/src/vendor.min.css"), None);
        assert_eq!(kind("/p/src/main.ts"), None);
    }
}
