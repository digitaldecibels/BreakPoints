//! The viewport row: spawning panels, placing them, scrolling them, and keeping
//! their scroll positions in step.
//!
//! Panels are real child webviews positioned at absolute pixel coordinates
//! inside the window. They are not part of the chrome's DOM, so nothing in CSS
//! can move them; every position here is computed in Rust and pushed.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    utils::config::BackgroundThrottlingPolicy,
    webview::{PageLoadEvent, Webview, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, Wry,
};

use crate::model::{FitMode, PanelState, Viewport};
use crate::state::{Endpoint, Shared};

// The chrome's vertical rhythm, mirrored from src/ui/metrics.js. These two
// files are the contract between the chrome layer and the panel positions, so
// they have to agree exactly.
pub const TOOLBAR_H: f64 = 56.0;
/// The pan strip, which sits along the bottom edge rather than under the
/// toolbar: it is a scrollbar, and a scrollbar belongs at the end of the thing
/// it scrolls.
pub const STRIP_H: f64 = 12.0;
/// 26 rather than 16: the panel names sat too close under the toolbar. The
/// chrome's own `mt-*` on the label strip has to match this exactly.
pub const GAP_ABOVE_LABELS: f64 = 26.0;
pub const LABEL_H: f64 = 32.0;
/// The gap between the bottom of the label strip and the top of the panels.
///
/// 46 rather than the 16 the design asks for. Measured on 12 September 2026,
/// the chrome draws its label strip at 72..104 and Rust places panels at
/// `PANEL_TOP`, which should leave 16px clear, and on screen the panels still
/// covered the size line under each panel's name. Both sides' arithmetic
/// checked out, so something between the chrome's CSS origin and the origin a
/// child webview is positioned against differs by roughly the height of a
/// macOS title bar. This does not explain that; it moves the panels out of the
/// way of it. 56 cleared them with room to spare and 36 still cleared them;
/// 46 is where it settled by eye. The cost is 30px of panel height, which
/// `available_height` already subtracts.
pub const GAP_BELOW_LABELS: f64 = 46.0;
pub const PANEL_TOP: f64 = TOOLBAR_H + GAP_ABOVE_LABELS + LABEL_H + GAP_BELOW_LABELS;
pub const PANEL_GAP: f64 = 24.0;
pub const OUTER_MARGIN: f64 = 24.0;
pub const BOTTOM_MARGIN: f64 = 24.0;

pub struct Panel {
    pub viewport: Viewport,
    pub webview: Webview<Wry>,
    /// x position when the canvas scroll offset is zero.
    pub home_x: f64,
    pub scale: f64,
    /// On-screen size, which is the declared size times the scale.
    pub width: f64,
    pub height: f64,
    pub state: PanelState,
    pub last_scroll_pct: f64,
    /// The URL of the document this panel is actually showing, as reported by
    /// the navigation delegate.
    ///
    /// Not the same as the row's URL, and the difference is load bearing. A row
    /// opened on http that redirects to https leaves the row's URL on http,
    /// and whether a panel's messages have to be collected by the pump depends
    /// on the protocol of the document the panel really has. Deciding that from
    /// the row meant every panel queued messages that nobody ever drained,
    /// until the queue overflowed and started discarding, with scroll sync,
    /// console capture and problem reports all dead and nothing saying so.
    pub document_url: String,
    /// The width the page itself last said it was, or `None` before it has
    /// said anything.
    ///
    /// The declared width being exact is the property this whole app rests on,
    /// and until now nothing checked it. Every page reports its `innerWidth`
    /// on load and that number was forwarded to the window and dropped. When
    /// it disagrees with the declared width, something has gone wrong that is
    /// invisible otherwise: a docked inspector taking the frame, a layout that
    /// happened before the zoom landed, or a width changed while the panels
    /// were hidden and never applied.
    pub reported_width: Option<f64>,
    /// Whether this webview has ever committed a navigation.
    ///
    /// Until it has, wry queues every `eval` as a bare string and throws the
    /// callback away, so asking the page for its message queue empties it and
    /// loses whatever was in it. See the trap in CLAUDE.md.
    pub ever_committed: bool,
}

pub struct Canvas {
    pub panels: Vec<Panel>,
    pub scroll_x: f64,
    pub total_width: f64,
    pub url: String,
    pub zoom_to_fit: bool,
    pub fit_mode: FitMode,
    pub sync_on: bool,
    /// Whether a link followed in one panel is followed in all of them.
    pub follow: Follow,
    /// Whether the panels are armed to pick an element and describe a problem.
    pub picking: bool,
    /// Every panel as tall as the canvas, instead of at its declared height.
    pub full_height: bool,
    /// True while a sheet is open. Panels are hidden so the chrome can draw
    /// over the canvas; child webviews always composite above it.
    pub panels_hidden: bool,
    /// The panel whose Web Inspector is open, if any.
    ///
    /// Inspecting is a mode rather than a one-off action, because opening the
    /// inspector does two things to the row that have to be undone together.
    /// See `set_inspecting`.
    pub inspecting: Option<String>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            panels: Vec::new(),
            scroll_x: 0.0,
            total_width: 0.0,
            url: String::new(),
            zoom_to_fit: true,
            fit_mode: FitMode::Height,
            sync_on: true,
            follow: Follow::default(),
            picking: false,
            full_height: false,
            panels_hidden: false,
            inspecting: None,
        }
    }
}

/// What the chrome needs in order to draw the label strip aligned to the panels
/// and to size the scroll strip. Every number here is the true one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelInfo {
    pub id: String,
    pub name: String,
    pub source: String,
    /// Which panel this is counting from the left, starting at one.
    ///
    /// Nobody looking at the row says "panel zero". They say "the second one"
    /// or "frame 2", and an agent handed only a zero-based index has to guess
    /// which of the two they meant.
    pub position: usize,
    /// Declared viewport size, which is what the page actually renders at.
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub home_x: f64,
    pub on_screen_width: f64,
    pub on_screen_height: f64,
    pub state: PanelState,
    /// How many errors this panel's page has logged since it loaded.
    ///
    /// The console has always been captured and only an agent could see it, so
    /// an error that happens at one width and nowhere else was invisible
    /// unless somebody thought to ask.
    pub console_errors: usize,
    /// The document this panel is actually showing. Empty until it has
    /// reported one. It can differ from the row's URL, and when it does that
    /// is worth seeing: a site that redirects on width puts two panels on two
    /// different pages.
    pub document_url: String,
    /// What the page says its viewport is, once it has said anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_width: Option<f64>,
    /// True when the page disagrees with the label by more than a pixel.
    ///
    /// This is the one claim the app cannot afford to get wrong, so it is
    /// reported rather than corrected: a panel drawing at a width other than
    /// the one on its label makes every measurement taken from it worthless,
    /// and silently fixing the number would hide the cause.
    pub width_mismatch: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasInfo {
    pub panels: Vec<PanelInfo>,
    pub total_width: f64,
    pub scroll_x: f64,
    pub panel_top: f64,
    pub url: String,
    pub zoom_to_fit: bool,
    pub scroll_sync: bool,
    pub follow_links: bool,
    pub picking: bool,
    pub full_height: bool,
    /// Which panel is being inspected, so the chrome can say so and offer a
    /// way out of the mode.
    pub inspecting: Option<String>,
}

/// Where one panel sits, before any webview exists. Pure arithmetic so the
/// layout can be reasoned about and tested without a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub home_x: f64,
    pub scale: f64,
    pub width: f64,
    pub height: f64,
}

/// Lay a row of viewports out left to right.
///
/// Zoom-to-fit scales each panel down so it fits the available height. The page
/// still renders at the declared width, because the webview's zoom and its
/// frame shrink together, which leaves the CSS viewport untouched and the media
/// queries firing where they should.
pub fn layout(
    viewports: &[Viewport],
    available_height: f64,
    zoom_to_fit: bool,
    fit_mode: FitMode,
    full_height: bool,
) -> (Vec<Placement>, f64) {
    // Every panel as tall as the canvas allows, at its real width.
    //
    // A viewport's declared height is a guess at a device, which is useful for
    // judging a phone and useless when what you want is to see as much of a
    // page as the window can show. This keeps the width exactly, because the
    // width is the breakpoint and the breakpoint is the whole point, and takes
    // the height from the window instead.
    //
    // It overrides zoom to fit rather than combining with it. Fit exists to
    // make a panel short enough to see all of; this makes it as tall as the
    // window allows. Doing both at once has no meaning.
    if full_height {
        let mut x = OUTER_MARGIN;
        let mut out = Vec::with_capacity(viewports.len());
        for vp in viewports {
            let width = vp.width.round().max(1.0);
            out.push(Placement {
                home_x: x,
                scale: 1.0,
                width,
                height: available_height.round().max(1.0),
            });
            x += width + PANEL_GAP;
        }
        return (out, (x - PANEL_GAP + OUTER_MARGIN).max(0.0));
    }

    let uniform = if zoom_to_fit && fit_mode == FitMode::Uniform {
        viewports
            .iter()
            .map(|v| available_height / v.height.max(1.0))
            .fold(1.0_f64, f64::min)
            .clamp(0.05, 1.0)
    } else {
        1.0
    };

    let mut x = OUTER_MARGIN;
    let mut out = Vec::with_capacity(viewports.len());
    for vp in viewports {
        let nominal = if !zoom_to_fit {
            1.0
        } else {
            match fit_mode {
                FitMode::Uniform => uniform,
                FitMode::Height => (available_height / vp.height.max(1.0)).clamp(0.05, 1.0),
            }
        };

        // A webview's frame is whole pixels, so the frame width gets rounded.
        // The CSS viewport is frame width divided by zoom, so the zoom has to
        // be derived back from the rounded frame or the page renders a pixel
        // narrow: at 767 a `min-width: 768px` query does not fire, and the
        // whole point of this app is that it does. Exact width beats an exact
        // scale, and under uniform fit the scales can now differ by a fraction
        // of a percent as a result.
        let width = (vp.width * nominal).round().max(1.0);
        let scale = width / vp.width.max(1.0);
        let height = (vp.height * scale).round();

        out.push(Placement {
            home_x: x,
            scale,
            width,
            height,
        });
        x += width + PANEL_GAP;
    }
    let total = if viewports.is_empty() {
        0.0
    } else {
        x - PANEL_GAP + OUTER_MARGIN
    };
    (out, total)
}

/// Height left for panels once the chrome has taken its share.
/// Whether a queued message is somebody moving the row.
///
/// The pump speeds up after a gesture because one is usually the start of
/// several. A console line is not a gesture, and neither is a page load.
fn is_gesture(message: &serde_json::Value) -> bool {
    matches!(
        message.get("path").and_then(|p| p.as_str()),
        Some("/p/scroll") | Some("/p/wheel")
    )
}

/// Remember the window's size, so nothing on a hot path has to ask for it.
///
/// Asking sends a message to the main thread and blocks until it answers, with
/// no timeout, which is the last thing a per-frame path should do against a
/// main thread laying out seven pages.
pub fn remember_window_size(app: &AppHandle, state: &Shared) {
    let Some(window) = app.get_window("main") else { return };
    let Ok(size) = window.inner_size() else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    *state.window_width.lock().unwrap() = size.width as f64 / scale;
    *state.window_height.lock().unwrap() = size.height as f64 / scale;
}

