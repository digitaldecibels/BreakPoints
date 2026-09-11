//! `audit_all`: the tedious part of "test my project at every viewport and fix
//! what's obviously broken", done once and returned as one structured report.
//!
//! A screenshot alone tells an agent that something looks off without saying
//! what. These probes measure instead, so a finding reads "Laptop 1024:
//! .card-grid right edge at 1043, overflows by 19px" rather than a picture and
//! a hunch.

use serde_json::{json, Value};
use tauri::AppHandle;

use crate::state::Shared;
use crate::tools;

/// Run in every panel. Returns findings, never throws: a probe that cannot run
/// should cost one finding, not the whole audit.
const PROBES: &str = r##"
var out = [];
var W = window.innerWidth;
var H = window.innerHeight;

function selectorFor(el) {
  if (!el || el === document.documentElement) return "html";
  if (el.id) return "#" + el.id;
  var part = el.tagName.toLowerCase();
  if (el.classList && el.classList.length) {
    part += "." + Array.prototype.slice.call(el.classList, 0, 2).join(".");
  }
  var parent = el.parentElement;
  if (parent) {
    var same = Array.prototype.filter.call(parent.children, function (c) {
      return c.tagName === el.tagName;
    });
    if (same.length > 1) part += ":nth-of-type(" + (same.indexOf(el) + 1) + ")";
    if (parent !== document.body && parent !== document.documentElement) {
      var up = parent.id ? "#" + parent.id : parent.tagName.toLowerCase();
      return up + " > " + part;
    }
  }
  return part;
}

// Is this element inside an <svg>? Paths in an icon overlap each other by
// design, and on one real page every single "high" finding was one path
// crossing the next inside the same icon. Nothing inside an svg is a layout
// decision, so nothing inside one is a layout bug.
function inSvg(el) {
  for (var n = el; n; n = n.parentElement) {
    if (n.tagName && n.tagName.toLowerCase() === "svg") return true;
  }
  return false;
}

// Is this hidden from a person, whatever its box says?
//
// The visually-hidden idiom gives a skip link a real 40 x 44 box inside a 1px
// clipped parent. Measuring the box alone reported it as a tap target too
// small to hit, which is advice about something nobody can see.
function unseen(el) {
  for (var n = el; n && n !== document.documentElement; n = n.parentElement) {
    var cs = getComputedStyle(n);
    if (cs.display === "none" || cs.visibility === "hidden" || cs.opacity === "0") return true;
    if (cs.clipPath && cs.clipPath !== "none") return true;
    if (cs.clip && cs.clip !== "auto") return true;
    if (n !== el) {
      var r = n.getBoundingClientRect();
      if (r.width <= 1 || r.height <= 1) return true;
    }
  }
  return false;
}

function add(kind, severity, el, message, measured) {
  out.push({
    kind: kind,
    severity: severity,
    selector: el ? selectorFor(el) : null,
    message: message,
    measured: measured === undefined ? null : measured,
    rect: el && el.getBoundingClientRect
      ? (function (r) {
          return { x: Math.round(r.left), y: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height) };
        })(el.getBoundingClientRect())
      : null,
  });
}

// The single most common responsive bug.
try {
  var docWidth = document.documentElement.scrollWidth;
  if (docWidth > W + 1) {
    add("overflow", "high", null, "Page scrolls sideways by " + (docWidth - W) + "px", docWidth);
    var all = document.querySelectorAll("body *");
    var blamed = 0;
    for (var i = 0; i < all.length && blamed < 5; i++) {
      var el = all[i];
      var r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      if (r.right > W + 1) {
        var cs = getComputedStyle(el);
        if (cs.position === "fixed" || cs.visibility === "hidden") continue;
        add("overflow-element", "high", el,
            "Right edge at " + Math.round(r.right) + ", overflows by " + Math.round(r.right - W) + "px",
            Math.round(r.right));
        blamed++;
      }
    }
  }
} catch (e) { add("probe-failed", "low", null, "overflow probe: " + e.message); }

