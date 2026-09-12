//! Everything about one note, in one call.
//!
//! WHY THIS EXISTS
//!
//! A note says what is wrong and where. It does not say what the element
//! actually measures, what it measures at the other widths, what the page
//! logged, or which rule is producing the value being complained about. A
//! session handed a note had to go and get all four, one bridge call at a
//! time, and those calls are the same four every single time.
//!
//! So this is the four calls, made at once, keyed by the note's own id. It is
//! the difference between a session spending five round trips establishing
//! facts the app already had, and a session starting with the answer.
//!
//! THE COMPARISON IS THE POINT
//!
//! The measurement is taken in every open panel, not only in the one the note
//! came from. A padding of 8px at 375px means nothing on its own; 8px at 375
//! and 24px everywhere else is the bug, stated. That comparison is the thing
//! this app exists to make cheap, and doing it by hand was five calls and some
//! arithmetic.
//!
//! WHAT IT DOES NOT DO
//!
//! It does not judge. Nothing here says the element is wrong, because the app
//! cannot know that: a width that differs from the others is often exactly
//! what the design asks for. It lays the four facts out and leaves the reading
//! to whoever asked.

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::state::Shared;
use crate::tools;

/// The same element, measured in one panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtWidth {
    pub panel: String,
    pub panel_name: String,
    pub width: f64,
    /// False when the selector matches nothing in this panel, which is itself
    /// worth knowing: an element that exists at one width and not another is a
    /// different bug from one that is the wrong size.
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_: Option<Value>,
    /// The handful of properties a layout note is nearly always about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub styles: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One CSS rule that matches the element, and where it came from.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchingRule {
    pub selector: String,
    /// The media condition it sits inside, when it sits inside one. This is
    /// what turns "why is it 8px" into "because the 24px rule is behind a
    /// min-width this panel does not meet".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    /// Whether that condition is true in this panel right now.
    pub applies: bool,
    /// The declarations, as written.
    pub declarations: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub id: String,
    pub note: String,
    pub width: f64,
    pub panel: String,
    pub panel_name: String,
    pub selector: String,
    pub element: String,
    pub url: String,
    pub at: u64,
    /// True once a session has been handed it, and what it said back.
    pub delivered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<crate::state::Reply>,

    /// The element as it stands now, truncated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markup: Option<String>,
    /// The same element in every open panel, left to right.
    pub across_widths: Vec<AtWidth>,
    /// Properties that differ between widths, named. The short answer.
    pub differs: Vec<String>,
    /// Rules matching the element in the panel the note came from.
    pub matching_rules: Vec<MatchingRule>,
    /// What this panel's page logged as an error, most recent last.
    pub console_errors: Vec<String>,
    pub summary: String,
}

/// Properties a layout note is nearly always about.
///
/// Deliberately short. Every computed property is six hundred values per panel
/// and answers nothing; these are the ones that move when a layout is wrong at
/// one width.
const WATCHED: &[&str] = &[
    "display",
    "position",
    "font-size",
    "line-height",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "flex-direction",
    "flex-wrap",
    "grid-template-columns",
    "overflow-x",
    "text-align",
    "gap",
];

/// Measure one element: its box and the watched properties.
fn measure_script(selector: &str) -> String {
    format!(
        r#"
var el = document.querySelector({sel});
if (!el) return {{ found: false }};
var r = el.getBoundingClientRect();
var cs = getComputedStyle(el);
var want = {watched};
var styles = {{}};
for (var i = 0; i < want.length; i++) styles[want[i]] = cs.getPropertyValue(want[i]);
return {{
  found: true,
  box: {{
    x: Math.round(r.left), y: Math.round(r.top + window.scrollY),
    w: Math.round(r.width * 10) / 10, h: Math.round(r.height * 10) / 10,
  }},
  styles: styles,
  viewport: window.innerWidth,
}};
"#,
        sel = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
        watched = serde_json::to_string(WATCHED).unwrap_or_else(|_| "[]".into()),
    )
}

/// The element's markup, cut short.
fn markup_script(selector: &str) -> String {
    format!(
        r#"
var el = document.querySelector({sel});
if (!el) return null;
var html = el.outerHTML || "";
return html.length > 2000 ? html.slice(0, 2000) + "…" : html;
"#,
        sel = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
    )
}