/// The height a panel may use, from the remembered window size.
pub fn available_height_cached(app: &AppHandle, state: &Shared) -> f64 {
    let cached = *state.window_height.lock().unwrap();
    if cached <= 0.0 {
        remember_window_size(app, state);
        let cached = *state.window_height.lock().unwrap();
        if cached <= 0.0 {
            return 600.0;
        }
        return (cached - PANEL_TOP - BOTTOM_MARGIN - STRIP_H).max(120.0);
    }
    (cached - PANEL_TOP - BOTTOM_MARGIN - STRIP_H).max(120.0)
}

/// Whether a panel is on screen, or close enough to be about to be.
///
/// The row is routinely four times the width of the window: seven Bucknell
/// viewports come to 6365px and a 1400px window shows three of them. Asking the
/// other four for a scroll position nobody can have changed was more than half
/// of everything the pump did.
///
/// The margin means a panel is already being collected from by the time it
/// slides into view, rather than a tick late.
pub fn panel_on_screen(home_x: f64, width: f64, scroll_x: f64, window_width: f64) -> bool {
    const MARGIN: f64 = 200.0;
    // Nobody has told us how wide the window is yet, so assume everything is
    // visible. Guessing the other way would silently stop collecting from the
    // whole row.
    if window_width <= 0.0 {
        return true;
    }
    let left = home_x - scroll_x;
    left + width > -MARGIN && left < window_width + MARGIN
}


