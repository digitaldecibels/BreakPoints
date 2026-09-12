//! The shapes that cross every boundary in the app: Rust to chrome webview,
//! Rust to disk, Rust to agent. One definition each, serialised as camelCase
//! because the other two sides are JavaScript and Markdown.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One panel's declared size. Width is the measurement that matters; height is
/// derived (see `generate`) and always editable afterwards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub id: String,
    pub name: String,
    pub width: f64,
    pub height: f64,
    /// Framework key this came from: "md", "narrow", "css", "custom".
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// A Figma node URL or a repo-relative image path under `.breakpoints/refs/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

fn default_source() -> String {
    "custom".into()
}
fn yes() -> bool {
    true
}

impl Viewport {
    pub fn new(name: impl Into<String>, width: f64, height: f64, source: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            id: slug_id(&name, width),
            name,
            width,
            height,
            source: source.into(),
            enabled: true,
            reference: None,
        }
    }
}

/// Ids have to survive a round trip through a webview label and an agent
/// request, so they are lowercase ASCII with the width appended to keep two
/// same-named viewports apart.
pub fn slug_id(name: &str, width: f64) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-').replace("--", "-");
    format!("{}-{}", if base.is_empty() { "vp" } else { &base }, width as i64)
}

/// The generic set from section 5. Not the default experience: this is what you
/// get when Break/Points has nothing better to offer.
pub fn fallback_viewports() -> Vec<Viewport> {
    vec![
        Viewport::new("Mobile Small", 375.0, 667.0, "device"),
        Viewport::new("Mobile Regular", 430.0, 932.0, "device"),
        Viewport::new("Tablet", 768.0, 1024.0, "device"),
        Viewport::new("Desktop", 1440.0, 900.0, "device"),
        Viewport::new("Wide", 1920.0, 1080.0, "device"),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub locked: bool,
    pub viewports: Vec<Viewport>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan: Option<String>,
    /// Hash of the extracted breakpoint widths, not of the file bytes. A save
    /// that changes no width is a no-op (section 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    /// Where this project's screenshots are written. `None` means the user's
    /// Downloads folder.
    ///
    /// Per project rather than app-wide, because a screenshot is evidence about
    /// one site and filing all of them together makes them useless the moment
    /// you are working on two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shot_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeightStrategy {
    /// Aspect ratio of a real device in that size class.
    DeviceRatio,
    /// One height for every panel.
    Fixed,
    /// Whatever you typed.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FitMode {
    /// Each panel scaled so it fits vertically.
    Height,
    /// One factor across the row, so relative proportions stay true.
    Uniform,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub active_profile: String,
    pub profiles: BTreeMap<String, Profile>,
    pub projects: BTreeMap<String, ProjectRecord>,
    pub url_to_project: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_project: Option<String>,
    pub height_strategy: HeightStrategy,
    pub fixed_height: f64,
    pub fit_mode: FitMode,
    pub edge_testing: bool,
    pub zoom_to_fit: bool,
    /// Every panel as tall as the canvas allows, rather than at its declared
    /// height. Remembered, because it is a way of working rather than a thing
    /// you do once.
    #[serde(default)]
    pub full_height: bool,
    pub scroll_sync: bool,
    /// Whether a link followed in one panel is followed in all of them.
    #[serde(default = "yes")]
    pub follow_links: bool,
    pub agent_bridge: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_token: Option<String>,
    /// Where the window was last left, so it comes back there instead of in
    /// the middle of whatever you are working on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowGeometry>,
    /// Which browser "Open in browser" uses, by id. `None` means none has been
    /// chosen and the first installed one wins.
    ///
    /// The panels themselves are not affected and cannot be: they are
    /// WKWebViews because that is the only engine Tauri has on macOS. This
    /// preference is only about where a page goes when you want somebody
    /// else's devtools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    /// The standing instruction sent with every reported problem.
    ///
    /// A note says what is wrong. It does not say what to do about it, and an
    /// agent handed only a description will guess. This is the sentence in
    /// front of it, editable because what you want done with a note is a matter
    /// of how you work rather than something the app should decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_prompt: Option<String>,
    /// Run the layout checks again whenever the project's stylesheets change.
    ///
    /// Off by default. A rebuild every few seconds should not start an audit
    /// every few seconds, and somebody in the middle of editing does not want
    /// a notice every time they save.
    #[serde(default)]
    pub recheck_on_change: bool,
}

/// What travels with a note when nobody has written their own instruction.
pub const DEFAULT_REPORT_PROMPT: &str = "Fix this layout problem in the project this panel is pointed at.\n\nVerify the premise before changing anything: measure the element at the width given and confirm it is actually wrong. Notes are written quickly while looking at a screen, and some of them describe something that is already correct. If it is already correct, say so rather than changing it.\n\nThe width is the point. A fix that works at one width and breaks another is not a fix.";


/// Position and size of the main window, in logical pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WindowGeometry {
    /// Is this somewhere a window could actually be seen?
    ///
    /// A saved position outlives the display it was saved on. Unplug an
    /// external monitor and the window comes back at x = 2400 on a laptop that
    /// stops at 1512, which looks exactly like the app failing to launch.
    pub fn fits_within(&self, screen_width: f64, screen_height: f64) -> bool {
        const EDGE: f64 = 80.0;
        self.width >= 400.0
            && self.height >= 300.0
            && self.x + self.width > EDGE
            && self.y + self.height > EDGE
            && self.x < screen_width - EDGE
            && self.y < screen_height - EDGE
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "default".to_string(),
            Profile {
                name: "Default".into(),
                locked: true,
                viewports: fallback_viewports(),
            },
        );
        Self {
            active_profile: "default".into(),
            profiles,
            projects: BTreeMap::new(),
            url_to_project: BTreeMap::new(),
            last_url: None,
            last_project: None,
            height_strategy: HeightStrategy::DeviceRatio,
            fixed_height: 900.0,
            fit_mode: FitMode::Height,
            edge_testing: false,
            zoom_to_fit: true,
            full_height: false,
            scroll_sync: true,
            follow_links: true,
            agent_bridge: false,
            bridge_token: None,
            window: None,
            browser: None,
            report_prompt: None,
            recheck_on_change: false,
        }
    }
}

