//! Accessibility checks, one per panel, with the breakpoint width attached.
//!
//! This is the same thesis as the Report button. A violation that only exists
//! at one width is invisible to a tool that tests one width, and a tap target
//! too small on a phone, a heading order that only breaks when a column
//! stacks, or contrast against a background image that only shows at a
//! narrow viewport are all of that kind.
//!
//! axe-core is vendored rather than fetched: the audit has to work with no
//! network, and the version has to be the same one the tests were written
//! against. It is injected the way the panel picker is, because a WKWebView
//! cannot be driven by the Chrome DevTools Protocol and so Lighthouse is not
//! an option here.
//!
//! Every check runs in the page, so the cost is the page's, not the app's.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::Shared;
use crate::tools;

/// The whole library, in the binary. 580KB, and the alternative is an audit
/// that fails on a plane.
const AXE: &str = include_str!("../assets/axe.min.js");

/// How long one panel gets to finish. axe on a heavy page is seconds, not
/// milliseconds, and a slow answer is still worth having.
const PANEL_BUDGET: Duration = Duration::from_secs(30);

/// How often to ask a panel whether it has finished.
const POLL: Duration = Duration::from_millis(250);

/// One violation, as it was found in one panel.
///
/// Deserialized as well as serialized, because this is also the shape the
/// injected script hands back out of the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelViolation {
    pub rule: String,
    pub impact: String,
    pub help: String,
    pub help_url: String,
    /// The elements it was found on, as selectors.
    pub targets: Vec<String>,
}

/// Every violation found at one width.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelAudit {
    pub panel: String,
    pub panel_name: String,
    pub width: f64,
    pub violations: Vec<PanelViolation>,
    /// Set instead of `violations` when the panel could not be asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One rule, across every width it was found at.
///
/// This is the shape that earns the app its keep. `widths` is the whole point:
/// a rule broken at 375 and nowhere else is a different problem from one
/// broken everywhere, and it is the one a single-width tool cannot see.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleAcrossWidths {
    pub rule: String,
    pub impact: String,
    pub help: String,
    pub help_url: String,
    /// Every width the rule was broken at, ascending.
    pub widths: Vec<f64>,
    /// True when it was not broken at every width that answered, which makes
    /// it a responsive problem rather than a page-wide one.
    pub width_specific: bool,
    /// A few of the elements, for orientation rather than completeness.
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessReport {
    /// Ranked: the worst impact first, and within an impact the ones that only
    /// happen at some widths, because those are the ones nothing else finds.
    pub rules: Vec<RuleAcrossWidths>,
    pub panels: Vec<PanelAudit>,
    pub widths_audited: Vec<f64>,
    pub violations_total: usize,
    /// How many rules were broken at some widths and not others.
    pub width_specific_count: usize,
    pub axe_version: String,
}

fn impact_rank(impact: &str) -> u8 {
    match impact {
        "critical" => 0,
        "serious" => 1,
        "moderate" => 2,
        "minor" => 3,
        _ => 4,
    }
}

/// Inject the library, start a run, and wait for it.
///
/// Three evals rather than one, because `eval_js` cannot wait for a promise:
/// the value a webview hands back is whatever the expression evaluated to, and
/// a pending promise serialises to an empty object. So the run parks its result
/// on a window property and we read that.
async fn audit_one(state: &Shared, panel_id: &str) -> Result<Vec<PanelViolation>, String> {
    // Concatenated, never formatted: the minified library is full of braces and
    // every one of them would have to be doubled for `format!`.
    let mut inject = String::with_capacity(AXE.len() + 512);
    inject.push_str("if (!window.axe) { try { ");
    inject.push_str(AXE);
    inject.push_str(" } catch (e) { window.__bpAxeLoadError = String(e); } } ");
    inject.push_str("return window.axe ? \"ready\" : (window.__bpAxeLoadError || \"axe did not load\");");

    let loaded = tools::eval_js(state, panel_id, &inject).await?;
    if loaded.as_str() != Some("ready") {
        return Err(format!(
            "axe-core did not load in this panel: {}",
            loaded.as_str().unwrap_or("no reason given")
        ));
    }

    // Only violations. axe returns passes and incomplete results too, and on a
    // real page those are thousands of lines nobody reads.
    let start = r#"
window.__bpAxeDone = null;
window.__bpAxeError = null;
window.axe.run(document, { resultTypes: ["violations"] })
  .then(function (results) { window.__bpAxeDone = results; })
  .catch(function (e) { window.__bpAxeError = String((e && e.message) || e); });
return "started";
"#;
    tools::eval_js(state, panel_id, start).await?;

    let read = r#"
if (window.__bpAxeError) return { state: "error", error: window.__bpAxeError };
if (!window.__bpAxeDone) return { state: "running" };
return {
  state: "done",
  violations: (window.__bpAxeDone.violations || []).map(function (v) {
    return {
      rule: v.id,
      impact: v.impact || "unknown",
      help: v.help || "",
      helpUrl: v.helpUrl || "",
      targets: (v.nodes || []).slice(0, 10).map(function (n) {
        return (n.target || []).join(" ");
      }),
    };
  }),
};
"#;

    let deadline = tokio::time::Instant::now() + PANEL_BUDGET;
    loop {
        let answer = tools::eval_js(state, panel_id, read).await?;
        match answer.get("state").and_then(Value::as_str) {
            Some("done") => {
                let list = answer
                    .get("violations")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(vec![]));
                return serde_json::from_value::<Vec<PanelViolation>>(list)
                    .map_err(|e| format!("axe results did not parse: {e}"));
            }
            Some("error") => {
                return Err(answer
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("axe failed")
                    .to_string())
            }
            _ => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "axe did not finish within {} seconds",
                        PANEL_BUDGET.as_secs()
                    ));
                }
                tokio::time::sleep(POLL).await;
            }
        }
    }
}

