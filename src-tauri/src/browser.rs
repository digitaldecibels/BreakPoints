//! Opening a panel's URL in a real browser, at that panel's width.
//!
//! WHY THIS EXISTS
//!
//! Every panel is a WKWebView, because Tauri renders through WRY and WRY uses
//! the platform's native engine, which on macOS is WebKit with no alternative.
//! So the Inspect button will always be WebKit's Web Inspector. Chrome DevTools
//! cannot attach to a panel at all: DevTools speaks the Chrome DevTools
//! Protocol and a WKWebView speaks WebKit's own remote inspector protocol.
//! Different protocol, not a setting anybody can flip.
//!
//! What we can do is hand the page to the browser you actually want, at the
//! width you were looking at, and let you use that browser's own tools. The row
//! stays the measuring instrument; this is the escape hatch to Chrome, Firefox
//! or Safari when you need something only they have.
//!
//! FOCUS
//!
//! This one is allowed to steal focus, unlike everything else the app launches.
//! You pressed a button that says "open this in Chrome"; a browser that opened
//! silently behind the window would be the bug.

use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// A browser we know how to size a window in.
///
/// Stored as a string in the config rather than by path, so the config stays
/// readable and a browser that gets moved or reinstalled still resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Browser {
    Chrome,
    Brave,
    Edge,
    Firefox,
    Safari,
}

impl Browser {
    pub const ALL: [Browser; 5] = [
        Browser::Chrome,
        Browser::Brave,
        Browser::Edge,
        Browser::Firefox,
        Browser::Safari,
    ];

    /// The name `open -a` wants, which is also the folder in /Applications.
    pub fn app_name(self) -> &'static str {
        match self {
            Browser::Chrome => "Google Chrome",
            Browser::Brave => "Brave Browser",
            Browser::Edge => "Microsoft Edge",
            Browser::Firefox => "Firefox",
            Browser::Safari => "Safari",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Brave => "brave",
            Browser::Edge => "edge",
            Browser::Firefox => "firefox",
            Browser::Safari => "safari",
        }
    }

    pub fn from_id(id: &str) -> Option<Browser> {
        Browser::ALL.into_iter().find(|b| b.id() == id)
    }

    /// Is it actually on this machine? The settings list shows only what is,
    /// because offering a browser that is not installed produces a failure at
    /// the moment of use rather than at the moment of choosing.
    pub fn installed(self) -> bool {
        PathBuf::from(format!("/Applications/{}.app", self.app_name())).exists()
    }

    /// Chromium-family browsers take the same flags, because they are the same
    /// browser with different paint.
    fn is_chromium(self) -> bool {
        matches!(self, Browser::Chrome | Browser::Brave | Browser::Edge)
    }

    /// Can this browser be told what size to open at?
    ///
    /// Safari cannot. It has no command line at all beyond a URL, so it opens
    /// wherever it last was and the width has to be set by hand, which makes it
    /// close to useless for this app's purpose. Worth saying out loud in the UI
    /// rather than letting someone measure a Safari window and trust it.
    pub fn can_size(self) -> bool {
        !matches!(self, Browser::Safari)
    }
}

/// Every browser installed, in a fixed order so the settings list does not
/// reshuffle itself between launches.
pub fn installed() -> Vec<Browser> {
    Browser::ALL.into_iter().filter(|b| b.installed()).collect()
}

/// Build the argument list for one browser at one size.
///
/// Pure, so the flags can be tested without launching anything. Every test in
/// this file exercises this function rather than `Command`.
pub fn args_for(browser: Browser, url: &str, width: f64, height: f64, profile: &str) -> Vec<String> {
    let w = width.round() as i64;
    let h = height.round() as i64;

    if browser.is_chromium() {
        return vec![
            // `--app` drops the tab strip and the toolbar, so the window is
            // almost all viewport and the width we ask for is the width the
            // page gets. A normal Chrome window would render the page narrower
            // than the number on the label, which is the single thing this app
            // exists to prevent.
            format!("--app={url}"),
            format!("--window-size={w},{h}"),
            // A separate profile directory is what makes `--window-size` work
            // at all. Passing it to an already-running Chrome opens a window in
            // the existing process and the sizing flags are ignored, so the
            // window comes back at whatever size Chrome last used. The cost is
            // that this profile is not signed in to anything; the benefit is a
            // clean profile with no extensions injecting CSS into a page you
            // are trying to measure, which is what you want for testing anyway.
            format!("--user-data-dir={profile}"),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
        ];
    }

    if browser == Browser::Firefox {
        return vec![
            "-new-window".to_string(),
            url.to_string(),
            "-width".to_string(),
            w.to_string(),
            "-height".to_string(),
            h.to_string(),
        ];
    }

    // Safari: the URL and nothing else.
    vec![url.to_string()]
}

