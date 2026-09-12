//! Ask the page what its breakpoints are, and compare that to what the code
//! said.
//!
//! Everything else in this app infers breakpoints from source: a config parsed
//! without running it, media queries read out of stylesheets, variables
//! resolved by hand. All of that is a model of what the browser will do.
//!
//! The browser is already here. A panel holds the real, resolved stylesheet:
//! the preprocessor has run, imports are followed, custom media is expanded,
//! and a container query is distinguishable from a viewport one because the
//! engine parsed it. Reading the media rules out of the page is the ground
//! truth for the exact question the scanner estimates.
//!
//! This is a check, not a detector. It cannot replace the scanner, because it
//! needs a panel pointed at a running site, and the scanner has to work on a
//! folder before anything is running. What it can do is confirm what the
//! scanner found, and say what it missed.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::scanner::units::widths_in_query;
use crate::state::Shared;
use crate::tools;

/// One width, and whether the code and the page agree about it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidthCheck {
    pub width: f64,
    /// The name the project gave it, when the scan found one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// True when the scan found this width in the code.
    pub in_code: bool,
    /// How many media rules in the live page use it.
    pub rules_in_page: usize,
    pub verdict: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub panel: String,
    pub url: String,
    pub widths: Vec<WidthCheck>,
    /// Stylesheets the page would not let us read, almost always because they
    /// came from another origin. Their rules are invisible here, so a width
    /// only used in one of them reads as missing when it is not.
    pub unreadable_stylesheets: usize,
    pub stylesheets: usize,
    /// Media rules that had no width condition: print, colour scheme, reduced
    /// motion. Counted so a page full of them does not read as a page with
    /// nothing in it.
    pub rules_without_a_width: usize,
    /// Container queries in the live page. Not viewport widths and never
    /// panels, but a project that has moved to them looks empty otherwise.
    pub container_rules: usize,
    pub summary: String,
}

/// Walk the page's own stylesheets and report every media condition.
///
/// Cross-origin sheets throw on `cssRules` rather than returning nothing, so
/// each one is tried on its own and counted when it refuses.
const READ_THE_PAGE: &str = r#"
var conditions = {};
var sheets = 0;
var unreadable = 0;
var containers = 0;

function walk(rules) {
  for (var i = 0; i < rules.length; i++) {
    var rule = rules[i];
    // CSSMediaRule
    if (rule.media && typeof rule.conditionText === "string") {
      var text = rule.conditionText;
      conditions[text] = (conditions[text] || 0) + 1;
    }
    // CSSContainerRule, which is about an element and never a panel
    if (rule.constructor && rule.constructor.name === "CSSContainerRule") {
      containers += 1;
    }
    if (rule.cssRules) {
      try { walk(rule.cssRules); } catch (e) {}
    }
  }
}

for (var s = 0; s < document.styleSheets.length; s++) {
  sheets += 1;
  var sheet = document.styleSheets[s];
  var rules = null;
  try { rules = sheet.cssRules; } catch (e) { unreadable += 1; continue; }
  if (!rules) { unreadable += 1; continue; }
  try { walk(rules); } catch (e) { unreadable += 1; }
}

return {
  conditions: conditions,
  sheets: sheets,
  unreadable: unreadable,
  containers: containers,
  url: location.href,
};
"#;

/// Compare the widths the code declares against the widths the page uses.
pub async fn verify(state: &Shared, panel: &str) -> Result<Verification, String> {
    let id = crate::canvas::resolve_id(state, panel)
        .ok_or_else(|| format!("no panel matches \"{panel}\""))?;
    if let Some(reason) = crate::canvas::cannot_measure(state, Some(&id)) {
        return Err(reason);
    }

    let answer = tools::eval_js(state, &id, READ_THE_PAGE).await?;
    let conditions = answer
        .get("conditions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // Widths the live page actually uses, and how many rules use each.
    let mut in_page: BTreeMap<i64, usize> = BTreeMap::new();
    let mut without_a_width = 0usize;
    for (condition, count) in &conditions {
        let count = count.as_u64().unwrap_or(1) as usize;
        let hits = widths_in_query(condition);
        if hits.is_empty() {
            without_a_width += count;
            continue;
        }
        for hit in hits {
            *in_page.entry(hit.boundary.round() as i64).or_default() += count;
        }
    }

    // Widths the code declares, from whatever the row was built from.
    let declared: Vec<(f64, Option<String>)> = tools::list_panels(state)
        .into_iter()
        .map(|panel| (panel.width, Some(panel.name)))
        .collect();

    let mut widths: Vec<WidthCheck> = Vec::new();
    for (width, name) in &declared {
        let key = width.round() as i64;
        let rules = in_page.get(&key).copied().unwrap_or(0);
        widths.push(WidthCheck {
            width: *width,
            name: name.clone(),
            in_code: true,
            rules_in_page: rules,
            verdict: if rules > 0 {
                "the page has rules at this width"
            } else {
                "nothing in the page changes at this width"
            },
        });
    }
    for (key, rules) in &in_page {
        if declared.iter().any(|(w, _)| w.round() as i64 == *key) {
            continue;
        }
        widths.push(WidthCheck {
            width: *key as f64,
            name: None,
            in_code: false,
            rules_in_page: *rules,
            verdict: "the page changes here and no panel is open at it",
        });
    }
    widths.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());

    let missing = widths.iter().filter(|w| w.in_code && w.rules_in_page == 0).count();
    let extra = widths.iter().filter(|w| !w.in_code).count();
    let unreadable = answer.get("unreadable").and_then(Value::as_u64).unwrap_or(0) as usize;

    let mut summary = format!(
        "{} of {} open widths have rules in this page",
        widths.iter().filter(|w| w.in_code && w.rules_in_page > 0).count(),
        declared.len()
    );
    if extra > 0 {
        summary.push_str(&format!(", and the page changes at {extra} widths with no panel"));
    }
    if missing > 0 && unreadable > 0 {
        summary.push_str(&format!(
            ". {unreadable} stylesheets could not be read, so a missing width may only be missing from view"
        ));
    }

    Ok(Verification {
        panel: id,
        url: answer
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        widths,
        unreadable_stylesheets: unreadable,
        stylesheets: answer.get("sheets").and_then(Value::as_u64).unwrap_or(0) as usize,
        rules_without_a_width: without_a_width,
        container_rules: answer.get("containers").and_then(Value::as_u64).unwrap_or(0) as usize,
        summary,
    })
}