/// Every panel, in order, and one report.
///
/// Sequentially on purpose. Seven copies of axe walking seven DOMs at once
/// makes every one of them slower and the machine unusable, and this is a
/// measuring instrument: a result that took twice as long to collect is still
/// the same result, but a machine under load reports different timings.
pub async fn audit_all(state: &Shared) -> Result<AccessReport, String> {
    let panels = tools::list_panels(state);
    if panels.is_empty() {
        return Err("no panels are open, so there is nothing to audit. Open a project or navigate to a URL first.".into());
    }

    let mut audits: Vec<PanelAudit> = Vec::new();
    for panel in &panels {
        let (violations, error) = match audit_one(state, &panel.id).await {
            Ok(found) => (found, None),
            // One panel failing must not lose the other six. A panel that
            // never loaded is the usual reason, and it is worth saying which.
            Err(err) => (Vec::new(), Some(err)),
        };
        audits.push(PanelAudit {
            panel: panel.id.clone(),
            panel_name: panel.name.clone(),
            width: panel.width,
            violations,
            error,
        });
    }

    Ok(summarise(audits, env!("CARGO_PKG_VERSION")))
}

/// Turn per-panel results into the cross-width view.
///
/// Pure, so the ranking is unit tested without a window.
pub fn summarise(audits: Vec<PanelAudit>, _version: &str) -> AccessReport {
    let answered: Vec<&PanelAudit> = audits.iter().filter(|a| a.error.is_none()).collect();
    let widths_audited: Vec<f64> = answered.iter().map(|a| a.width).collect();

    let mut grouped: BTreeMap<String, RuleAcrossWidths> = BTreeMap::new();
    let mut violations_total = 0usize;

    for audit in &answered {
        for violation in &audit.violations {
            violations_total += 1;
            let entry = grouped
                .entry(violation.rule.clone())
                .or_insert_with(|| RuleAcrossWidths {
                    rule: violation.rule.clone(),
                    impact: violation.impact.clone(),
                    help: violation.help.clone(),
                    help_url: violation.help_url.clone(),
                    widths: Vec::new(),
                    width_specific: false,
                    examples: Vec::new(),
                });
            if !entry.widths.contains(&audit.width) {
                entry.widths.push(audit.width);
            }
            // The worst impact seen for a rule is the one that matters.
            if impact_rank(&violation.impact) < impact_rank(&entry.impact) {
                entry.impact = violation.impact.clone();
            }
            for target in &violation.targets {
                if entry.examples.len() < 3 && !entry.examples.contains(target) {
                    entry.examples.push(target.clone());
                }
            }
        }
    }

    let mut rules: Vec<RuleAcrossWidths> = grouped.into_values().collect();
    for rule in rules.iter_mut() {
        rule.widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // Broken everywhere that answered is a page problem; broken at some
        // widths is the responsive one this app exists to surface.
        rule.width_specific = !widths_audited.is_empty() && rule.widths.len() < widths_audited.len();
    }

    // Worst impact first, then width-specific before page-wide, then the
    // narrower failures, then the name so the order never wobbles.
    rules.sort_by(|a, b| {
        impact_rank(&a.impact)
            .cmp(&impact_rank(&b.impact))
            .then_with(|| b.width_specific.cmp(&a.width_specific))
            .then_with(|| a.widths.len().cmp(&b.widths.len()))
            .then_with(|| a.rule.cmp(&b.rule))
    });

    let width_specific_count = rules.iter().filter(|r| r.width_specific).count();

    AccessReport {
        rules,
        panels: audits,
        widths_audited,
        violations_total,
        width_specific_count,
        axe_version: axe_version().to_string(),
    }
}