/// Injected into every panel at document start, on every navigation.
///
/// It cannot use `window.__TAURI__`: on macOS an injected script runs without
/// the Tauri IPC bridge on a page we did not serve. So it posts to the loopback
/// callback server instead, and Rust pushes back with `eval`.
///
/// There is one page it cannot post from, and it is the common one. WebKit
/// blocks an http request made by an https document, and unlike Chrome it does
/// not exempt `127.0.0.1`. Every Lando, DDEV and hosted site is https, so on
/// those pages the loopback route is dead: no ready report, no console lines,
/// no scroll sync. Those pages queue their messages in `window.__bpOut`
/// instead and Rust collects them with `eval` (see `pump`).
fn injected_script(panel_id: &str, endpoint: &Endpoint) -> String {
    format!(
        // r##"…"## rather than r#"…"#: the picker builds id selectors with
        // `"#" + id`, and that sequence would close a single-hash raw string.
        r##"(function () {{
  if (window.__bpInstalled) return;
  window.__bpInstalled = true;
  var ID = {id};
  var URL_BASE = "http://127.0.0.1:{port}";
  var NONCE = {nonce};
  var ignoreUntil = 0, lastSent = 0, pending = null;
  var THROTTLE_MS = 60;

  // See the comment on this function in Rust. An https page cannot reach an
  // http loopback server in WebKit, so it queues instead of posting.
  var CAN_POST = location.protocol !== "https:";
  var OUT = [];
  var MAX_QUEUED = 300;

  // Called from Rust on every pump tick. Hands over the queue and empties it.
  window.__bpDrain = function () {{
    var queued = OUT;
    OUT = [];
    return queued;
  }};

  function post(path, body) {{
    body.panel = ID;
    body.nonce = NONCE;
    if (!CAN_POST) {{
      // Two of the same kind in a row are one decision written twice: the
      // newest scroll position wins, and wheel deltas add up.
      var last = OUT[OUT.length - 1];
      if (last && last.path === path && path === "/p/scroll") {{
        last.body = body;
        return;
      }}
      if (last && last.path === path && path === "/p/wheel") {{
        last.body.dx += body.dx;
        return;
      }}
      OUT.push({{ path: path, body: body }});
      if (OUT.length > MAX_QUEUED) OUT.splice(0, OUT.length - MAX_QUEUED);
      return;
    }}
    try {{
      fetch(URL_BASE + path, {{
        method: "POST",
        // text/plain keeps this a simple request, so there is no preflight.
        headers: {{ "content-type": "text/plain;charset=UTF-8" }},
        body: JSON.stringify(body),
        keepalive: true,
      }}).catch(function () {{}});
    }} catch (e) {{}}
  }}

  function maxScroll() {{
    return Math.max(0, document.documentElement.scrollHeight - window.innerHeight);
  }}

  // Called from Rust. The ignore window stops two panels pushing each other
  // back and forth forever.
  window.__bpApplyScroll = function (pct) {{
    ignoreUntil = Date.now() + 200;
    window.scrollTo(0, pct * maxScroll());
  }};

  function report() {{
    lastSent = Date.now();
    var m = maxScroll();
    post("/p/scroll", {{ pct: m > 0 ? window.scrollY / m : 0 }});
  }}

  // Deliberately not requestAnimationFrame. WebKit suspends rAF in a webview
  // that is not in the frontmost window, which is exactly the case when an
  // agent is driving the app from a terminal, and the callback simply never
  // runs. A timer keeps working, and a leading-edge throttle still reports
  // about sixteen times a second while you drag.
  window.addEventListener("scroll", function () {{
    if (Date.now() < ignoreUntil) return;
    var since = Date.now() - lastSent;
    if (since >= THROTTLE_MS) {{
      report();
      return;
    }}
    if (pending) return;
    pending = setTimeout(function () {{
      pending = null;
      if (Date.now() >= ignoreUntil) report();
    }}, THROTTLE_MS - since);
  }}, {{ passive: true }});

  // Something on the page that scrolls sideways and has somewhere left to go,
  // like a carousel. It asked for this gesture before we did.
  function ownsSideways(target, dx) {{
    // A wheel event's target can be the document or the window, neither of
    // which has a parent or a computed style.
    var start = target && target.nodeType === 1 ? target : null;
    for (var n = start; n && n !== document.body; n = n.parentElement) {{
      if (n.scrollWidth <= n.clientWidth + 1) continue;
      var overflow = getComputedStyle(n).overflowX;
      if (overflow !== "auto" && overflow !== "scroll") continue;
      var room = dx > 0
        ? n.scrollWidth - n.clientWidth - n.scrollLeft
        : n.scrollLeft;
      if (room > 1) return true;
    }}
    return false;
  }}

  // A two-finger sideways swipe pans the whole row, because the panel is
  // covering the chrome and the chrome is the only thing that could otherwise
  // hear it. Anything mostly vertical is left alone: it belongs to the page
  // under the pointer, which is what a person expects when they scroll a site.
  window.addEventListener("wheel", function (e) {{
    if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    if (ownsSideways(e.target, e.deltaX)) return;
    e.preventDefault();
    post("/p/wheel", {{ dx: e.deltaX }});
  }}, {{ passive: false }});

  function fmt(v) {{
    if (typeof v === "string") return v;
    if (v instanceof Error) return v.message;
    try {{ return JSON.stringify(v); }} catch (e) {{ return String(v); }}
  }}

  ["log", "info", "warn", "error"].forEach(function (level) {{
    var orig = console[level];
    console[level] = function () {{
      try {{
        post("/p/console", {{
          level: level,
          text: Array.prototype.map.call(arguments, fmt).join(" "),
        }});
      }} catch (e) {{}}
      return orig.apply(console, arguments);
    }};
  }});

  window.addEventListener("error", function (e) {{
    post("/p/console", {{
      level: "error",
      text: (e.message || "Script error") + " @ " + (e.filename || "") + ":" + (e.lineno || 0),
    }});
  }});

  window.addEventListener("unhandledrejection", function (e) {{
    post("/p/console", {{ level: "error", text: "Unhandled rejection: " + fmt(e.reason) }});
  }});

  // Used while a full-page shot walks the page. A sticky header painted into
  // every tile is the thing that makes a stitched screenshot obviously fake,
  // so it is hidden for every tile after the first and put back afterwards.
  var pinned = [];
  window.__bpHidePinned = function () {{
    if (pinned.length) return;
    var all = document.body ? document.body.querySelectorAll("*") : [];
    for (var i = 0; i < all.length; i++) {{
      var position = getComputedStyle(all[i]).position;
      if (position !== "fixed" && position !== "sticky") continue;
      pinned.push([all[i], all[i].style.visibility]);
      all[i].style.visibility = "hidden";
    }}
  }};
  window.__bpShowPinned = function () {{
    for (var i = 0; i < pinned.length; i++) {{
      pinned[i][0].style.visibility = pinned[i][1];
    }}
    pinned = [];
  }};

  // A single page app changes the URL without loading a document, so this
  // script never runs again and a full load report never arrives. Watch the
  // history API instead.
  //
  // The fragment is deliberately left out of the comparison. An anchor link
  // changes only the hash, and scroll sync already carries the other panels to
  // the same place; reloading them for a jump inside a page they already have
  // would be worse than doing nothing. Analytics code also rewrites the hash
  // constantly, and every one of those would otherwise be a navigation.
  function here() {{
    return location.origin + location.pathname + location.search;
  }}
  var lastNav = here();
  function softNav() {{
    var now = here();
    if (now === lastNav) return;
    lastNav = now;
    post("/p/nav", {{ url: location.href }});
  }}
  ["pushState", "replaceState"].forEach(function (name) {{
    var orig = history[name];
    if (typeof orig !== "function") return;
    history[name] = function () {{
      var out = orig.apply(this, arguments);
      try {{ softNav(); }} catch (e) {{}}
      return out;
    }};
  }});
  window.addEventListener("popstate", softNav);

  // ---- Reporting a problem ------------------------------------------------
  //
  // Hover to highlight, click to describe. The whole thing lives inside the
  // page because the chrome cannot draw over a panel: panels are child webviews
  // that composite on top of it.
  //
  // Everything it adds is marked data-bp-ui, is ignored by the picker itself,
  // and is torn out again when picking stops, so the page is left as it was.
  var picking = false, hovered = null, box = null, tag = null, form = null;

  function ui(kind) {{
    var el = document.createElement(kind);
    el.setAttribute("data-bp-ui", "1");
    return el;
  }}

  function ours(node) {{
    for (var n = node; n; n = n.parentElement) {{
      if (n.nodeType === 1 && n.hasAttribute && n.hasAttribute("data-bp-ui")) return true;
    }}
    return false;
  }}

  // A selector you can paste into the console and get the same element back.
  // An id wins outright; otherwise walk up, using nth-of-type to stay exact.
  function selectorFor(el) {{
    if (!el || el.nodeType !== 1) return "";
    if (el.id) return "#" + CSS.escape(el.id);
    var parts = [];
    for (var n = el; n && n.nodeType === 1 && n !== document.documentElement; n = n.parentElement) {{
      var part = n.tagName.toLowerCase();
      if (n.id) {{ parts.unshift("#" + CSS.escape(n.id)); break; }}
      var parent = n.parentElement;
      if (parent) {{
        var same = [];
        for (var i = 0; i < parent.children.length; i++) {{
          if (parent.children[i].tagName === n.tagName) same.push(parent.children[i]);
        }}
        if (same.length > 1) part += ":nth-of-type(" + (same.indexOf(n) + 1) + ")";
      }}
      parts.unshift(part);
      if (parts.length > 6) break;
    }}
    return parts.join(" > ");
  }}

  // What a person would call this thing, which is not the same as its selector.
  function nameFor(el) {{
    var name = el.tagName.toLowerCase();
    if (el.id) return name + "#" + el.id;
    var cls = (el.getAttribute("class") || "").trim().split(/\s+/).filter(Boolean);
    return cls.length ? name + "." + cls.slice(0, 3).join(".") : name;
  }}

  function place(el) {{
    var r = el.getBoundingClientRect();
    box.style.top = (r.top + window.scrollY) + "px";
    box.style.left = (r.left + window.scrollX) + "px";
    box.style.width = r.width + "px";
    box.style.height = r.height + "px";
    tag.textContent = nameFor(el) + "  " + Math.round(r.width) + " x " + Math.round(r.height);
    // Above the box unless it would go off the top.
    var above = r.top + window.scrollY - 20;
    tag.style.top = (above > window.scrollY ? above : r.top + window.scrollY + r.height) + "px";
    tag.style.left = (r.left + window.scrollX) + "px";
  }}

  function onMove(e) {{
    if (!picking || form) return;
    var el = e.target;
    if (!el || el.nodeType !== 1 || ours(el)) return;
    hovered = el;
    place(el);
  }}

  // Describe something without arming anything first.
  //
  // Reporting was two steps: turn the mode on, then click. A held modifier is
  // the same promise the mode makes, that this click describes rather than
  // follows, without having to say so in advance. Command and shift together,
  // because either alone already means something to a browser.
  function onShortcutClick(e) {{
    if (picking || !(e.metaKey && e.shiftKey)) return;
    if (ours(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    if (form) {{
      closeForm();
      return;
    }}
    arm();
    openForm(e.target);
  }}

  function onClick(e) {{
    if (!picking) return;
    if (ours(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    // Clicking away from an open form dismisses it, the way every other
    // popover on a Mac behaves.
    if (form) {{
      closeForm();
      return;
    }}
    openForm(hovered || e.target);
  }}

  function button(label, accent) {{
    var b = ui("button");
    b.type = "button";
    b.textContent = label;
    b.style.cssText =
      "height:24px;padding:0 10px;border-radius:4px;font:12px inherit;font-weight:500;cursor:pointer;" +
      (accent
        ? "background:#4c8dff;color:#0e1520;border:1px solid #4c8dff;"
        : "background:#252529;color:#e8e8ea;border:1px solid #323236;");
    return b;
  }}

  function openForm(el) {{
    hovered = el;
    place(el);
    form = ui("div");
    form.style.cssText =
      "position:absolute;z-index:2147483647;background:#1c1c1f;border:1px solid #4c8dff;" +
      "border-radius:6px;padding:10px;width:320px;box-shadow:0 8px 24px rgba(0,0,0,.5);" +
      "font:13px -apple-system,system-ui,sans-serif;color:#e8e8ea;";
    var r = el.getBoundingClientRect();
    form.style.top = (r.bottom + window.scrollY + 8) + "px";
    form.style.left = Math.max(8, Math.min(r.left + window.scrollX, window.scrollX + window.innerWidth - 340)) + "px";

    // A titlebar with a close control, because a box with no way out of it is
    // a trap however many keys happen to dismiss it.
    var head = ui("div");
    head.style.cssText = "display:flex;align-items:flex-start;gap:8px;margin-bottom:6px;";
    var what = ui("div");
    what.style.cssText = "flex:1;min-width:0;font:11px ui-monospace,monospace;color:#8a8a92;word-break:break-all;";
    what.textContent = nameFor(el) + " at " + window.innerWidth + "px";
    var close = ui("button");
    close.type = "button";
    close.setAttribute("aria-label", "Close");
    close.textContent = "×";
    close.style.cssText =
      "flex:0 0 auto;width:20px;height:20px;line-height:18px;padding:0;border-radius:4px;" +
      "background:transparent;border:1px solid #323236;color:#8a8a92;font:14px inherit;cursor:pointer;";
    head.appendChild(what);
    head.appendChild(close);

    var input = ui("textarea");
    input.setAttribute("placeholder", "What is wrong here?");
    input.style.cssText =
      "width:100%;box-sizing:border-box;height:64px;resize:none;background:#141416;" +
      "border:1px solid #323236;border-radius:4px;color:#e8e8ea;padding:6px;font:13px inherit;outline:none;";

    var foot = ui("div");
    foot.style.cssText = "display:flex;align-items:center;gap:8px;margin-top:8px;";
    var hint = ui("div");
    hint.style.cssText = "flex:1;min-width:0;font:11px -apple-system,system-ui,sans-serif;color:#5c5c63;";
    hint.textContent = "Enter sends";
    var cancel = button("Cancel", false);
    var send = button("Send", true);
    foot.appendChild(hint);
    foot.appendChild(cancel);
    foot.appendChild(send);

    form.appendChild(head);
    form.appendChild(input);
    form.appendChild(foot);
    document.body.appendChild(form);
    input.focus();

    function submit() {{
      var note = input.value.trim();
      if (!note) {{ input.focus(); return; }}
      var rect = el.getBoundingClientRect();
      post("/p/report", {{
        note: note,
        selector: selectorFor(el),
        element: nameFor(el),
        innerWidth: window.innerWidth,
        url: location.href,
        title: document.title,
        rect: {{
          x: Math.round(rect.left), y: Math.round(rect.top + window.scrollY),
          w: Math.round(rect.width), h: Math.round(rect.height),
        }},
      }});
      closeForm();
    }}

    close.addEventListener("click", function (ev) {{ ev.preventDefault(); ev.stopPropagation(); closeForm(); }});
    cancel.addEventListener("click", function (ev) {{ ev.preventDefault(); ev.stopPropagation(); closeForm(); }});
    send.addEventListener("click", function (ev) {{ ev.preventDefault(); ev.stopPropagation(); submit(); }});

    input.addEventListener("keydown", function (ev) {{
      ev.stopPropagation();
      if (ev.key === "Escape") {{ ev.preventDefault(); closeForm(); return; }}
      if (ev.key === "Enter" && !ev.shiftKey) {{
        ev.preventDefault();
        submit();
      }}
    }});

    form.__bpSubmit = submit;
  }}

  function closeForm() {{
    if (form && form.parentNode) form.parentNode.removeChild(form);
    form = null;
  }}

  // Escape closes whatever is in front of you: the form first, then the mode.
  // Keying this off the focused element meant Escape did nothing whenever the
  // cursor had left the box, which is a box you cannot get out of.
  function onKey(e) {{
    if (!picking || e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    if (form) closeForm();
    else window.__bpPick(false);
  }}

  // The highlight and the listeners, without changing the mode.
  function arm() {{
    if (box) return;
    box = ui("div");
    box.style.cssText =
      "position:absolute;z-index:2147483646;pointer-events:none;background:rgba(76,141,255,.16);" +
      "outline:1px solid #4c8dff;border-radius:2px;top:0;left:0;width:0;height:0;";
    tag = ui("div");
    tag.style.cssText =
      "position:absolute;z-index:2147483647;pointer-events:none;background:#4c8dff;color:#0e1520;" +
      "font:10px ui-monospace,monospace;padding:2px 5px;border-radius:3px;white-space:nowrap;top:0;left:0;";
    document.body.appendChild(box);
    document.body.appendChild(tag);
  }}

  document.addEventListener("click", onShortcutClick, true);

  window.__bpPick = function (on) {{
    if (on === picking) return picking;
    picking = !!on;
    if (picking) {{
      box = ui("div");
      box.style.cssText =
        "position:absolute;z-index:2147483646;pointer-events:none;background:rgba(76,141,255,.16);" +
        "outline:1px solid #4c8dff;border-radius:2px;top:0;left:0;width:0;height:0;";
      tag = ui("div");
      tag.style.cssText =
        "position:absolute;z-index:2147483647;pointer-events:none;background:#4c8dff;color:#0e1520;" +
        "font:10px ui-monospace,monospace;padding:2px 5px;border-radius:3px;white-space:nowrap;top:0;left:0;";
      document.body.appendChild(box);
      document.body.appendChild(tag);
      document.addEventListener("mousemove", onMove, true);
      document.addEventListener("click", onClick, true);
      document.addEventListener("keydown", onKey, true);
    }} else {{
      closeForm();
      document.removeEventListener("mousemove", onMove, true);
      document.removeEventListener("click", onClick, true);
      document.removeEventListener("keydown", onKey, true);
      [box, tag].forEach(function (el) {{ if (el && el.parentNode) el.parentNode.removeChild(el); }});
      box = null;
      tag = null;
      hovered = null;
    }}
    return picking;
  }};

  function ready() {{
    post("/p/ready", {{ url: location.href, title: document.title, width: window.innerWidth }});
  }}
  if (document.readyState === "loading") {{
    document.addEventListener("DOMContentLoaded", ready);
  }} else {{
    ready();
  }}
}})();"##,
        id = serde_json::to_string(panel_id).unwrap(),
        port = endpoint.port,
        nonce = serde_json::to_string(&endpoint.nonce).unwrap(),
    )
}

/// Tear the row down and build a fresh one. Returns the total row width so the
/// chrome can size its scroll strip.
pub async fn spawn(
    app: &AppHandle,
    state: &Shared,
    viewports: Vec<Viewport>,
    url: &str,
) -> Result<f64, String> {
    // One rebuild at a time. See `AppState::spawning` for what interleaving
    // two of them does to the row.
    let _building = state.spawning.lock().await;

    let window = app.get_window("main").ok_or("no main window")?;
    let endpoint = state.endpoint.get().cloned().ok_or("callback server not started")?;

    // Take the old panels out of the lock before closing them; closing
    // dispatches to the main thread and must not run while the mutex is held.
    // Ids are reused across rebuilds, so a claim left over from the row being
    // torn down would mark its replacement committed before it has loaded.
    state.committed_early.lock().unwrap().clear();

    let old: Vec<Panel> = {
        let mut canvas = state.canvas.lock().unwrap();
        canvas.panels.drain(..).collect()
    };
    for panel in old {
        let _ = panel.webview.close();
    }

    let target = crate::util::normalize_url(url);
    // Every panel is born here and sent to the real page once it is the right
    // size. `normalize_url("")` is about:blank.
    let blank = crate::util::normalize_url("");
    let (zoom_to_fit, fit_mode, full_height) = {
        let canvas = state.canvas.lock().unwrap();
        (canvas.zoom_to_fit, canvas.fit_mode, canvas.full_height)
    };
    let (places, total) =
        layout(&viewports, available_height_cached(app, state), zoom_to_fit, fit_mode, full_height);

    // The row is emptied first and then filled in one panel at a time, because
    // a page can finish loading and report in before the last panel has even
    // been created. Holding the whole list back until the end silently dropped
    // those reports and left every panel but the last marked as loading.
    {
        let mut canvas = state.canvas.lock().unwrap();
        canvas.scroll_x = 0.0;
        canvas.total_width = total;
        canvas.url = target.to_string();
        canvas.panels_hidden = false;
    }

    for (vp, place) in viewports.into_iter().zip(places) {
        let handle = app.clone();
        let load_state = state.clone();
        let panel_id = vp.id.clone();
        // Built blank, zoomed, and only then sent to the page.
        //
        // A webview created at its scaled size starts loading immediately, and
        // the zoom is a separate message that lands afterwards, so with
        // fit-to-width on a 1024 panel used to lay out its first document as a
        // 512 viewport. WebKit re-lays out when the zoom arrives, but anything
        // the page decides once is already decided: a media query read at parse
        // time, a handler reading innerWidth, an image picking a source, a
        // framework choosing a breakpoint on boot. For an app whose whole claim
        // is the exactness of that width, that is the worst moment to be wrong.
        let builder = WebviewBuilder::new(
            format!("panel-{}", vp.id),
            WebviewUrl::External(blank.clone()),
        )
        .initialization_script(injected_script(&vp.id, &endpoint))
        .disable_drag_drop_handler()
        .zoom_hotkeys_enabled(false)
        // Panels have to keep running when the app is not frontmost, because
        // an agent drives them from a terminal. Suspended timers mean scroll
        // sync and console capture quietly stop.
        .background_throttling(BackgroundThrottlingPolicy::Disabled)
        .on_page_load(move |_wv, payload| {
            let phase = match payload.event() {
                PageLoadEvent::Started => "started",
                PageLoadEvent::Finished => "finished",
            };
            let _ = handle.emit(
                "panel:load",
                serde_json::json!({ "panel": panel_id, "phase": phase }),
            );

            // This is the only report of a page load that does not depend on
            // the page cooperating. The injected script's own `/p/ready` has to
            // travel back through the callback server or the pump, and neither
            // is reliable straight after a `navigate`, which left panels
            // showing "loading" forever and stopped a followed link dead.
            // Tauri's navigation delegate has no such problem, and it carries
            // the URL, which is exactly what following needs.
            let app = handle.clone();
            let state = load_state.clone();
            let id = panel_id.clone();
            let url = payload.url().to_string();
            let finished = matches!(payload.event(), PageLoadEvent::Finished);
            // Off the delegate, so nothing here can re-enter a webview call
            // from inside a webview callback.
            tauri::async_runtime::spawn(async move {
                // Either phase means this webview has a document, so it is safe
                // to ask questions of. `Finished` counts as well as `Started`,
                // because a load can be reported finished without this task
                // ever having seen it start.
                mark_committed(&state, &id);
                // Which protocol this panel's own document is on decides
                // whether its messages have to be collected.
                set_document_url(&state, &id, &url);

                // A load that lands nowhere is reported as starting and never
                // as finishing, so this cannot live in the finished branch.
                // Measured: pointing the row at a closed port produces one
                // report per panel, phase started, document about:blank, and
                // the panels keep whatever state they already had.
                let requested = state.canvas.lock().unwrap().url.clone();
                if load_failed(&url, &requested) {
                    confirm_load_failure(app.clone(), state.clone(), id.clone(), requested);
                    return;
                }

                if finished {
                    set_panel_state(&app, &state, &id, PanelState::Loaded);
                    rearm_picking(&state, &id);
                    follow_navigation(&app, &state, &id, &url);
                } else {
                    set_panel_state(&app, &state, &id, PanelState::Loading);
                }
            });
        });

        let webview = match window.add_child(
            builder,
            LogicalPosition::new(place.home_x, PANEL_TOP),
            LogicalSize::new(place.width, place.height),
        ) {
            Ok(webview) => webview,
            Err(err) => {
                // Half a row is not a row of the width we promised. The strip
                // and every crop rectangle downstream are derived from this
                // number, so leave it describing what actually exists.
                {
                    let mut canvas = state.canvas.lock().unwrap();
                    canvas.total_width = canvas
                        .panels
                        .iter()
                        .map(|p| p.home_x + p.width)
                        .fold(0.0_f64, f64::max);
                }
                emit_canvas(app, state);
                return Err(err.to_string());
            }
        };

        let _ = webview.set_zoom(place.scale);
        // The viewport is now the declared width, so go to the real page.
        let _ = webview.navigate(target.clone());

        // Claimed before the canvas lock is taken, because `mark_committed`
        // takes them in the other order.
        let committed = state.committed_early.lock().unwrap().remove(&vp.id);

        state.canvas.lock().unwrap().panels.push(Panel {
            viewport: vp,
            webview,
            home_x: place.home_x,
            scale: place.scale,
            width: place.width,
            height: place.height,
            state: PanelState::Loading,
            last_scroll_pct: UNKNOWN_SCROLL,
            document_url: String::new(),
            reported_width: None,
            ever_committed: committed,
        });

        // Panels sometimes render white when several spawn in the same frame.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    emit_canvas(app, state);
    Ok(total)
}

/// Recompute placement against the current window size and push it. Used on
/// resize and whenever zoom-to-fit changes, so no panel is ever respawned just
/// to change its scale.
pub fn relayout(app: &AppHandle, state: &Shared) {
    let avail = available_height_cached(app, state);
    let mut canvas = state.canvas.lock().unwrap();
    let viewports: Vec<Viewport> = canvas.panels.iter().map(|p| p.viewport.clone()).collect();
    let (places, total) = layout(&viewports, avail, canvas.zoom_to_fit, canvas.fit_mode, canvas.full_height);
    let scroll_x = canvas.scroll_x.min((total - 100.0).max(0.0));
    canvas.scroll_x = scroll_x;
    canvas.total_width = total;

    for (panel, place) in canvas.panels.iter_mut().zip(places) {
        panel.home_x = place.home_x;
        panel.scale = place.scale;
        panel.width = place.width;
        panel.height = place.height;
        // Applied even while the panels are hidden. Skipping it left the record
        // and the webview disagreeing for as long as a sheet stayed open, so
        // `set_viewport` returned a width the page did not have and anything
        // that measured in the interval measured the old one. Setting a frame
        // does not reveal a hidden webview; only `show` does, and that stays
        // in `set_panels_hidden`.
        let _ = panel.webview.set_zoom(place.scale);
        let _ = panel
            .webview
            .set_position(LogicalPosition::new(place.home_x - scroll_x, PANEL_TOP));
        let _ = panel
            .webview
            .set_size(LogicalSize::new(place.width, place.height));
    }
    drop(canvas);
    emit_canvas(app, state);
}

/// Slide the whole row. Called from the chrome's scroll strip, at frame rate,
/// so it does the least work it can and never emits.
pub fn set_scroll(state: &Shared, offset: f64) {
    let mut canvas = state.canvas.lock().unwrap();
    if canvas.panels_hidden {
        canvas.scroll_x = offset;
        return;
    }
    canvas.scroll_x = offset;
    for panel in canvas.panels.iter() {
        let _ = panel
            .webview
            .set_position(LogicalPosition::new(panel.home_x - offset, PANEL_TOP));
    }
}

/// Where a pan of `dx` lands, given a row of `total_width` in a window of
/// `window_width`.
///
/// A row narrower than the window has nowhere to go, and a wheel gesture keeps
/// arriving after you hit either end, so both are clamped here rather than at
/// the call site.
pub fn clamped_scroll(current: f64, dx: f64, total_width: f64, window_width: f64) -> f64 {
    let max = (total_width - window_width).max(0.0);
    (current + dx).clamp(0.0, max)
}

/// Pan the row by a delta and tell the chrome where it landed.
///
/// The scroll strip clamps itself, so `set_scroll` never had to. A wheel has
/// no strip behind it, so the clamping happens here instead.
pub fn nudge_scroll(app: &AppHandle, state: &Shared, dx: f64) {
    // Read, never asked for. Asking the window for its scale and size sends
    // two messages to the main thread and blocks on each until it answers,
    // with no timeout. This runs off the main thread on every wheel event, so
    // during a two-finger pan it was parking a worker twice per event behind a
    // main thread laying out seven pages, and if that thread was inside a
    // modal or the Web Inspector it parked indefinitely.
    let window_width = *state.window_width.lock().unwrap();

    // Read both numbers out of the lock before doing anything that locks again.
    let (current, total_width) = {
        let canvas = state.canvas.lock().unwrap();
        (canvas.scroll_x, canvas.total_width)
    };
    let next = clamped_scroll(current, dx, total_width, window_width);
    if (next - current).abs() < 0.5 {
        return;
    }
    set_scroll(state, next);
    let _ = app.emit("canvas:scroll", next);
}

/// Collect queued messages out of panels that cannot post to the callback
/// server, and hand them to the same handlers a posted message would reach.
///
/// Only https pages are pumped: an http page posts directly and its queue is
/// always empty, so asking it would be pure cost. The tick is slow while
/// nothing is happening and fast for a moment after a wheel or a scroll, which
/// is what makes a two-finger pan track the fingers instead of lurching.
pub fn start_pump(app: AppHandle, state: Shared) {
    /// Straight after a message, because one is usually the start of a gesture.
    const ACTIVE_MS: u64 = 16;
    /// How long a message keeps the fast rate alive.
    const STAY_FAST: std::time::Duration = std::time::Duration::from_millis(400);
    /// The window has focus, so a person could start scrolling at any moment.
    const WATCHFUL_MS: u64 = 150;
    /// Nothing has happened for a while, even with focus.
    const DOZING_MS: u64 = 600;
    /// The window is behind something else. Nobody is scrolling a panel by
    /// hand, and an agent driving the app does not need sixty ticks a second.
    const BACKGROUND_MS: u64 = 1500;
    /// Quiet for this long with focus and the rate drops to `DOZING_MS`.
    const DOZE_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

    /// How often every panel is collected from, on-screen or not.
    const SWEEP_EVERY: u64 = 8;

    tauri::async_runtime::spawn(async move {
        let mut last_message = std::time::Instant::now();
        let mut sweep: u64 = 0;
        loop {
            // Asking a webview a question is not cheap: each one is a script
            // evaluation and a callback through Tauri's own machinery, and six
            // panels at a fixed 120ms cost 12% of a CPU core for as long as the
            // app is open. That is a real problem for an app whose job is
            // measuring how a page performs, so the rate follows what is
            // actually happening.
            let since = last_message.elapsed();
            let focused = *state.window_focused.lock().unwrap();
            let interval = if since < STAY_FAST {
                ACTIVE_MS
            } else if !focused {
                BACKGROUND_MS
            } else if since < DOZE_AFTER {
                WATCHFUL_MS
            } else {
                DOZING_MS
            };
            tokio::time::sleep(std::time::Duration::from_millis(interval)).await;

            // Off-screen panels are collected from occasionally rather than
            // never. Nobody is scrolling one, but a page that logs on a timer
            // still fills its queue, and that queue discards its oldest
            // messages once it is full.
            sweep = sweep.wrapping_add(1);
            let everything = sweep % SWEEP_EVERY == 0;

            let ids: Vec<String> = {
                let canvas = state.canvas.lock().unwrap();
                let window_width = *state.window_width.lock().unwrap();
                canvas
                    .panels
                    .iter()
                    // Asking a panel that has never committed a navigation is
                    // worse than not asking: wry queues the script with no
                    // callback, so the drain runs later, empties the queue and
                    // hands the contents to nobody.
                    .filter(|p| p.ever_committed)
                    // Each panel's own document decides this, never the row's.
                    // A row opened on http that redirects to https left every
                    // panel queueing into a void.
                    .filter(|p| needs_pump(&p.document_url))
                    .filter(|p| {
                        everything
                            || panel_on_screen(p.home_x, p.width, canvas.scroll_x, window_width)
                    })
                    .map(|p| p.viewport.id.clone())
                    .collect()
            };
            if ids.is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                continue;
            }

            // Asked all at once, not one after another. Each panel used to be
            // awaited in turn with its own half-second timeout, so a tick cost
            // the sum of the slow ones rather than the slowest, and one panel
            // with a busy main thread held up every other panel's scroll sync
            // and wheel deltas behind it. Seven wedged panels meant a
            // three-and-a-half second tick and a row that looked frozen.
            let drains = ids.into_iter().map(|id| {
                let state = state.clone();
                async move {
                    let drained = tokio::time::timeout(
                        std::time::Duration::from_millis(500),
                        crate::tools::eval_wrapped(&state, &id, drain_script()),
                    )
                    .await;
                    (id, drained)
                }
            });

            // Dispatched in a fixed order once they are all back, so two
            // panels answering at the same moment cannot interleave their
            // messages.
            for (id, drained) in futures_util::future::join_all(drains).await {
                let Ok(Ok(serde_json::Value::Array(messages))) = drained else {
                    continue;
                };
                // Only a gesture keeps the fast rate alive. Anything in the
                // queue used to, and the injected script captures every
                // console call, so a page that logs on a timer, a framework's
                // dev build, or a page that throws repeatedly held the 16ms
                // rate for the life of the session. That is several times the
                // intended cost, sustained, in an app whose job is measuring
                // how a page performs.
                if messages.iter().any(is_gesture) {
                    last_message = std::time::Instant::now();
                }
                for message in messages {
                    crate::callback::dispatch_from(&app, &state, &id, &message);
                }
            }
        }
    });
}

/// Point one panel at a URL.
///
/// A panel that failed was hidden, so the chrome could draw the failed state at
/// its exact size. Sending it somewhere new has to put it back, or it loads the
/// page invisibly and the row silently has a hole in it. `set_panel_state` does
/// this for a state change that arrives from a report; these two paths set the
/// state themselves and have to do the same.
fn send_to(panel: &mut Panel, target: &url::Url, sheet_open: bool) {
    let was_failed = matches!(panel.state, PanelState::Failed(_));
    panel.state = PanelState::Loading;
    if was_failed && !sheet_open {
        let _ = panel.webview.show();
    }
    let _ = panel.webview.navigate(target.clone());
}

pub fn navigate_all(state: &Shared, url: &str) -> Result<(), String> {
    let target = crate::util::parse_url(url)?;
    let mut canvas = state.canvas.lock().unwrap();
    canvas.url = target.to_string();
    let hidden = canvas.panels_hidden;
    let mut awaiting = HashSet::new();
    for panel in canvas.panels.iter_mut() {
        send_to(panel, &target, hidden);
        awaiting.insert(panel.viewport.id.clone());
    }
    // Every one of those panels is about to report in. Without this the first
    // report back reads as somebody clicking a link.
    canvas.follow.pushed(awaiting, Instant::now());
    Ok(())
}

/// How long a pushed navigation is given to settle before a report counts as a
/// fresh instruction again. A panel that never reports back, because it failed
/// to load, must not wedge following forever.
const FOLLOW_GUARD: Duration = Duration::from_millis(4000);
/// More broadcasts than this inside `FOLLOW_WINDOW` is not a person clicking.
const FOLLOW_BURST: usize = 6;
const FOLLOW_WINDOW: Duration = Duration::from_secs(5);

/// What to do with a URL a panel just said it is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowAction {
    /// An echo of a navigation we pushed, a redirect off the back of one, or
    /// simply where the row already is.
    Ignore,
    /// Somebody followed a link. Take every other panel there.
    Broadcast,
    /// The panels are sending each other back and forth. Stop.
    GiveUp,
}

/// Whether a link followed in one panel is followed in all of them, and the
/// bookkeeping that tells a person's click apart from the echo of the last
/// push.
///
/// This is separate from `Canvas` because it is the part that gets things
/// wrong, and a `Canvas` cannot be built without a window. Every panel we
/// navigate loads, reports in, and looks exactly like someone clicking a link,
/// so a push writes down who it is waiting to hear from and spends every
/// report until that list empties. A redirect landing inside that window is
/// absorbed the same way, which is the right answer: each panel follows the
/// redirect itself, at its own width.
pub struct Follow {
    pub on: bool,
    pending: Option<Pending>,
    /// When the recent broadcasts happened, oldest first.
    history: Vec<Instant>,
}

struct Pending {
    awaiting: HashSet<String>,
    at: Instant,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            on: true,
            pending: None,
            history: Vec::new(),
        }
    }
}

