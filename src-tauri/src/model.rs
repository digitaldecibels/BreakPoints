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

/// The smallest the row zoom goes: ten times smaller than actual size.
///
/// Chosen so that the far end of the slider means something you can say out
/// loud: ten viewports in the space one of them took. Smaller than this and a
/// panel is too small to read anything in, which makes the rest of the slider
/// travel wasted.
pub const ROW_SCALE_FLOOR: f64 = 0.1;

/// Turn a slider position into the scale the row is drawn at.
///
/// Geometric rather than linear, because zoom is perceived in ratios: halfway
/// along should look halfway between 1x and a tenth, and linearly it does not.
/// At 0 this is exactly 1.0, so a slider that has never been touched changes
/// nothing at all.
pub fn row_scale(row_zoom: f64) -> f64 {
    let t = row_zoom.clamp(0.0, 1.0);
    if t == 0.0 {
        return 1.0;
    }
    ROW_SCALE_FLOOR.powf(t)
}

/// Where a panel sits vertically, and whether it keeps its declared height.
///
/// `Top` is the honest default: a viewport's declared height is a guess at a
/// device, and drawing it at that height from a fixed top edge is the least
/// the app can claim. `Centre` is the same heights, balanced in the space,
/// which reads better when the panels are much shorter than the window.
/// `Stretch` throws the declared height away and takes the window's instead,
/// for when you want to see as much of a page as you can. Width is never
/// touched by any of them, because the width is the breakpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerticalAlign {
    #[default]
    Top,
    Center,
    Stretch,
}

impl VerticalAlign {
    /// True when the declared height is replaced by the window's.
    pub fn stretches(self) -> bool {
        self == VerticalAlign::Stretch
    }
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
    /// Where a panel sits in the space between the labels and the status bar,
    /// and whether it keeps its own height at all.
    ///
    /// One setting rather than two flags, because the three answers are
    /// alternatives: a panel is at the top, or it is centred, or it is stretched
    /// to the window. Two booleans could say "centred and stretched", which
    /// means nothing.
    #[serde(default)]
    pub vertical_align: VerticalAlign,
    /// How far the whole row is zoomed out, from 0 to 1.
    ///
    /// 0 is actual size. 1 is `ROW_SCALE_FLOOR`, which is ten times smaller, so
    /// ten viewports occupy the space one did. Stored as the slider's own
    /// position rather than as the resulting scale, because the position is
    /// what has to come back on the slider and deriving it from a scale means
    /// a logarithm every time the window is drawn.
    ///
    /// Separate from Fit, and multiplied with it. Fit answers "make this fit
    /// the height"; this answers "show me more of the row", and doing both is
    /// a reasonable thing to want.
    #[serde(default)]
    pub row_zoom: f64,
    /// The old boolean this replaced. Read so that a config written before the
    /// change keeps the setting it had, never written back.
    #[serde(default, rename = "fullHeight", skip_serializing)]
    pub legacy_full_height: Option<bool>,
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
    /// Which terminal the "Start a session" button opens, by id. `None` means
    /// none has been chosen and the first installed one wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// The command that button runs in it.
    ///
    /// Editable because the agent is a matter of what you use rather than
    /// something the app should decide: Claude Code is the default, and
    /// anything that speaks to the bridge works the same way. `None` means the
    /// default in `terminal.rs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_command: Option<String>,
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
            vertical_align: VerticalAlign::Top,
            legacy_full_height: None,
            row_zoom: 0.0,
            scroll_sync: true,
            follow_links: true,
            agent_bridge: false,
            bridge_token: None,
            window: None,
            browser: None,
            terminal: None,
            agent_command: None,
            report_prompt: None,
            recheck_on_change: false,
        }
    }
}

impl AppConfig {
    /// Carry forward settings whose shape has changed.
    ///
    /// `fullHeight` was a boolean and is now one of three alignments. Without
    /// this, somebody who worked with stretched panels opens the app after an
    /// update and finds them back at their declared heights with no
    /// explanation, which reads as the app having forgotten something.
    pub fn migrate(&mut self) {
        if self.legacy_full_height == Some(true) && self.vertical_align == VerticalAlign::Top {
            self.vertical_align = VerticalAlign::Stretch;
        }
        // Read once. Saving never writes it back, so the next load has nothing
        // to carry.
        self.legacy_full_height = None;
    }

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
mod vertical_align_tests {
    use super::*;

    /// Somebody who worked with stretched panels keeps them after the update.
    #[test]
    fn the_old_full_height_boolean_becomes_stretch() {
        let mut config = AppConfig::default();
        config.legacy_full_height = Some(true);
        config.migrate();
        assert_eq!(config.vertical_align, VerticalAlign::Stretch);
        assert_eq!(config.legacy_full_height, None, "read once, never written back");
    }

    /// An explicit choice made after the update is not overwritten by the old
    /// boolean sitting in the same file.
    #[test]
    fn a_new_choice_beats_the_old_boolean() {
        let mut config = AppConfig::default();
        config.legacy_full_height = Some(true);
        config.vertical_align = VerticalAlign::Center;
        config.migrate();
        assert_eq!(config.vertical_align, VerticalAlign::Center);
    }

    #[test]
    fn only_stretch_replaces_the_declared_height() {
        assert!(VerticalAlign::Stretch.stretches());
        assert!(!VerticalAlign::Top.stretches());
        assert!(!VerticalAlign::Center.stretches());
    }

    /// The default is the one that claims least.
    #[test]
    fn top_is_the_default() {
        assert_eq!(VerticalAlign::default(), VerticalAlign::Top);
        assert_eq!(AppConfig::default().vertical_align, VerticalAlign::Top);
    }
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
