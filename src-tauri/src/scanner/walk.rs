//! One bounded walk of the project, shared by every detector.
//!
//! These limits are load-bearing, not suggestions. A Drupal site with contrib
//! modules is enormous, and a scan that walks all of it hangs the app on the
//! first project someone tries.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;

use super::log::ScanLog;

/// Files kept for the detectors to query.
pub const MAX_FILES: usize = 3000;
/// Directory entries the walk will look at before giving up. Looking is cheap;
/// keeping and reading are not, which is why these are two different numbers.
/// A flat cap on everything gets spent on Drupal core before it ever reaches
/// the theme.
pub const MAX_ENTRIES: usize = 120_000;
pub const MAX_DEPTH: usize = 12;
pub const MAX_CSS_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Directories that never hold a project's own breakpoints and always hold
/// thousands of files.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "dist",
    "build",
    "coverage",
    "target",
    "bower_components",
    "__pycache__",
    "tmp",
    "storybook-static",
    // Drupal keeps other people's code here, and it is never the site's own
    // responsive decisions.
    "contrib",
    // Static site generators put their build here under a reserved name, and
    // it is not always inside a directory called dist or build. A Drupal site
    // with an Astro docs build in `web/private/docs/` was offering four
    // "breakpoints" that all came out of Astro's own theme.
    "_astro",
    "_next",
    "_site",
    "pagefind",
];

/// A hidden directory is tooling. The one exception is DDEV, whose config is
/// the whole point of looking.
pub fn skip_hidden(name: &str) -> bool {
    name.starts_with('.') && name != ".ddev"
}

/// A Drupal docroot's `core` is Drupal itself: thousands of stylesheets and a
/// pile of `*.breakpoints.yml` that belong to core, not to this site.
pub fn is_drupal_core(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) != Some("core") {
        return false;
    }
    match path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        Some("web") | Some("docroot") | Some("html") | Some("public") => true,
        // A `core` directly under the project root is Drupal only if it looks
        // like Drupal, and `src/core` in a JS project must survive.
        _ => path.join("lib").join("Drupal.php").is_file(),
    }
}

/// The only files any detector asks for. Everything else is walked past.
fn worth_keeping(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.ends_with(".breakpoints.yml") {
        return true;
    }
    if name.starts_with("tailwind.config.") || name.starts_with("vite.config.") {
        return true;
    }
    if matches!(
        name,
        "package.json"
            | "composer.json"
            | ".lando.yml"
            | ".lando.yaml"
            | "config.yaml"
            | "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("css") | Some("scss") | Some("sass")
    )
}

/// The result of the walk: relative paths only, so nothing that leaves the
/// project root can be read later by mistake.
pub struct FileIndex {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    pub scanned: usize,
    pub skipped: usize,
    pub truncated: bool,
    pub started: Instant,
}

impl FileIndex {
    pub fn build(root: &Path, log: &mut ScanLog) -> Self {
        let started = Instant::now();
        let mut files = Vec::new();
        let mut skipped = 0usize;
        let mut seen = 0usize;
        let mut truncated = false;

        let walker = WalkBuilder::new(root)
            // .lando.yml sits at the root and .ddev is a hidden directory, so
            // hidden entries are walked and then filtered by name.
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .parents(false)
            .max_depth(Some(MAX_DEPTH))
            .filter_entry(|entry| {
                if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    return true;
                }
                let name = entry.file_name().to_string_lossy();
                if SKIP_DIRS.contains(&name.as_ref()) || skip_hidden(&name) {
                    return false;
                }
                !is_drupal_core(entry.path())
            })
            .build();