impl Follow {
    /// `href` is the URL the panel reports, `current` is where the row believes
    /// it is. `panels` is how many panels exist: one panel has nobody to tell.
    pub fn decide(
        &mut self,
        from: &str,
        href: &str,
        current: &str,
        panels: usize,
        now: Instant,
    ) -> FollowAction {
        if !self.on || panels < 2 {
            return FollowAction::Ignore;
        }

        // Still waiting on panels we pushed. Cross this one off; a report is
        // only an instruction again once the row has settled.
        if let Some(pending) = self.pending.as_mut() {
            let expired = now.duration_since(pending.at) > FOLLOW_GUARD;
            pending.awaiting.remove(from);
            if !expired {
                if pending.awaiting.is_empty() {
                    self.pending = None;
                }
                return FollowAction::Ignore;
            }
            // Past the deadline we have stopped waiting, so this report is not
            // owed to us and gets judged on its URL like any other. Spending it
            // as well would swallow the first click after a panel that never
            // reported back, which is the case the deadline exists for.
            self.pending = None;
        }

        if href == current {
            return FollowAction::Ignore;
        }

        // A site that redirects on width sends two panels chasing each other
        // forever. Nothing a person does looks like this.
        self.history
            .retain(|at| now.duration_since(*at) < FOLLOW_WINDOW);
        if self.history.len() >= FOLLOW_BURST {
            self.on = false;
            self.history.clear();
            self.pending = None;
            return FollowAction::GiveUp;
        }
        self.history.push(now);
        FollowAction::Broadcast
    }