impl AppConfig {
    pub fn active_viewports(&self) -> Vec<Viewport> {
        self.profiles
            .get(&self.active_profile)
            .map(|p| p.viewports.clone())
            .unwrap_or_else(fallback_viewports)
    }
}

/// What a panel is doing, so the chrome can draw the failed state at the
/// panel's exact size rather than collapsing it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "state", content = "code")]
pub enum PanelState {
    Loading,
    Loaded,
    Failed(u16),
}

/// Whether the dev URL answered. Drives the warn dot in the toolbar.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum UrlStatus {
    #[default]
    Unknown,
    Ok,
    NotResponding,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64, width: f64, height: f64) -> WindowGeometry {
        WindowGeometry { x, y, width, height }
    }

    #[test]
    fn a_window_on_the_screen_is_restored() {
        assert!(at(100.0, 80.0, 1400.0, 900.0).fits_within(2560.0, 1440.0));
        assert!(at(0.0, 0.0, 1400.0, 900.0).fits_within(1512.0, 982.0));
    }

    #[test]
    fn a_window_left_on_a_monitor_that_is_gone_is_not_restored() {
        // Unplug the external display and a saved x of 2400 puts the window
        // somewhere nobody can reach, which reads as the app failing to launch.
        assert!(!at(2400.0, 300.0, 1400.0, 900.0).fits_within(1512.0, 982.0));
        assert!(!at(200.0, 1800.0, 1400.0, 900.0).fits_within(1512.0, 982.0));
    }

    #[test]
    fn a_window_pushed_almost_entirely_off_the_left_or_top_is_not_restored() {
        assert!(!at(-1390.0, 100.0, 1400.0, 900.0).fits_within(1512.0, 982.0));
        assert!(!at(100.0, -890.0, 1400.0, 900.0).fits_within(1512.0, 982.0));
    }

    #[test]
    fn a_window_hanging_off_an_edge_but_still_grabbable_is_restored() {
        // Partly off screen is a position someone chose, not a broken one.
        assert!(at(1300.0, 100.0, 1400.0, 900.0).fits_within(1512.0, 982.0));
    }

    #[test]
    fn a_saved_size_too_small_to_use_is_ignored() {
        assert!(!at(100.0, 100.0, 200.0, 900.0).fits_within(2560.0, 1440.0));
        assert!(!at(100.0, 100.0, 1400.0, 100.0).fits_within(2560.0, 1440.0));
    }
}