/// Open one URL in one browser at one size.
///
/// `profile_root` is where a Chromium profile is kept. One directory per
/// browser, so switching between Chrome and Brave does not make each one
/// rebuild the other's profile on every launch.
pub fn open(
    browser: Browser,
    url: &str,
    width: f64,
    height: f64,
    profile_root: &PathBuf,
) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("that panel has no URL yet".into());
    }

    let profile = profile_root.join(browser.id());
    if browser.is_chromium() {
        std::fs::create_dir_all(&profile)
            .map_err(|e| format!("could not make a profile directory for {}: {e}", browser.app_name()))?;
    }

    let args = args_for(browser, url, width, height, &profile.to_string_lossy());

    // `-n` because a new instance is what carries the sizing flags, and `-a`
    // names the application rather than letting the URL pick the default
    // browser. Safari gets neither: there is nothing to size, so reusing the
    // running copy is friendlier than starting a second one.
    let mut command = Command::new("open");
    if browser.can_size() {
        command.arg("-n");
    }
    command.arg("-a").arg(browser.app_name());
    if browser == Browser::Safari {
        command.arg(url);
    } else {
        command.arg("--args").args(&args);
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", browser.app_name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_gets_app_mode_and_a_size() {
        let args = args_for(Browser::Chrome, "https://example.test/", 768.0, 1020.0, "/tmp/p");
        assert!(args.contains(&"--app=https://example.test/".to_string()));
        assert!(args.contains(&"--window-size=768,1020".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/p".to_string()));
    }

    #[test]
    fn brave_and_edge_take_the_same_flags_as_chrome() {
        let chrome = args_for(Browser::Chrome, "https://a.test/", 640.0, 800.0, "/tmp/p");
        for other in [Browser::Brave, Browser::Edge] {
            let args = args_for(other, "https://a.test/", 640.0, 800.0, "/tmp/p");
            assert_eq!(args, chrome, "{} should take Chrome's flags", other.app_name());
        }
    }

    #[test]
    fn a_fractional_width_is_rounded_not_truncated() {
        // Panel widths are f64 and a zoomed panel's width is rarely whole.
        // Truncating 767.6 to 767 would open the browser one pixel below a
        // 768 breakpoint, which is the exact failure the panel layout code
        // already guards against.
        let args = args_for(Browser::Chrome, "https://a.test/", 767.6, 1020.4, "/tmp/p");
        assert!(args.contains(&"--window-size=768,1020".to_string()));
    }

    #[test]
    fn firefox_takes_its_own_flags() {
        let args = args_for(Browser::Firefox, "https://a.test/", 550.0, 730.0, "/tmp/p");
        assert_eq!(
            args,
            vec!["-new-window", "https://a.test/", "-width", "550", "-height", "730"]
        );
    }

    #[test]
    fn safari_gets_only_the_url() {
        let args = args_for(Browser::Safari, "https://a.test/", 550.0, 730.0, "/tmp/p");
        assert_eq!(args, vec!["https://a.test/"]);
        assert!(!Browser::Safari.can_size());
    }

    #[test]
    fn ids_round_trip() {
        for browser in Browser::ALL {
            assert_eq!(Browser::from_id(browser.id()), Some(browser));
        }
        assert_eq!(Browser::from_id("netscape"), None);
    }

    #[test]
    fn an_empty_url_is_refused_before_anything_is_launched() {
        let err = open(Browser::Chrome, "   ", 800.0, 600.0, &PathBuf::from("/tmp")).unwrap_err();
        assert!(err.contains("no URL"), "{err}");
    }
}