    /// Record a navigation pushed to `awaiting`, whether that came from a
    /// followed link or from the toolbar. Either way every one of those panels
    /// is about to report in.
    pub fn pushed(&mut self, awaiting: HashSet<String>, now: Instant) {
        self.pending = Some(Pending { awaiting, at: now });
    }

    /// Following is a mode, and a mode that silently stays stale is worse than
    /// one that resets.
    pub fn reset(&mut self, on: bool) {
        self.on = on;
        self.pending = None;
        self.history.clear();
    }
}

/// A panel reported that it is showing a different URL, so take the rest of the
/// row there too.
pub fn follow_navigation(app: &AppHandle, state: &Shared, from: &str, url: &str) {
    let target = crate::util::normalize_url(url);
    let href = target.as_str().to_string();

    let action = {
        let mut canvas = state.canvas.lock().unwrap();
        let panels = canvas.panels.len();
        let current = canvas.url.clone();
        canvas
            .follow
            .decide(from, &href, &current, panels, Instant::now())
    };

    match action {
        FollowAction::Ignore => {}
        FollowAction::Broadcast => {
            {
                let mut canvas = state.canvas.lock().unwrap();
                canvas.url = href;
                let hidden = canvas.panels_hidden;
                let mut awaiting = HashSet::new();
                for panel in canvas.panels.iter_mut() {
                    if panel.viewport.id == from {
                        continue;
                    }
                    send_to(panel, &target, hidden);
                    awaiting.insert(panel.viewport.id.clone());
                }
                canvas.follow.pushed(awaiting, Instant::now());
            }
            emit_canvas(app, state);
        }
        FollowAction::GiveUp => {
            emit_canvas(app, state);
            let _ = app.emit(
                "canvas:notice",
                "Panels kept sending each other to different URLs, so following links is off.",
            );
        }
    }
}

/// Arm or disarm the element picker in every panel.
///
/// It has to be a mode rather than always on, because while it is armed a click
/// describes an element instead of following a link, and that is not a thing to
/// do to somebody by surprise.
pub fn set_picking(state: &Shared, on: bool) {
    let mut canvas = state.canvas.lock().unwrap();
    canvas.picking = on;
    let js = format!("window.__bpPick && window.__bpPick({on})");
    for panel in canvas.panels.iter() {
        let _ = panel.webview.eval(&js);
    }
}

/// A page that has just loaded has a fresh document and knows nothing about the
/// mode the app is in, so re-arm it.
pub fn rearm_picking(state: &Shared, id: &str) {
    let canvas = state.canvas.lock().unwrap();
    if !canvas.picking {
        return;
    }
    if let Some(panel) = find(&canvas, id) {
        let _ = panel.webview.eval("window.__bpPick && window.__bpPick(true)");
    }
}

pub fn set_follow(state: &Shared, on: bool) {
    state.canvas.lock().unwrap().follow.reset(on);
}

pub fn reload_all(state: &Shared) {
    let canvas = state.canvas.lock().unwrap();
    for panel in canvas.panels.iter() {
        let _ = panel.webview.eval("location.reload()");
    }
}

/// Open the Web Inspector on one panel, so its page can be picked apart at the
/// width it is actually rendering at.
///
/// This is WebKit's own inspector, in its own window, and it takes focus when it
/// opens. There is no way to ask for it unfocused.
pub fn inspect_panel(app: &AppHandle, state: &Shared, id: &str) -> Result<(), String> {
    // The handle is taken out of the lock and the guard dropped before the
    // inspector is opened.
    //
    // This command is synchronous, so it runs on the main thread, and opening
    // the inspector takes focus. AppKit can deliver the focus event inline
    // during that call, and this app's focus handler locks the same canvas
    // mutex, which is not reentrant. That is a frozen window with nothing in
    // the log, rather than a panic. Not reproduced, and it costs nothing to
    // make impossible.
    let webview = {
        let canvas = state.canvas.lock().unwrap();
        find(&canvas, id)
            .ok_or_else(|| format!("no panel called {id}"))?
            .webview
            .clone()
    };
    webview.open_devtools();

    set_inspecting(app, state, Some(id.to_string()));
    Ok(())
}

/// Enter or leave the inspect mode.
///
/// WHAT OPENING THE INSPECTOR ACTUALLY DOES
///
/// WebKit docks its inspector into the webview it is inspecting and takes that
/// webview's frame for itself: the page ends up at the window's full width,
/// with the inspector below it. On a 5120px display that means a 1024px panel
/// renders its page at 5120px. It covers the chrome and every panel to its
/// right, and the width on its label becomes a lie, which for this app is the
/// worst failure available.
///
/// TWO THINGS, UNDONE TOGETHER
///
/// So the mode does two things. It hides every other panel, because the docked
/// inspector has taken the window and a row of panels behind it is noise you
/// cannot see anyway. And it keeps re-asserting the inspected panel's frame,
/// because WebKit re-takes it: setting the size back does win and does stick,
/// but only until the inspector next decides to lay itself out, so a one-shot
/// restore loses a race it cannot see.
///
/// WHY LEAVING IS MANUAL
///
/// There is no way to be told the inspector has closed. WebKit offers no event
/// and Tauri surfaces none, so nothing can notice it and put the row back.
/// Hence a visible control in the chrome, and pressing Inspect again on the
/// same panel toggles out. Guessing from window focus was the alternative and
/// it is worse: you click between the inspector and the page constantly while
/// using it, and each of those would rebuild the row underneath you.
pub fn set_inspecting(app: &AppHandle, state: &Shared, id: Option<String>) {
    let mut show_or_hide: Vec<(tauri::Webview<Wry>, bool)> = Vec::new();
    {
        let mut canvas = state.canvas.lock().unwrap();
        if canvas.inspecting == id {
            return;
        }
        canvas.inspecting = id.clone();

        // A sheet already hides everything; leave it alone and let closing it
        // sort the row out, or the two would fight over the same webviews.
        if !canvas.panels_hidden {
            // Collected under the lock, acted on after it. Showing or hiding a
            // webview is a call into AppKit, and this module's own rule is
            // never to hold a lock across one.
            for panel in canvas.panels.iter() {
                let failed = matches!(panel.state, PanelState::Failed(_));
                let wanted = match &id {
                    Some(only) => &panel.viewport.id == only,
                    None => true,
                };
                show_or_hide.push((panel.webview.clone(), wanted && !failed));
            }
        }
    }

    for (webview, show) in show_or_hide {
        let _ = if show { webview.show() } else { webview.hide() };
    }

    if id.is_none() {
        relayout(app, state);
        emit_canvas(app, state);
        return;
    }

    // Hold the panel at its declared size for as long as the mode lasts. The
    // task ends itself when the mode does, so entering the mode twice cannot
    // leave two of them running.
    let watch_state = state.clone();
    let watched = id.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let still = watch_state.canvas.lock().unwrap().inspecting.clone();
            if still != watched {
                break;
            }
            restore_frames(&watch_state);
        }
    });

    emit_canvas(app, state);
}