/// Read the version out of the vendored bundle, so a report says which axe
/// found these rather than whatever version happens to be documented.
pub fn axe_version() -> &'static str {
    // axe.min.js carries `.version="4.13.0"` near the top.
    const NEEDLE: &str = "version=\"";
    let head = &AXE[..AXE.len().min(4000)];
    match head.find(NEEDLE) {
        Some(at) => {
            let rest = &head[at + NEEDLE.len()..];
            match rest.find('"') {
                Some(end) => &rest[..end],
                None => "unknown",
            }
        }
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violation(rule: &str, impact: &str) -> PanelViolation {
        PanelViolation {
            rule: rule.into(),
            impact: impact.into(),
            help: format!("{rule} help"),
            help_url: format!("https://dequeuniversity.com/rules/axe/{rule}"),
            targets: vec![format!("#{rule}")],
        }
    }

    fn audit(width: f64, violations: Vec<PanelViolation>) -> PanelAudit {
        PanelAudit {
            panel: format!("p{width}"),
            panel_name: format!("{width}px"),
            width,
            violations,
            error: None,
        }
    }

    /// The whole reason this exists. A rule broken at one width and not the
    /// others is a responsive problem, and it has to be marked as one.
    #[test]
    fn a_rule_broken_at_one_width_only_is_marked_width_specific() {
        let report = summarise(
            vec![
                audit(375.0, vec![violation("target-size", "serious")]),
                audit(768.0, vec![]),
                audit(1280.0, vec![]),
            ],
            "test",
        );
        assert_eq!(report.rules.len(), 1);
        assert_eq!(report.rules[0].widths, vec![375.0]);
        assert!(report.rules[0].width_specific);
        assert_eq!(report.width_specific_count, 1);
    }

    #[test]
    fn a_rule_broken_everywhere_is_not_width_specific() {
        let report = summarise(
            vec![
                audit(375.0, vec![violation("image-alt", "critical")]),
                audit(768.0, vec![violation("image-alt", "critical")]),
            ],
            "test",
        );
        assert_eq!(report.rules[0].widths, vec![375.0, 768.0]);
        assert!(!report.rules[0].width_specific);
        assert_eq!(report.width_specific_count, 0);
    }

    /// An agent triages badly without an order, so the order is opinionated:
    /// impact first, and within one impact the width-specific one first.
    #[test]
    fn the_worst_and_the_narrowest_come_first() {
        let report = summarise(
            vec![
                audit(
                    375.0,
                    vec![
                        violation("minor-thing", "minor"),
                        violation("everywhere", "serious"),
                        violation("only-here", "serious"),
                        violation("worst", "critical"),
                    ],
                ),
                audit(768.0, vec![violation("everywhere", "serious")]),
            ],
            "test",
        );
        let order: Vec<&str> = report.rules.iter().map(|r| r.rule.as_str()).collect();
        assert_eq!(order, vec!["worst", "only-here", "everywhere", "minor-thing"]);
    }

    /// A panel that could not be asked must not be counted as a panel where
    /// nothing was wrong, because that would make every rule look
    /// width-specific.
    #[test]
    fn a_panel_that_failed_is_not_counted_as_a_clean_one() {
        let mut failed = audit(1280.0, vec![]);
        failed.error = Some("panel never loaded".into());
        let report = summarise(
            vec![
                audit(375.0, vec![violation("image-alt", "critical")]),
                audit(768.0, vec![violation("image-alt", "critical")]),
                failed,
            ],
            "test",
        );
        assert_eq!(report.widths_audited, vec![375.0, 768.0]);
        assert!(
            !report.rules[0].width_specific,
            "broken at both widths that answered"
        );
        assert_eq!(report.panels.len(), 3, "the failure is still reported");
    }

    #[test]
    fn the_report_says_which_axe_found_these() {
        let version = axe_version();
        assert!(
            version.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "read a version out of the bundle, got {version}"
        );
    }
}
