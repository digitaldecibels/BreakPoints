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
use notify_debouncer_full::{
    new_debouncer_opt, DebounceEventResult, Debouncer, NoCache,
};
use tauri::{AppHandle, Emitter};

use crate::project_file;

/// Dropping this stops the watch.
pub struct WatchHandle {
    _debouncer: Debouncer<notify::RecommendedWatcher, NoCache>,
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
    // `NoCache`, deliberately.
    //
    // The cached variant walks the entire project when the watch starts and
    // stats every entry it finds, keeping the result for the life of the
    // watch. On a Drupal project that is node_modules plus vendor plus
    // web/core: hundreds of thousands of stats, seconds of disk work on a
    // worker thread, and tens of megabytes held for as long as the project is
    // open. It walks again every time a directory appears, so a `npm install`
    // or a build does it repeatedly. All of that exists to stitch rename
    // events together, and `classify` does not care about renames.
    let mut debouncer = new_debouncer_opt::<_, notify::RecommendedWatcher, NoCache>(
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
        NoCache,
        notify::Config::default(),
    )
    .map_err(|e| e.to_string())?;

    debouncer
        .watcher()
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;

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

    // The same rules the scanner walks by, rather than a second list that
    // drifts from it. Without `contrib` and Drupal's `core`, a `composer
    // install` offered a rescan of thousands of files that CLAUDE.md is
    // explicit are not the site; without the hidden-directory rule, a checkout
    // in `.claude/worktrees` carries its own `breakpoints.md` and reloaded the
    // panels from a copy of the repo.
    // Directories only. The file's own name is not a directory name, and
    // `.breakpoints.json` and `.lando.yml` are both files we very much want.
    let parent = path.parent();
    if let Some(parent) = parent {
        for component in parent.components() {
            let Some(name) = component.as_os_str().to_str() else { continue };
            if crate::scanner::walk::SKIP_DIRS.contains(&name)
                || crate::scanner::walk::skip_hidden(name)
            {
                return None;
            }
        }
        for ancestor in parent.ancestors() {
            if crate::scanner::walk::is_drupal_core(ancestor) {
                return None;
            }
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

    /// The watcher used to keep its own shorter list, so these three woke the
    /// app up and offered a rescan of files that are not the site at all.
    #[test]
    fn it_skips_everything_the_scanner_skips() {
        assert_eq!(kind("/p/web/core/themes/x/x.breakpoints.yml"), None, "Drupal core");
        assert_eq!(kind("/p/web/modules/contrib/x/x.breakpoints.yml"), None, "contrib");
        assert_eq!(
            kind("/p/.claude/worktrees/copy/breakpoints.md"),
            None,
            "a second checkout of the same repo"
        );
        // And a hidden directory that is deliberately not skipped.
        assert_eq!(kind("/p/.ddev/config.yaml"), Some("config"));
    }
}