/// Put every panel back at the position, size and zoom Rust already believes it
/// has.
///
/// This recomputes nothing. `relayout` is for when the numbers themselves
/// change; this is for when something outside the app has moved a webview
/// behind our back, which in practice means the Web Inspector.
pub fn restore_frames(state: &Shared) {
    let canvas = state.canvas.lock().unwrap();
    // A hidden panel is hidden because a sheet is open. Re-asserting its frame
    // would put it back over the sheet.
    if canvas.panels_hidden {
        return;
    }
    let scroll_x = canvas.scroll_x;
    for panel in canvas.panels.iter() {
        // While one panel is being inspected the others are hidden, and putting
        // a hidden webview back at its place would show it again.
        if let Some(only) = &canvas.inspecting {
            if &panel.viewport.id != only {
                continue;
            }
        }
        let _ = panel.webview.set_zoom(panel.scale);
        let _ = panel
            .webview
            .set_position(LogicalPosition::new(panel.home_x - scroll_x, PANEL_TOP));
        let _ = panel
            .webview
            .set_size(LogicalSize::new(panel.width, panel.height));
    }
}

pub fn reload_panel(state: &Shared, id: &str) {
    let canvas = state.canvas.lock().unwrap();
    if let Some(panel) = find(&canvas, id) {
        let _ = panel.webview.eval("location.reload()");
    }
}

/// One panel scrolled, so move the others to match.
///
/// The position is a percentage of scrollable height, not a pixel offset,
/// because the same page is a different height at 640 wide than at 1536.
/// The drain, wrapped once and kept.
///
/// It never varies, and the pump sends it to every visible panel on every
/// tick, so building the wrapper each time was a string allocation several
/// hundred times a second for no gain.
fn drain_script() -> &'static str {
    static SCRIPT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SCRIPT.get_or_init(|| {
        crate::tools::wrap_script("return window.__bpDrain ? window.__bpDrain() : [];")
    })
}

/// A panel whose position we cannot vouch for. Out of the 0 to 1 range, so it
/// never matches a target and the next sync always tells it.
const UNKNOWN_SCROLL: f64 = -1.0;

/// How far a panel has to be from the target before it is worth telling.
///
/// A proportion of the page, not pixels, because that is what is being synced.
/// A thousandth of a long page is a pixel or two, which nobody can see.
const SYNC_EPSILON: f64 = 0.001;

/// Forget where every panel is, so the next sync tells all of them.
pub fn forget_scroll_positions(canvas: &mut Canvas) {
    for panel in canvas.panels.iter_mut() {
        panel.last_scroll_pct = UNKNOWN_SCROLL;
    }
}

pub fn sync_scroll(state: &Shared, from: &str, pct: f64) {
    let mut canvas = state.canvas.lock().unwrap();
    if !canvas.sync_on {
        return;
    }
    let target = clamp01(pct);
    let js = format!("window.__bpApplyScroll && window.__bpApplyScroll({target})");

    for panel in canvas.panels.iter_mut() {
        if panel.viewport.id == from {
            // Where the panel doing the scrolling is, so that when it is the
            // one being told later, we know whether it needs telling.
            panel.last_scroll_pct = target;
            continue;
        }
        // Panels converge during a steady scroll, and once they have arrived
        // there is nothing to say. This used to push into every other panel on
        // every message, which for seven panels is about 42 evaluations per
        // throttle window for the whole length of a scroll.
        if (panel.last_scroll_pct - target).abs() < SYNC_EPSILON {
            continue;
        }
        panel.last_scroll_pct = target;
        let _ = panel.webview.eval(&js);
    }
}

/// Sheets are chrome, and chrome composites below the panels, so a sheet can
/// only be seen if the panels get out of the way.
pub fn set_panels_hidden(app: &AppHandle, state: &Shared, hidden: bool) {
    let mut show_or_hide: Vec<(tauri::Webview<Wry>, bool)> = Vec::new();
    {
        let mut canvas = state.canvas.lock().unwrap();
        if canvas.panels_hidden == hidden {
            return;
        }
        canvas.panels_hidden = hidden;
        for panel in canvas.panels.iter() {
            let failed = matches!(panel.state, PanelState::Failed(_));
            show_or_hide.push((panel.webview.clone(), !(hidden || failed)));
        }
    }

    for (webview, show) in show_or_hide {
        let _ = if show { webview.show() } else { webview.hide() };
    }
    if !hidden {
        relayout(app, state);
    }
}

/// Whether a finished load actually landed anywhere.
///
/// A navigation that fails at the network level, a refused connection, a name
/// that does not resolve, a certificate WebKit will not trust, never commits.
/// The panel is left showing `about:blank` and the delegate reports the load
/// finished, so the label said "loaded" over a blank panel and the failed
/// state, which exists precisely to draw the failure at the panel's exact
/// size, was never reached.
///
/// Measured rather than assumed: pointing a row at a closed port puts every
/// panel on `about:blank` with the state set to loaded.
///
/// `requested` is where the row or the panel was sent. A panel genuinely asked
/// for `about:blank`, which is the empty state at first launch, has not
/// failed.
pub fn load_failed(document: &str, requested: &str) -> bool {
    let blank = document.is_empty() || document == "about:blank";
    blank && !requested.is_empty() && requested != "about:blank"
}

/// Give a blank panel a moment, then call it failed if it is still blank.
///
/// A real navigation can pass through a blank document on its way somewhere,
/// so marking a panel failed the instant it reports one would hide it and then
/// show it again, a flicker for every ordinary page load. Waiting and looking
/// again costs nothing, because a panel that has genuinely failed stays blank.
///
/// The row's URL is checked again too: if it has moved on, this panel's blank
/// document belongs to a navigation nobody is waiting for any more.
fn confirm_load_failure(app: AppHandle, state: Shared, id: String, requested: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
        let still_blank = {
            let canvas = state.canvas.lock().unwrap();
            if canvas.url != requested {
                return;
            }
            match canvas.panels.iter().find(|p| p.viewport.id == id) {
                Some(panel) => load_failed(&panel.document_url, &requested),
                None => return,
            }
        };
        if still_blank {
            set_panel_state(&app, &state, &id, PanelState::Failed(0));
        }
    });
}

/// Why a panel cannot be measured right now, or `None` if it can.
///
/// Panels are child webviews composited over the chrome, so the only way to
/// draw a sheet is to hide them, and a window capture at a hidden panel's
/// rectangle returns whatever chrome is painted there instead. That came back
/// as a valid PNG of the sheet, reported as a success, which for a measuring
/// instrument is worse than an error. Inspecting has the same effect on every
/// panel except the one being inspected.
///
/// Pass the panel being measured, or `None` when the whole row is.
pub fn cannot_measure(state: &Shared, id: Option<&str>) -> Option<String> {
    let canvas = state.canvas.lock().unwrap();
    if canvas.panels_hidden {
        return Some(
            "the panels are hidden behind a sheet, so there is nothing to measure. Close it first."
                .to_string(),
        );
    }
    let inspecting = canvas.inspecting.as_deref()?;
    // Every other panel is hidden behind the docked inspector.
    if id == Some(inspecting) {
        return None;
    }
    let name = canvas
        .panels
        .iter()
        .find(|p| p.viewport.id == inspecting)
        .map(|p| p.viewport.name.clone())
        .unwrap_or_else(|| inspecting.to_string());
    Some(format!(
        "the row is hidden while {name} is being inspected, so there is nothing to measure. Leave inspect mode first."
    ))
}

/// Whether a panel's own document has to have its messages collected.
///
/// An http page posts to the loopback callback server directly, so its queue
/// is always empty and asking it is pure cost. WebKit blocks that request from
/// an https document and does not exempt loopback, so an https page queues
/// instead and something has to come and take the queue.
pub fn needs_pump(document_url: &str) -> bool {
    document_url.starts_with("https:")
}

/// Remember which document a panel is showing.
pub fn set_document_url(state: &Shared, id: &str, url: &str) {
    let mut canvas = state.canvas.lock().unwrap();
    if let Some(panel) = canvas.panels.iter_mut().find(|p| p.viewport.id == id) {
        if panel.document_url != url {
            panel.document_url = url.to_string();
        }
        // A new document starts at the top, so whatever we last told this
        // panel about where it should be is no longer true of it. Out of
        // range, so the next sync always tells it.
        panel.last_scroll_pct = UNKNOWN_SCROLL;
    }
}

/// Record that a panel has committed a navigation, so the pump may ask it
/// questions.
///
/// Until a webview commits, wry queues every `eval` as a bare string and
/// throws the callback away, so asking is worse than not asking: the drain
/// runs later, empties the page's queue and hands the contents to nobody.
///
/// Two things used to go wrong here. Only the `Started` branch set the flag, so
/// a load that went straight to `Finished` never set it at all. And the lookup
/// races the push in `spawn`, so a report arriving first found no panel and did
/// nothing, leaving that panel permanently unasked. Both are why a panel could
/// silently stop reporting anything for a whole session, which reads as the
/// Report box being broken.
pub fn mark_committed(state: &Shared, id: &str) {
    {
        let mut canvas = state.canvas.lock().unwrap();
        if let Some(panel) = canvas.panels.iter_mut().find(|p| p.viewport.id == id) {
            panel.ever_committed = true;
            return;
        }
    }
    state.committed_early.lock().unwrap().insert(id.to_string());
}

/// How far the page may be from its label before it counts as wrong.
///
/// Half a pixel, so a whole pixel out is a finding. That is deliberate and it
/// is the tightest useful value: the frame is whole pixels and the viewport is
/// the frame divided by the zoom, so `layout` derives the zoom back from the
/// rounded frame to make the two agree exactly. When they do not, one pixel is
/// enough to matter, because a page at 767 does not fire a `min-width: 768px`
/// query and looks for all the world like the query is broken. That exact bug
/// is why the layout arithmetic is the way it is, and this is the check that
/// would have caught it.
const WIDTH_TOLERANCE: f64 = 0.5;

/// Whether a page's own idea of its width disagrees with the declared one.
///
/// A panel that has not reported yet is not a disagreement. Silence and a
/// wrong answer are different things, and treating the first as the second
/// would light up every panel for the moment between spawning and loading.
pub fn width_disagrees(declared: f64, reported: Option<f64>) -> bool {
    match reported {
        Some(reported) => (declared - reported).abs() > WIDTH_TOLERANCE,
        None => false,
    }
}

/// Record what a page says its viewport is, and say so if it is wrong.
///
/// Called on every load report. The first report can arrive before the zoom
/// has been applied, so a disagreement is confirmed by measuring again rather
/// than believed at once: see `confirm_width`.
pub fn set_reported_width(app: &AppHandle, state: &Shared, id: &str, reported: f64) -> bool {
    let disagrees = {
        let mut canvas = state.canvas.lock().unwrap();
        let Some(panel) = canvas.panels.iter_mut().find(|p| p.viewport.id == id) else {
            return false;
        };
        if panel.reported_width == Some(reported) {
            return width_disagrees(panel.viewport.width, panel.reported_width);
        }
        panel.reported_width = Some(reported);
        width_disagrees(panel.viewport.width, Some(reported))
    };
    emit_canvas(app, state);
    disagrees
}