        for entry in walker {
            seen += 1;
            if seen > MAX_ENTRIES {
                log.skip("<walk>", "hit the entry cap of 120000");
                truncated = true;
                break;
            }
            if seen % 512 == 0 && started.elapsed() > TIMEOUT {
                log.skip("<walk>", "hit the 10 second scan timeout");
                truncated = true;
                break;
            }
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if !worth_keeping(path) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if meta.len() > MAX_FILE_BYTES {
                    log.skip(&rel_string(root, path), "larger than 2MB");
                    skipped += 1;
                    continue;
                }
            }
            if files.len() >= MAX_FILES {
                log.skip("<walk>", "hit the 3000 file cap");
                truncated = true;
                break;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                files.push(rel.to_path_buf());
            }
        }

        log.note(format!(
            "walk looked at {seen} entries, kept {} files, skipped {}, {} ms{}",
            files.len(),
            skipped,
            started.elapsed().as_millis(),
            if truncated { ", truncated" } else { "" }
        ));

        Self {
            root: root.to_path_buf(),
            scanned: files.len(),
            files,
            skipped,
            truncated,
            started,
        }
    }

    pub fn out_of_time(&self) -> bool {
        self.started.elapsed() > TIMEOUT
    }

    /// Every indexed file whose name matches, cheapest test first.
    pub fn by_name(&self, name: &str) -> Vec<&PathBuf> {
        self.files
            .iter()
            .filter(|p| p.file_name().map(|n| n == name).unwrap_or(false))
            .collect()
    }

    pub fn by_extension(&self, exts: &[&str]) -> Vec<&PathBuf> {
        self.files
            .iter()
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| exts.contains(&e))
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Files whose name ends with a suffix, for `*.breakpoints.yml`.
    pub fn by_suffix(&self, suffix: &str) -> Vec<&PathBuf> {
        self.files
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(suffix))
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Files whose stem matches, for `vite.config.js` / `.ts` / `.mjs`.
    pub fn by_stem(&self, stem: &str) -> Vec<&PathBuf> {
        self.files
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&format!("{stem}.")))
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn absolute(&self, rel: &Path) -> PathBuf {
        self.root.join(rel)
    }
}

/// Reading is budgeted separately from walking, because a hundred small
/// stylesheets are cheap and one 8MB compiled bundle is not.
pub struct ReadBudget {
    remaining: usize,
}

impl ReadBudget {
    pub fn new() -> Self {
        Self {
            remaining: MAX_CSS_BYTES,
        }
    }

    pub fn read(&mut self, index: &FileIndex, rel: &Path, log: &mut ScanLog) -> Option<String> {
        let display = rel.to_string_lossy().to_string();
        if self.remaining == 0 {
            log.skip(&display, "read budget of 10MB is spent");
            return None;
        }
        let text = match std::fs::read_to_string(index.absolute(rel)) {
            Ok(text) => text,
            Err(err) => {
                log.skip(&display, &format!("could not read: {err}"));
                return None;
            }
        };
        self.remaining = self.remaining.saturating_sub(text.len());
        log.read(&display, text.len());
        Some(text)
    }
}

impl Default for ReadBudget {
    fn default() -> Self {
        Self::new()
    }
}

pub fn rel_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_files_a_detector_asks_for_are_kept() {
        for keep in [
            "src/styles.css",
            "themes/x/_layout.scss",
            "tailwind.config.ts",
            "web/themes/x/x.breakpoints.yml",
            "package.json",
            ".ddev/config.yaml",
        ] {
            assert!(worth_keeping(Path::new(keep)), "{keep}");
        }
        for drop in ["src/main.ts", "README.md", "web/index.php", "logo.png"] {
            assert!(!worth_keeping(Path::new(drop)), "{drop}");
        }
    }

    #[test]
    fn hidden_directories_are_skipped_except_ddev() {
        assert!(skip_hidden(".claude"));
        assert!(skip_hidden(".git"));
        assert!(skip_hidden(".twprobe"));
        assert!(!skip_hidden(".ddev"));
        assert!(!skip_hidden("themes"));
    }

    #[test]
    fn a_drupal_docroot_core_is_recognised_but_a_js_src_core_is_not() {
        assert!(is_drupal_core(Path::new("/p/web/core")));
        assert!(is_drupal_core(Path::new("/p/docroot/core")));
        assert!(!is_drupal_core(Path::new("/p/src/core")));
        assert!(!is_drupal_core(Path::new("/p/web/themes")));
    }
}