/// Every rule in the page that matches this element, in the order the browser
/// read them, with the media condition each sits inside.
///
/// Order is document order rather than specificity order, because specificity
/// is a calculation and this is a report: the rules are listed as written and
/// the reader can see which one is behind a query that is not firing. A
/// cross-origin stylesheet throws on `cssRules`, so each is tried on its own.
fn rules_script(selector: &str) -> String {
    format!(
        r#"
var el = document.querySelector({sel});
if (!el) return [];
var out = [];

function walk(rules, media, href) {{
  for (var i = 0; i < rules.length; i++) {{
    var rule = rules[i];
    if (rule.media && typeof rule.conditionText === "string") {{
      var inner = media ? media + " and " + rule.conditionText : rule.conditionText;
      var applies = true;
      try {{ applies = window.matchMedia(rule.conditionText).matches; }} catch (e) {{}}
      if (rule.cssRules) walk(rule.cssRules, inner, href);
      continue;
    }}
    if (rule.selectorText) {{
      var hit = false;
      // A selector list can contain one part that throws, so each is tried.
      var parts = rule.selectorText.split(",");
      for (var p = 0; p < parts.length; p++) {{
        try {{ if (el.matches(parts[p].trim())) {{ hit = true; break; }} }} catch (e) {{}}
      }}
      if (hit) {{
        var applies = true;
        if (media) {{ try {{ applies = window.matchMedia(media).matches; }} catch (e) {{}} }}
        var body = rule.cssText || "";
        var open = body.indexOf("{{");
        out.push({{
          selector: rule.selectorText,
          media: media || null,
          applies: applies,
          declarations: open >= 0 ? body.slice(open + 1, body.lastIndexOf("}}")).trim() : "",
          source: href || null,
        }});
      }}
      continue;
    }}
    if (rule.cssRules) walk(rule.cssRules, media, href);
  }}
}}

for (var s = 0; s < document.styleSheets.length; s++) {{
  var sheet = document.styleSheets[s];
  var rules = null;
  try {{ rules = sheet.cssRules; }} catch (e) {{ continue; }}
  if (!rules) continue;
  try {{ walk(rules, null, sheet.href); }} catch (e) {{}}
}}
// Long stylesheets can match a lot; the last ones are the ones that win.
return out.length > 40 ? out.slice(out.length - 40) : out;
"#,
        sel = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into()),
    )
}

/// Explain one note.
pub async fn explain(app: &AppHandle, state: &Shared, id: &str) -> Result<Explanation, String> {
    let note = state
        .recent_notes()
        .into_iter()
        .find(|note| note.id == id)
        .ok_or_else(|| {
            let known: Vec<String> = state
                .recent_notes()
                .into_iter()
                .map(|note| format!("{} ({}px)", note.id, note.width.round()))
                .collect();
            if known.is_empty() {
                "no notes have been written in this sitting".to_string()
            } else {
                format!("no note with id \"{id}\". This sitting has: {}", known.join(", "))
            }
        })?;
    let _ = app;

    let selector = note.selector.clone();
    if selector.trim().is_empty() {
        return Err("this note points at a region rather than an element, so there is nothing to measure".into());
    }

    // Markup and rules come from the panel the note was written in, because
    // that is the width the complaint is about.
    let markup = tools::eval_js(state, &note.panel, &markup_script(&selector))
        .await
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    let matching_rules: Vec<MatchingRule> =
        match tools::eval_js(state, &note.panel, &rules_script(&selector)).await {
            Ok(value) => serde_json::from_value(value).unwrap_or_default(),
            Err(_) => Vec::new(),
        };

    // The measurement, in every panel, which is the comparison that matters.
    let mut across_widths = Vec::new();
    for panel in tools::list_panels(state) {
        let answer = tools::eval_js(state, &panel.id, &measure_script(&selector)).await;
        match answer {
            Ok(value) => {
                let found = value.get("found").and_then(Value::as_bool).unwrap_or(false);
                across_widths.push(AtWidth {
                    panel: panel.id.clone(),
                    panel_name: panel.name.clone(),
                    width: panel.width,
                    found,
                    box_: value.get("box").cloned(),
                    styles: value.get("styles").cloned(),
                    error: None,
                });
            }
            Err(err) => across_widths.push(AtWidth {
                panel: panel.id.clone(),
                panel_name: panel.name.clone(),
                width: panel.width,
                found: false,
                box_: None,
                styles: None,
                error: Some(err),
            }),
        }
    }

    let differs = differing_properties(&across_widths);

    let console_errors: Vec<String> = tools::get_console(state, &note.panel)
        .unwrap_or_default()
        .into_iter()
        .filter(|line| line.level == "error")
        .map(|line| line.text)
        .collect();

    let summary = summarise(&note, &across_widths, &differs, &console_errors);

    Ok(Explanation {
        id: note.id.clone(),
        note: note.note.clone(),
        width: note.width,
        panel: note.panel.clone(),
        panel_name: note.panel_name.clone(),
        selector,
        element: note.element.clone(),
        url: note.url.clone(),
        at: note.at,
        delivered: note.delivered,
        reply: note.reply.clone(),
        markup,
        across_widths,
        differs,
        matching_rules,
        console_errors,
        summary,
    })
}