/// Measure a panel's width again, a moment later, and keep the answer.
///
/// A first load can genuinely report the wrong width and then be right: the
/// webview is created at its scaled size and starts loading immediately, and
/// the zoom is a separate message that lands afterwards. Believing that first
/// number would mark every panel in a zoomed row as wrong and never clear it.
/// So a disagreement is checked once more before it is trusted.
pub fn confirm_width(app: AppHandle, state: Shared, id: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let Ok(value) = crate::tools::eval_js(&state, &id, "return window.innerWidth;").await
        else {
            return;
        };
        if let Some(width) = value.as_f64() {
            set_reported_width(&app, &state, &id, width);
        }
    });
}

pub fn set_panel_state(app: &AppHandle, state: &Shared, id: &str, new_state: PanelState) {
    {
        let mut canvas = state.canvas.lock().unwrap();
        let sheet_open = canvas.panels_hidden;
        let Some(panel) = canvas.panels.iter_mut().find(|p| p.viewport.id == id) else {
            return;
        };
        if panel.state == new_state {
            return;
        }
        // A panel that failed shows WebKit's own error page, which says nothing
        // useful and loses the one piece of information that matters: the size.
        // So it gets out of the way and the chrome draws the failed state at
        // the panel's exact dimensions instead.
        let failed = matches!(new_state, PanelState::Failed(_));
        panel.state = new_state;
        if !sheet_open {
            let _ = if failed {
                panel.webview.hide()
            } else {
                panel.webview.show()
            };
        }
    }
    emit_canvas(app, state);
}

/// Mark every panel with the same state. A dev URL that is not answering fails
/// all of them at once, which is what actually happens.
pub fn set_all_panel_states(app: &AppHandle, state: &Shared, new_state: PanelState) {
    {
        let mut canvas = state.canvas.lock().unwrap();
        let sheet_open = canvas.panels_hidden;
        let failed = matches!(new_state, PanelState::Failed(_));
        for panel in canvas.panels.iter_mut() {
            panel.state = new_state.clone();
            if !sheet_open {
                let _ = if failed { panel.webview.hide() } else { panel.webview.show() };
            }
        }
    }
    emit_canvas(app, state);
}

pub fn info(state: &Shared) -> CanvasInfo {
    let canvas = state.canvas.lock().unwrap();
    CanvasInfo {
        panels: canvas
            .panels
            .iter()
            .enumerate()
            .map(|(at, p)| PanelInfo {
                id: p.viewport.id.clone(),
                name: p.viewport.name.clone(),
                source: p.viewport.source.clone(),
                position: at + 1,
                width: p.viewport.width,
                height: p.viewport.height,
                scale: p.scale,
                home_x: p.home_x,
                on_screen_width: p.width,
                on_screen_height: p.height,
                state: p.state.clone(),
                console_errors: state
                    .console
                    .lock()
                    .unwrap()
                    .get(&p.viewport.id)
                    .map(|lines| lines.iter().filter(|l| l.level == "error").count())
                    .unwrap_or(0),
                document_url: p.document_url.clone(),
                reported_width: p.reported_width,
                width_mismatch: width_disagrees(p.viewport.width, p.reported_width),
            })
            .collect(),
        total_width: canvas.total_width,
        scroll_x: canvas.scroll_x,
        panel_top: PANEL_TOP,
        url: canvas.url.clone(),
        zoom_to_fit: canvas.zoom_to_fit,
        scroll_sync: canvas.sync_on,
        follow_links: canvas.follow.on,
        picking: canvas.picking,
        full_height: canvas.full_height,
        inspecting: canvas.inspecting.clone(),
    }
}

pub fn emit_canvas(app: &AppHandle, state: &Shared) {
    let _ = app.emit("canvas:layout", info(state));
}

/// Resolve a panel by index or by name, case-insensitively and by prefix, so
/// "fix it at md" and "fix the overlap in window 4" both land.
pub fn resolve_id(state: &Shared, needle: &str) -> Option<String> {
    let canvas = state.canvas.lock().unwrap();
    let viewports: Vec<Viewport> = canvas.panels.iter().map(|p| p.viewport.clone()).collect();
    match_viewport(&viewports, needle).map(|at| viewports[at].id.clone())
}

/// Which viewport does this name mean?
///
/// Separate from `resolve_id` so it can be tested: a panel owns a live webview
/// and cannot be built without a window, but the matching is the part that
/// gets things wrong.
pub fn match_viewport(viewports: &[Viewport], needle: &str) -> Option<usize> {
    let needle = needle.trim();
    // Nothing is not a panel. An empty name used to fall through the matching
    // and land on the first panel, so a caller that forgot the argument got a
    // confident answer about a panel it had not asked for.
    if needle.is_empty() {
        return None;
    }
    if let Ok(number) = needle.parse::<usize>() {
        if number < viewports.len() {
            return Some(number);
        }
        // Not a valid index. In an app whose entire vocabulary is widths, a
        // bare number is far more likely to be one, and "1024" is the obvious
        // way to name the 1024 panel. Returning nothing for it was a dead end
        // with no hint about what to type instead.
        return viewports.iter().position(|v| v.width as usize == number);
    }

    let lower = needle.to_ascii_lowercase();
    let exact = viewports.iter().position(|v| {
        v.id.eq_ignore_ascii_case(&lower)
            || v.name.eq_ignore_ascii_case(&lower)
            || v.source.eq_ignore_ascii_case(&lower)
    });
    if exact.is_some() {
        return exact;
    }
    viewports.iter().position(|v| {
        v.name.to_ascii_lowercase().starts_with(&lower)
            || v.id.to_ascii_lowercase().starts_with(&lower)
    })
}