// Siblings sitting on top of each other, which is how nav lands over a logo.
try {
  var parents = document.querySelectorAll("body, body *");
  var collisions = 0;
  for (var p = 0; p < parents.length && collisions < 8; p++) {
    if (inSvg(parents[p])) continue;
    var kids = Array.prototype.filter.call(parents[p].children, function (c) {
      var cs = getComputedStyle(c);
      if (cs.position === "absolute" || cs.position === "fixed" || cs.display === "none") return false;
      if (unseen(c)) return false;
      var r = c.getBoundingClientRect();
      return r.width > 8 && r.height > 8;
    });
    for (var a = 0; a < kids.length && collisions < 8; a++) {
      for (var b = a + 1; b < kids.length && collisions < 8; b++) {
        var ra = kids[a].getBoundingClientRect();
        var rb = kids[b].getBoundingClientRect();
        var overlapX = Math.min(ra.right, rb.right) - Math.max(ra.left, rb.left);
        var overlapY = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
        if (overlapX > 4 && overlapY > 4) {
          add("overlap", "high", kids[a],
              "Overlaps " + selectorFor(kids[b]) + " by " + Math.round(overlapX) + " x " + Math.round(overlapY) + "px",
              Math.round(overlapX * overlapY));
          collisions++;
        }
      }
    }
  }
} catch (e) { add("probe-failed", "low", null, "overlap probe: " + e.message); }

// Type too small to read.
try {
  var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, null);
  var seen = [], node, small = 0;
  while ((node = walker.nextNode()) && small < 6) {
    if (!node.nodeValue || !node.nodeValue.trim()) continue;
    var el = node.parentElement;
    if (!el || seen.indexOf(el) !== -1) continue;
    seen.push(el);
    if (unseen(el)) continue;
    var size = parseFloat(getComputedStyle(el).fontSize);
    if (size && size < 12) {
      add("small-text", "medium", el, "Font size is " + size.toFixed(1) + "px", size);
      small++;
    }
  }
} catch (e) { add("probe-failed", "low", null, "type probe: " + e.message); }

// Images being stretched past what they contain.
try {
  var imgs = document.images, blurry = 0;
  for (var i = 0; i < imgs.length && blurry < 6; i++) {
    var img = imgs[i];
    var r = img.getBoundingClientRect();
    if (!img.naturalWidth || r.width < 40) continue;
    if (img.naturalWidth < r.width * 0.8) {
      add("upscaled-image", "medium", img,
          "Rendered at " + Math.round(r.width) + "px from a " + img.naturalWidth + "px source",
          img.naturalWidth);
      blurry++;
    }
  }
} catch (e) { add("probe-failed", "low", null, "image probe: " + e.message); }

// Tap targets, but only where a finger is doing the tapping.
try {
  if (W < 768) {
    var targets = document.querySelectorAll("a, button, [role=button], input[type=submit], summary");
    var tiny = 0;
    for (var i = 0; i < targets.length && tiny < 8; i++) {
      var r = targets[i].getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      if (unseen(targets[i])) continue;
      if (r.width < 44 || r.height < 44) {
        add("tap-target", "medium", targets[i],
            "Target is " + Math.round(r.width) + " x " + Math.round(r.height) + "px, under the 44px minimum",
            Math.round(Math.min(r.width, r.height)));
        tiny++;
      }
    }
  }
} catch (e) { add("probe-failed", "low", null, "tap target probe: " + e.message); }

// A sticky header that eats a short viewport.
try {
  var fixed = document.querySelectorAll("body *"), eaten = 0;
  for (var i = 0; i < fixed.length && eaten < 4; i++) {
    var cs = getComputedStyle(fixed[i]);
    if (cs.position !== "fixed" && cs.position !== "sticky") continue;
    var r = fixed[i].getBoundingClientRect();
    if (r.height > H * 0.3 && r.width > W * 0.5) {
      add("viewport-eater", "low", fixed[i],
          Math.round((r.height / H) * 100) + "% of the viewport height is a " + cs.position + " element",
          Math.round(r.height));
      eaten++;
    }
  }
} catch (e) { add("probe-failed", "low", null, "fixed element probe: " + e.message); }

return { width: W, height: H, findings: out };
"##;


/// How many findings of each severity, highest first.
fn tally(findings: &[Value]) -> (usize, usize, usize) {
    let mut totals = (0usize, 0usize, 0usize);
    for finding in findings {
        match finding.get("severity").and_then(Value::as_str) {
            Some("high") => totals.0 += 1,
            Some("medium") => totals.1 += 1,
            _ => totals.2 += 1,
        }
    }
    totals
}

/// Put the findings that mean the layout is broken first.
///
/// Agents triage badly without a ranking, so this report is opinionated about
/// severity on purpose. The sort is stable, so findings of equal severity stay
/// in the order the probes found them, which is document order.
fn rank(findings: &mut [Value]) {
    findings.sort_by_key(|f| match f.get("severity").and_then(Value::as_str) {
        Some("high") => 0,
        Some("medium") => 1,
        _ => 2,
    });
}