/// Which of the watched properties are not the same at every width.
///
/// The short answer to "what is different here". A property with one value
/// across the whole row is noise; a property with two is where to look.
fn differing_properties(across: &[AtWidth]) -> Vec<String> {
    let mut out = Vec::new();
    let found: Vec<&AtWidth> = across.iter().filter(|a| a.found).collect();
    if found.len() < 2 {
        return out;
    }
    for property in WATCHED {
        let mut seen: Option<&str> = None;
        for at in &found {
            let value = at
                .styles
                .as_ref()
                .and_then(|s| s.get(*property))
                .and_then(Value::as_str)
                .unwrap_or("");
            match seen {
                None => seen = Some(value),
                Some(first) if first != value => {
                    out.push((*property).to_string());
                    break;
                }
                _ => {}
            }
        }
    }
    out
}

fn summarise(
    note: &crate::state::NoteRecord,
    across: &[AtWidth],
    differs: &[String],
    console_errors: &[String],
) -> String {
    let missing: Vec<String> = across
        .iter()
        .filter(|a| !a.found && a.error.is_none())
        .map(|a| format!("{}px", a.width.round()))
        .collect();

    let mut parts = vec![format!(
        "\"{}\" at {}px",
        note.note.trim(),
        note.width.round()
    )];

    if !missing.is_empty() {
        parts.push(format!(
            "the element is not in the page at {}",
            missing.join(", ")
        ));
    }
    if differs.is_empty() {
        parts.push("nothing measured differs between the open widths".into());
    } else {
        parts.push(format!("differs between widths: {}", differs.join(", ")));
    }
    if !console_errors.is_empty() {
        parts.push(format!(
            "{} console error{} at this width",
            console_errors.len(),
            if console_errors.len() == 1 { "" } else { "s" }
        ));
    }
    parts.join(". ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(width: f64, found: bool, styles: Value) -> AtWidth {
        AtWidth {
            panel: format!("p{width}"),
            panel_name: format!("{width}"),
            width,
            found,
            box_: None,
            styles: Some(styles),
            error: None,
        }
    }

    /// The whole point: a property with one value everywhere is noise, and a
    /// property with two is the answer.
    #[test]
    fn only_properties_that_move_between_widths_are_named() {
        let across = vec![
            at(375.0, true, serde_json::json!({ "padding-top": "8px", "display": "flex" })),
            at(768.0, true, serde_json::json!({ "padding-top": "24px", "display": "flex" })),
        ];
        assert_eq!(differing_properties(&across), vec!["padding-top".to_string()]);
    }

    /// One panel cannot disagree with itself, so nothing is claimed.
    #[test]
    fn a_single_width_names_nothing() {
        let across = vec![at(375.0, true, serde_json::json!({ "padding-top": "8px" }))];
        assert!(differing_properties(&across).is_empty());
    }

    /// A width where the element is absent is left out of the comparison
    /// rather than counted as an empty value, which would make every property
    /// look as though it moved.
    #[test]
    fn a_width_without_the_element_does_not_fake_a_difference() {
        let across = vec![
            at(375.0, true, serde_json::json!({ "padding-top": "8px" })),
            at(768.0, true, serde_json::json!({ "padding-top": "8px" })),
            at(1024.0, false, serde_json::json!({})),
        ];
        assert!(differing_properties(&across).is_empty());
    }

    /// The summary says when the element is missing somewhere, because an
    /// element that exists at one width and not another is its own bug.
    #[test]
    fn the_summary_says_where_the_element_is_missing() {
        let note = crate::state::NoteRecord {
            id: "x".into(),
            panel: "sm".into(),
            panel_name: "Small".into(),
            width: 375.0,
            note: "the nav is gone".into(),
            selector: "#nav".into(),
            element: "nav".into(),
            url: String::new(),
            at: 1,
            delivered: false,
            delivered_to: None,
            reply: None,
        };
        let across = vec![
            at(375.0, true, serde_json::json!({})),
            at(768.0, false, serde_json::json!({})),
        ];
        let said = summarise(&note, &across, &[], &[]);
        assert!(said.contains("not in the page at 768px"), "{said}");
    }

    /// The measuring script has to be valid JavaScript, and a selector with a
    /// quote in it cannot break out of the string it is put in.
    #[test]
    fn a_quote_in_the_selector_cannot_escape_the_script() {
        let script = measure_script("a[href=\"x\"]");
        assert!(script.contains(r#"querySelector("a[href=\"x\"]")"#), "{script}");
    }
}