fn find<'a>(canvas: &'a Canvas, id: &str) -> Option<&'a Panel> {
    canvas.panels.iter().find(|p| p.viewport.id == id)
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vps() -> Vec<Viewport> {
        vec![
            Viewport::new("Mobile", 640.0, 850.0, "sm"),
            Viewport::new("Tablet", 768.0, 1020.0, "md"),
        ]
    }

    fn row() -> Vec<Viewport> {
        vec![
            Viewport::new("Mobile", 375.0, 810.0, "2xsm"),
            Viewport::new("Tablet", 768.0, 1020.0, "md"),
            Viewport::new("Laptop", 1024.0, 770.0, "lg"),
        ]
    }

    /// The script injected into every panel is JavaScript inside a Rust
    /// `format!` template, so nothing checks it: a stray brace or a bad
    /// substitution ships and then fails silently inside a page we do not own,
    /// taking scroll sync, console capture and the ready report with it.
    #[test]
    fn the_injected_script_is_valid_javascript() {
        let endpoint = Endpoint {
            port: 50309,
            nonce: "test-nonce".into(),
        };
        let script = injected_script("tablet-768", &endpoint);
        crate::util::tests_support::assert_js_parses(&script, "injected panel script");
    }

    #[test]
    fn the_injected_script_carries_the_panel_id_and_the_endpoint() {
        let endpoint = Endpoint {
            port: 50309,
            nonce: "test-nonce".into(),
        };
        let script = injected_script("tablet-768", &endpoint);
        assert!(script.contains(r#""tablet-768""#), "the panel names itself");
        assert!(script.contains("http://127.0.0.1:50309"), "knows where to post");
        assert!(script.contains(r#""test-nonce""#), "carries the nonce");
        assert!(
            script.contains("window.__bpDrain"),
            "an https panel has to be drainable, since it cannot post"
        );
    }

    /// The row: four panels, sitting on the same URL, nothing pending.
    fn settled_row() -> Follow {
        Follow::default()
    }

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_link_followed_in_one_panel_is_broadcast_to_the_rest() {
        let mut follow = settled_row();
        assert_eq!(
            follow.decide("a", "https://s.test/about", "https://s.test/", 4, Instant::now()),
            FollowAction::Broadcast
        );
    }

    #[test]
    fn a_panel_reporting_the_url_the_row_is_already_on_is_not_a_navigation() {
        let mut follow = settled_row();
        assert_eq!(
            follow.decide("a", "https://s.test/", "https://s.test/", 4, Instant::now()),
            FollowAction::Ignore
        );
    }

    /// A load that never committed leaves the panel blank and still reports
    /// finished. Pointing a row at a closed port is enough to see it: every
    /// panel ends on about:blank with its label reading "loaded".
    #[test]
    fn a_load_that_landed_nowhere_is_a_failure() {
        assert!(load_failed("about:blank", "https://example.test/"));
        assert!(load_failed("", "https://example.test/"));
        assert!(!load_failed("https://example.test/", "https://example.test/"));
        assert!(
            !load_failed("about:blank", "about:blank"),
            "the empty state at first launch asked for exactly this"
        );
        assert!(
            !load_failed("about:blank", ""),
            "nothing was requested, so nothing failed"
        );
    }

    /// A page that logs on a timer used to hold the pump at its fastest rate
    /// for the whole session, because anything in the queue counted as
    /// activity.
    #[test]
    fn only_a_gesture_keeps_the_pump_fast() {
        let msg = |path: &str| serde_json::json!({ "path": path, "body": {} });
        assert!(is_gesture(&msg("/p/scroll")));
        assert!(is_gesture(&msg("/p/wheel")));
        assert!(!is_gesture(&msg("/p/console")));
        assert!(!is_gesture(&msg("/p/ready")));
        assert!(!is_gesture(&msg("/p/report")));
        assert!(!is_gesture(&msg("/p/nav")));
        assert!(!is_gesture(&serde_json::json!({})));
    }

    /// An empty name used to land on the first panel, so a caller that forgot
    /// the argument got a confident answer about a panel it never asked for.
    #[test]
    fn nothing_is_not_a_panel() {
        let viewports = vec![
            Viewport::new("Small", 375.0, 667.0, "custom"),
            Viewport::new("Medium", 768.0, 1020.0, "medium"),
        ];
        assert_eq!(match_viewport(&viewports, ""), None);
        assert_eq!(match_viewport(&viewports, "   "), None);
        assert_eq!(match_viewport(&viewports, "0"), Some(0));
        assert_eq!(match_viewport(&viewports, "Medium"), Some(1));
    }

    /// The row is routinely four times the window's width, so most of what the
    /// pump used to do was asking panels nobody could see for a scroll
    /// position nobody could have changed.
    #[test]
    fn only_the_panels_on_screen_are_collected_from() {
        // A 1400px window over the real Bucknell row.
        let w = 1400.0;
        assert!(panel_on_screen(24.0, 375.0, 0.0, w), "first panel, row at rest");
        assert!(panel_on_screen(997.0, 640.0, 0.0, w), "third panel, partly visible");
        assert!(!panel_on_screen(4805.0, 1536.0, 0.0, w), "last panel, far off to the right");
        // Panned to the far end, the first panel is now the hidden one.
        assert!(!panel_on_screen(24.0, 375.0, 4800.0, w));
        assert!(panel_on_screen(4805.0, 1536.0, 4800.0, w));
        // Just off the left edge, and still collected from, because it is
        // about to be on screen.
        assert!(panel_on_screen(0.0, 300.0, 400.0, w));
        assert!(
            panel_on_screen(4805.0, 1536.0, 0.0, 0.0),
            "with no window width known, collect from everything"
        );
    }

    /// Whether a panel's messages have to be collected is a fact about that
    /// panel's own document. Reading it off the row meant a row opened on http
    /// that redirected to https had every panel queueing into a void, with
    /// scroll sync, console capture and problem reports all dead at once.
    #[test]
    fn collecting_is_decided_per_panel_not_per_row() {
        assert!(needs_pump("https://bucknell-be-the-ray.lndo.site/"));
        assert!(!needs_pump("http://localhost:1420/"));
        assert!(!needs_pump(""), "a panel that has not loaded has nothing to collect");
        assert!(!needs_pump("about:blank"));
    }

    /// A load reported before its panel was added to the row used to be
    /// dropped on the floor, and that panel was then never asked for anything
    /// again: no scroll sync, no console capture, no problem reports, for the
    /// whole session.
    #[test]
    fn a_commit_reported_before_its_panel_exists_is_kept() {
        let state: Shared = std::sync::Arc::new(crate::state::AppState::default());
        mark_committed(&state, "medium-768");
        assert!(
            state.committed_early.lock().unwrap().contains("medium-768"),
            "the report has to survive until the panel is pushed"
        );
    }

    /// A page that agrees with its label is not a finding, and a page that
    /// has not spoken yet is not one either. Only a real disagreement is.
    #[test]
    fn a_panel_is_only_wrong_when_the_page_says_so() {
        assert!(!width_disagrees(768.0, None), "silence is not a disagreement");
        assert!(!width_disagrees(768.0, Some(768.0)));
        assert!(
            !width_disagrees(768.0, Some(767.5)),
            "half a pixel is the frame rounding, not a bug"
        );
        assert!(width_disagrees(768.0, Some(767.0)), "a whole pixel out is wrong");
        assert!(
            width_disagrees(1024.0, Some(5120.0)),
            "a docked inspector gives the panel the whole window"
        );
        assert!(
            width_disagrees(1024.0, Some(512.0)),
            "a layout that happened before the zoom landed"
        );
    }

    /// The one that matters. Every panel we push loads and reports back, and
    /// each of those reports looks exactly like somebody clicking a link. If
    /// they were taken at face value the row would navigate itself forever.
    #[test]
    fn the_echo_of_a_push_is_spent_and_the_row_is_free_again_after_the_last_one() {
        let mut follow = settled_row();
        let now = Instant::now();
        assert_eq!(
            follow.decide("a", "https://s.test/about", "https://s.test/", 4, now),
            FollowAction::Broadcast
        );
        follow.pushed(ids(&["b", "c", "d"]), now);

        // The three panels we pushed report in on the new URL.
        for id in ["b", "c"] {
            assert_eq!(
                follow.decide(id, "https://s.test/about", "https://s.test/about", 4, now),
                FollowAction::Ignore
            );
        }
        assert_eq!(
            follow.decide("d", "https://s.test/about", "https://s.test/about", 4, now),
            FollowAction::Ignore
        );

        // The last one settled the row, so the next click is followed at once
        // rather than waiting out a timer.
        assert_eq!(
            follow.decide("a", "https://s.test/team", "https://s.test/about", 4, now),
            FollowAction::Broadcast
        );
    }

    /// A panel that failed to load never reports, so the list it is on never
    /// empties. Without the deadline, following would be wedged for the rest
    /// of the session.
    /// A panel that failed to load never reports, so the list it is on never
    /// empties. The deadline is what stops that wedging following for the rest
    /// of the session, and the report that trips it has to be acted on rather
    /// than spent: otherwise the first click after a panel goes quiet is
    /// silently swallowed.
    #[test]
    fn a_click_after_a_panel_goes_quiet_is_still_followed() {
        let mut follow = settled_row();
        let pushed_at = Instant::now();
        follow.pushed(ids(&["b", "c"]), pushed_at);

        let later = pushed_at + FOLLOW_GUARD + Duration::from_millis(1);
        assert_eq!(
            follow.decide("b", "https://s.test/team", "https://s.test/about", 4, later),
            FollowAction::Broadcast
        );
    }

    /// The same deadline, but the late report is only the echo we were waiting
    /// for. The URL is what tells them apart.
    #[test]
    fn a_late_echo_is_still_not_a_navigation() {
        let mut follow = settled_row();
        let pushed_at = Instant::now();
        follow.pushed(ids(&["b", "c"]), pushed_at);

        let later = pushed_at + FOLLOW_GUARD + Duration::from_millis(1);
        assert_eq!(
            follow.decide("b", "https://s.test/about", "https://s.test/about", 4, later),
            FollowAction::Ignore
        );
    }

    /// A site that redirects on width puts two panels on different URLs and
    /// each one keeps sending the other back. Nothing a person does looks like
    /// this, so it turns following off rather than hammering their dev server.
    #[test]
    fn panels_chasing_each_other_give_up_rather_than_loop() {
        let mut follow = settled_row();
        let now = Instant::now();
        let mut current = "https://s.test/".to_string();
        let mut broadcasts = 0;
        for turn in 0..20 {
            let href = if turn % 2 == 0 {
                "https://s.test/m/x"
            } else {
                "https://s.test/x"
            };
            match follow.decide("a", href, &current, 4, now) {
                FollowAction::Broadcast => {
                    broadcasts += 1;
                    current = href.to_string();
                }
                FollowAction::GiveUp => {
                    assert!(!follow.on, "following turns itself off");
                    assert_eq!(broadcasts, FOLLOW_BURST, "it stops at the burst limit");
                    return;
                }
                FollowAction::Ignore => {}
            }
        }
        panic!("the ping pong was never noticed");
    }

    #[test]
    fn following_off_means_a_report_is_only_a_report() {
        let mut follow = settled_row();
        follow.reset(false);
        assert_eq!(
            follow.decide("a", "https://s.test/about", "https://s.test/", 4, Instant::now()),
            FollowAction::Ignore
        );
    }

    /// One panel has nobody to tell, and pushing to an empty row would still
    /// arm the guard and leave it stale.
    #[test]
    fn a_single_panel_never_broadcasts() {
        let mut follow = settled_row();
        assert_eq!(
            follow.decide("a", "https://s.test/about", "https://s.test/", 1, Instant::now()),
            FollowAction::Ignore
        );
    }

    #[test]
    fn a_single_page_app_changing_the_path_is_reported_but_a_hash_is_not() {
        let endpoint = Endpoint {
            port: 50309,
            nonce: "test-nonce".into(),
        };
        let script = injected_script("md-768", &endpoint);
        assert!(script.contains("pushState"), "the history API is watched");
        assert!(script.contains("popstate"), "so is the back button");
        assert!(
            script.contains("location.origin + location.pathname + location.search"),
            "the fragment is left out, so an anchor link does not reload the row"
        );
    }

    #[test]
    fn a_panel_can_be_reached_by_index() {
        assert_eq!(match_viewport(&row(), "0"), Some(0));
        assert_eq!(match_viewport(&row(), "2"), Some(2));
    }

    #[test]
    fn a_number_that_is_not_an_index_is_read_as_a_width() {
        // "1024" is the obvious way to name the 1024 panel, and it used to
        // resolve to nothing at all: the parse succeeded, the index lookup
        // failed, and the name matching never got a turn.
        assert_eq!(match_viewport(&row(), "1024"), Some(2));
        assert_eq!(match_viewport(&row(), "768"), Some(1));
    }

    #[test]
    fn an_index_wins_over_a_width_when_both_could_match() {
        // Small numbers are indices first. Nobody has a 2px breakpoint.
        let viewports = vec![
            Viewport::new("A", 1.0, 800.0, "a"),
            Viewport::new("B", 0.0, 800.0, "b"),
        ];
        assert_eq!(match_viewport(&viewports, "0"), Some(0), "index, not width");
    }

    #[test]
    fn a_number_matching_neither_finds_nothing() {
        assert_eq!(match_viewport(&row(), "999"), None);
    }

    #[test]
    fn a_panel_can_be_reached_by_name_key_or_id_whatever_the_case() {
        assert_eq!(match_viewport(&row(), "Tablet"), Some(1));
        assert_eq!(match_viewport(&row(), "tablet"), Some(1));
        assert_eq!(match_viewport(&row(), "md"), Some(1));
        assert_eq!(match_viewport(&row(), "MD"), Some(1));
        assert_eq!(match_viewport(&row(), &row()[2].id), Some(2));
    }

    #[test]
    fn a_prefix_is_enough_and_surrounding_space_is_ignored() {
        assert_eq!(match_viewport(&row(), "Lap"), Some(2));
        assert_eq!(match_viewport(&row(), "  Tablet  "), Some(1));
    }

    #[test]
    fn an_exact_match_beats_a_prefix_of_an_earlier_panel() {
        let viewports = vec![
            Viewport::new("Tablet Landscape", 1024.0, 770.0, "lg"),
            Viewport::new("Tablet", 768.0, 1020.0, "md"),
        ];
        assert_eq!(match_viewport(&viewports, "Tablet"), Some(1));
    }

    #[test]
    fn nothing_matches_in_an_empty_row() {
        assert_eq!(match_viewport(&[], "0"), None);
        assert_eq!(match_viewport(&[], "Tablet"), None);
    }

    #[test]
    fn panning_stops_at_both_ends_of_the_row() {
        // A wheel gesture keeps arriving after you reach the end, so a delta
        // that would run past either edge has to land on the edge.
        assert_eq!(clamped_scroll(0.0, -500.0, 3000.0, 1200.0), 0.0);
        assert_eq!(clamped_scroll(1700.0, 500.0, 3000.0, 1200.0), 1800.0);
        assert_eq!(clamped_scroll(100.0, 200.0, 3000.0, 1200.0), 300.0);
    }

    #[test]
    fn a_row_narrower_than_the_window_does_not_pan() {
        assert_eq!(clamped_scroll(0.0, 400.0, 800.0, 1200.0), 0.0);
    }

    #[test]
    fn unzoomed_panels_are_declared_size_and_start_at_the_margin() {
        let (places, total) = layout(&vps(), 500.0, false, FitMode::Height, false);
        assert_eq!(places[0].home_x, OUTER_MARGIN);
        assert_eq!(places[0].width, 640.0);
        assert_eq!(places[0].scale, 1.0);
        assert_eq!(places[1].home_x, OUTER_MARGIN + 640.0 + PANEL_GAP);
        assert_eq!(total, OUTER_MARGIN * 2.0 + 640.0 + 768.0 + PANEL_GAP);
    }

    #[test]
    fn fit_height_scales_each_panel_separately() {
        let (places, _) = layout(&vps(), 510.0, true, FitMode::Height, false);
        assert!((places[0].scale - 0.6).abs() < 1e-9);
        assert!((places[1].scale - 0.5).abs() < 1e-9);
    }

    #[test]
    fn uniform_fit_uses_the_smallest_scale_for_every_panel() {
        let (places, _) = layout(&vps(), 510.0, true, FitMode::Uniform, false);
        // Rounding the frame to whole pixels can move a scale by a fraction of
        // a percent; the promise is that they are the same factor, not that
        // they are bit-identical.
        assert!((places[0].scale - places[1].scale).abs() < 0.002);
        assert!((places[0].scale - 0.5).abs() < 0.002);
    }

    #[test]
    fn a_scaled_panel_still_renders_at_exactly_its_declared_width() {
        // The CSS viewport a page sees is the frame width divided by the zoom.
        // At any scale, that has to come back to the breakpoint itself, or a
        // min-width query fires one pixel late and the app is lying.
        for height in [377.0, 512.0, 640.0, 719.0, 863.0] {
            let (places, _) = layout(&vps(), height, true, FitMode::Height, false);
            for (place, vp) in places.iter().zip(vps()) {
                let rendered = place.width / place.scale;
                assert!(
                    (rendered - vp.width).abs() < 0.001,
                    "{} at avail {height} rendered {rendered}, expected {}",
                    vp.name,
                    vp.width
                );
            }
        }
    }

    #[test]
    fn zoom_never_magnifies_a_panel_past_its_true_size() {
        let (places, _) = layout(&vps(), 4000.0, true, FitMode::Height, false);
        assert_eq!(places[0].scale, 1.0);
    }
}