/// Screenshot, console and probes for every panel, in one report.
pub async fn run(app: &AppHandle, state: &Shared) -> Result<Value, String> {
    let panels = tools::list_panels(state);
    if panels.is_empty() {
        return Err("no panels are open".into());
    }

    let mut results = Vec::new();
    let mut totals = (0usize, 0usize, 0usize);

    for panel in &panels {
        let probed = tools::eval_js(state, &panel.id, PROBES).await;
        let console = tools::get_console(state, &panel.id).unwrap_or_default();
        let errors: Vec<&crate::state::ConsoleLine> =
            console.iter().filter(|line| line.level == "error").collect();

        let mut findings = match &probed {
            Ok(value) => value
                .get("findings")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            Err(err) => vec![json!({
                "kind": "probe-failed",
                "severity": "low",
                "message": format!("probes did not run: {err}"),
            })],
        };

        for line in &errors {
            findings.push(json!({
                "kind": "console-error",
                "severity": "low",
                "message": line.text,
            }));
        }

        let (high, medium, low) = tally(&findings);
        totals.0 += high;
        totals.1 += medium;
        totals.2 += low;
        rank(&mut findings);

        results.push(json!({
            "panel": panel.name,
            "id": panel.id,
            "width": panel.width,
            "height": panel.height,
            "source": panel.source,
            "renderedWidth": probed.as_ref().ok().and_then(|v| v.get("width").cloned()),
            "findings": findings,
        }));
    }

    let _ = app;
    // Read the lock into a local first. A json! literal keeps its temporaries
    // alive for the whole expression, which is how a second lock inside one
    // turns into a deadlock against itself.
    let url = state.canvas.lock().unwrap().url.clone();

    Ok(json!({
        "url": url,
        "panels": results,
        "summary": {
            "high": totals.0,
            "medium": totals.1,
            "low": totals.2,
        },
        "severityMeaning": {
            "high": "overflow and overlap: the layout is broken here",
            "medium": "small type, upscaled images and tap targets: usability, and often a design call",
            "low": "console noise and viewport-hungry fixed elements: worth a look, rarely urgent",
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn finding(severity: &str, kind: &str) -> Value {
        json!({ "severity": severity, "kind": kind })
    }

    #[test]
    fn a_broken_layout_is_reported_before_a_design_call() {
        let mut findings = vec![
            finding("low", "console-error"),
            finding("medium", "tap-target"),
            finding("high", "overflow"),
        ];
        rank(&mut findings);
        let order: Vec<&str> = findings
            .iter()
            .map(|f| f["severity"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["high", "medium", "low"]);
    }

    #[test]
    fn findings_of_equal_severity_keep_the_order_they_were_found_in() {
        // Document order is the useful tiebreak: the first overflow on the page
        // is usually the one causing the rest.
        let mut findings = vec![
            finding("high", "first"),
            finding("high", "second"),
            finding("high", "third"),
        ];
        rank(&mut findings);
        let order: Vec<&str> = findings.iter().map(|f| f["kind"].as_str().unwrap()).collect();
        assert_eq!(order, ["first", "second", "third"]);
    }

    #[test]
    fn a_finding_with_no_severity_sinks_rather_than_disappearing() {
        let mut findings = vec![json!({ "kind": "mystery" }), finding("high", "overflow")];
        rank(&mut findings);
        assert_eq!(findings[0]["kind"], "overflow");
        assert_eq!(findings[1]["kind"], "mystery");
    }

    #[test]
    fn the_summary_counts_every_finding_exactly_once() {
        let findings = vec![
            finding("high", "a"),
            finding("high", "b"),
            finding("medium", "c"),
            json!({ "kind": "no severity at all" }),
        ];
        let (high, medium, low) = tally(&findings);
        assert_eq!((high, medium, low), (2, 1, 1));
        assert_eq!(high + medium + low, findings.len());
    }

    #[test]
    fn an_empty_report_counts_nothing() {
        assert_eq!(tally(&[]), (0, 0, 0));
    }

    /// The probes are JavaScript living in a Rust string, so nothing compiles
    /// them. A syntax error would turn every panel's result into a single
    /// "probes did not run" finding and look like a page problem.
    #[test]
    fn the_probe_script_is_valid_javascript() {
        // `return` at the top level is only legal inside a function, which is
        // how eval_js wraps it.
        let wrapped = format!("(function () {{ {PROBES} }})();");
        crate::util::tests_support::assert_js_parses(&wrapped, "audit probes");
    }
}
