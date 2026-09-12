//! Turning breakpoints into panels.
//!
//! A breakpoint is a width. A panel needs a height and a name as well, and both
//! get derived here and stay editable afterwards. Panels render at exactly the
//! breakpoint width, because that is the first pixel of the range and it is
//! where layouts actually break.

use serde::Serialize;

use crate::model::{HeightStrategy, Viewport};
use crate::scanner::drupal;
use crate::scanner::types::{BreakpointDiscovery, Edge, Kind};

/// Aspect ratios taken from a real device in each size class, so a 768 panel
/// looks like a tablet and a 1440 panel looks like a laptop.
const RATIOS: &[(f64, f64, &str)] = &[
    (480.0, 2.16, "iPhone 15 Pro, 393 x 852"),
    (900.0, 1.33, "iPad portrait, 768 x 1024"),
    (1279.0, 0.75, "iPad landscape, 1024 x 768"),
    (f64::MAX, 0.625, "MacBook Air, 1440 x 900"),
];

/// Real-world names, because "Tablet" reads faster than "md" when you are
/// scanning a row of five.
const BANDS: &[(f64, &str)] = &[
    (480.0, "Mobile"),
    (767.0, "Large Mobile"),
    (1023.0, "Tablet"),
    (1279.0, "Laptop"),
    (1535.0, "Desktop"),
    (f64::MAX, "Wide"),
];

pub fn height_for(width: f64, strategy: HeightStrategy, fixed: f64) -> f64 {
    match strategy {
        HeightStrategy::Fixed | HeightStrategy::Manual => fixed.max(320.0),
        HeightStrategy::DeviceRatio => {
            let ratio = RATIOS
                .iter()
                .find(|(limit, _, _)| width <= *limit)
                .map(|(_, ratio, _)| *ratio)
                .unwrap_or(0.625);
            ((width * ratio / 10.0).round() * 10.0).max(640.0)
        }
    }
}

pub fn reference_device(width: f64) -> &'static str {
    RATIOS
        .iter()
        .find(|(limit, _, _)| width <= *limit)
        .map(|(_, _, device)| *device)
        .unwrap_or("MacBook Air, 1440 x 900")
}

pub fn band_name(width: f64) -> &'static str {
    BANDS
        .iter()
        .find(|(limit, _)| width <= *limit)
        .map(|(_, name)| *name)
        .unwrap_or("Wide")
}

/// One option in the scan sheet: a viewport plus why it is being offered.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(flatten)]
    pub viewport: Viewport,
    pub checked: bool,
    pub file_count: usize,
    /// The right-hand column: a framework key, or how many files use it.
    pub detail: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    /// Checked by default. Framework-configured breakpoints, always.
    pub recommended: Vec<Candidate>,
    /// Unchecked and available, ranked by how many files use them.
    pub also_found: Vec<Candidate>,
    /// True when nothing was found and these are the generic device sizes.
    pub fallback: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_url: Option<String>,
}

/// Split what the scan found into what to check and what to merely offer.
pub fn recommend(
    breakpoints: &[BreakpointDiscovery],
    strategy: HeightStrategy,
    fixed: f64,
    edge_testing: bool,
) -> Recommendation {
    if breakpoints.is_empty() {
        return Recommendation {
            recommended: crate::model::fallback_viewports()
                .into_iter()
                .map(|viewport| Candidate {
                    detail: "standard size".into(),
                    checked: true,
                    file_count: 0,
                    source_file: String::new(),
                    line: None,
                    confidence: 0.2,
                    viewport,
                })
                .collect(),
            also_found: Vec::new(),
            fallback: true,
            dev_url: None,
        };
    }

    let has_framework = breakpoints
        .iter()
        .any(|b| matches!(b.kind, Kind::Configured | Kind::Framework));

    let mut configured: Vec<&BreakpointDiscovery> = breakpoints
        .iter()
        .filter(|b| matches!(b.kind, Kind::Configured | Kind::Framework))
        .collect();
    configured.sort_by(|a, b| a.width.partial_cmp(&b.width).unwrap());

    let mut extra: Vec<&BreakpointDiscovery> = breakpoints
        .iter()
        .filter(|b| matches!(b.kind, Kind::Css | Kind::Inferred))
        .collect();
    // Widely used first, because that is the ranking that makes the list
    // scannable in five seconds.
    extra.sort_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then(a.width.partial_cmp(&b.width).unwrap())
    });
    extra.truncate(crate::scanner::css::MAX_REPORTED);

    let mut recommended = build(&configured, strategy, fixed, edge_testing, true);

    // With no framework at all, the CSS widths are the only real answer, so the
    // most-used few get checked rather than merely offered.
    let auto_check = if has_framework { 0 } else { 3.min(extra.len()) };
    let (auto, offered) = extra.split_at(auto_check);
    if !auto.is_empty() {
        recommended.extend(build(auto, strategy, fixed, edge_testing, true));
        recommended.sort_by(|a, b| a.viewport.width.partial_cmp(&b.viewport.width).unwrap());
    }

    Recommendation {
        recommended,
        also_found: build(offered, strategy, fixed, false, false),
        fallback: false,
        dev_url: None,
    }
}

fn build(
    found: &[&BreakpointDiscovery],
    strategy: HeightStrategy,
    fixed: f64,
    edge_testing: bool,
    checked: bool,
) -> Vec<Candidate> {
    let named: Vec<(String, f64)> = found
        .iter()
        .map(|b| (name_for(b), b.width))
        .collect();

    let mut out = Vec::new();
    for (index, discovery) in found.iter().enumerate() {
        let base = &named[index].0;
        // Two breakpoints in the same band become "Tablet (md)" and
        // "Tablet (lg)", never "Tablet" and "Tablet 2".
        let collides = named
            .iter()
            .enumerate()
            .any(|(other, (name, _))| other != index && name == base);
        let name = if collides {
            match &discovery.name {
                Some(key) if key != base => format!("{base} ({key})"),
                _ => format!("{base} ({})", discovery.width as i64),
            }
        } else {
            base.clone()
        };

        let source = discovery
            .name
            .clone()
            .unwrap_or_else(|| match discovery.kind {
                Kind::Css => "css".into(),
                _ => "custom".into(),
            });

        let detail = match discovery.kind {
            Kind::Css => format!(
                "used in {} file{}",
                discovery.file_count,
                if discovery.file_count == 1 { "" } else { "s" }
            ),
            _ => source.clone(),
        };

        out.push(Candidate {
            viewport: Viewport::new(
                name.clone(),
                discovery.width,
                height_for(discovery.width, strategy, fixed),
                source.clone(),
            ),
            checked,
            file_count: discovery.file_count,
            detail: detail.clone(),
            source_file: discovery.source_file.clone(),
            line: discovery.line,
            confidence: discovery.confidence,
        });

        // Edge testing shows both sides of the transition: the last pixel of
        // the old layout and the first pixel of the new one.
        //
        // A max-width discovery gets its lower side whether edge testing is on
        // or not. `@media (max-width: 767px)` applies at 767 and stops at 768,
        // and 768 is the width recorded, so a desktop-first project whose
        // queries are all max-width had not one panel at a width where its own
        // rules actually apply. The pixel below is not an extra there: it is
        // the point.
        let always_show_below = discovery.edge == Edge::Max;
        if (edge_testing || always_show_below) && discovery.width > 1.0 {
            let below = discovery.width - 1.0;
            out.push(Candidate {
                viewport: Viewport::new(
                    if always_show_below {
                        // Not an edge case here, it is where the rule applies.
                        format!("{name} under")
                    } else {
                        format!("{name} edge")
                    },
                    below,
                    height_for(below, strategy, fixed),
                    source.clone(),
                ),
                checked,
                file_count: discovery.file_count,
                detail: if always_show_below {
                    format!("{detail}, where the max-width rule applies")
                } else {
                    format!("{detail}, edge")
                },
                source_file: discovery.source_file.clone(),
                line: discovery.line,
                confidence: discovery.confidence,
            });
        }
    }

    out.sort_by(|a, b| a.viewport.width.partial_cmp(&b.viewport.width).unwrap());
    out
}

/// Drupal writes real labels in its breakpoints files, and those are better
/// than anything a band lookup produces, so they are used as-is.
fn name_for(discovery: &BreakpointDiscovery) -> String {
    if drupal::is_drupal_source(&discovery.source) {
        if let Some(label) = &discovery.name {
            return title_case(label);
        }
    }
    band_name(discovery.width).to_string()
}

fn title_case(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => input.to_string(),
    }
}

/// Both edges of a `min` / `max` pair describe one boundary, so they collapse
/// into one panel rather than two.
pub fn collapse_edges(breakpoints: &mut Vec<BreakpointDiscovery>) {
    breakpoints.sort_by(|a, b| {
        a.width
            .partial_cmp(&b.width)
            .unwrap()
            .then_with(|| b.edge.eq(&Edge::Min).cmp(&a.edge.eq(&Edge::Min)))
    });
    breakpoints.dedup_by(|a, b| a.width == b.width);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::types::{Edge, Kind};

    fn bp(width: f64, name: Option<&str>, kind: Kind, files: usize) -> BreakpointDiscovery {
        BreakpointDiscovery {
            width,
            name: name.map(str::to_string),
            source: "test".into(),
            source_file: "test.css".into(),
            line: None,
            confidence: 0.9,
            kind,
            edge: Edge::Min,
            file_count: files,
        }
    }

    #[test]
    fn the_worked_example_from_the_plan_comes_out_exactly() {
        let widths = [640.0, 768.0, 1024.0, 1280.0, 1536.0];
        let heights: Vec<f64> = widths
            .iter()
            .map(|w| height_for(*w, HeightStrategy::DeviceRatio, 900.0))
            .collect();
        assert_eq!(heights, vec![850.0, 1020.0, 770.0, 800.0, 960.0]);
    }

    #[test]
    fn no_panel_comes_out_comically_short() {
        assert_eq!(height_for(320.0, HeightStrategy::DeviceRatio, 900.0), 690.0);
        assert_eq!(height_for(900.0, HeightStrategy::DeviceRatio, 900.0), 1200.0);
        assert_eq!(height_for(1024.0, HeightStrategy::DeviceRatio, 900.0), 770.0);
    }

    #[test]
    fn a_fixed_strategy_ignores_the_width_entirely() {
        assert_eq!(height_for(640.0, HeightStrategy::Fixed, 900.0), 900.0);
        assert_eq!(height_for(1536.0, HeightStrategy::Fixed, 900.0), 900.0);
    }

    #[test]
    fn names_come_from_the_band_the_width_falls_in() {
        assert_eq!(band_name(375.0), "Mobile");
        assert_eq!(band_name(640.0), "Large Mobile");
        assert_eq!(band_name(768.0), "Tablet");
        assert_eq!(band_name(1024.0), "Laptop");
        assert_eq!(band_name(1280.0), "Desktop");
        assert_eq!(band_name(1536.0), "Wide");
    }

    #[test]
    fn a_collision_is_disambiguated_by_the_framework_name_not_by_a_number() {
        let found = recommend(
            &[
                bp(768.0, Some("md"), Kind::Configured, 1),
                bp(900.0, Some("lg"), Kind::Configured, 1),
            ],
            HeightStrategy::DeviceRatio,
            900.0,
            false,
        );
        let names: Vec<String> = found.recommended.iter().map(|c| c.viewport.name.clone()).collect();
        assert_eq!(names, vec!["Tablet (md)", "Tablet (lg)"]);
    }

    #[test]
    fn configured_breakpoints_are_checked_and_css_ones_are_only_offered() {
        let found = recommend(
            &[
                bp(768.0, Some("md"), Kind::Configured, 1),
                bp(900.0, None, Kind::Css, 3),
            ],
            HeightStrategy::DeviceRatio,
            900.0,
            false,
        );
        assert_eq!(found.recommended.len(), 1);
        assert_eq!(found.also_found.len(), 1);
        assert!(!found.also_found[0].checked);
        assert_eq!(found.also_found[0].detail, "used in 3 files");
    }

    #[test]
    fn with_no_framework_the_most_used_css_widths_get_checked() {
        let found = recommend(
            &[
                bp(900.0, None, Kind::Css, 6),
                bp(1100.0, None, Kind::Css, 4),
                bp(1440.0, None, Kind::Css, 3),
                bp(1600.0, None, Kind::Css, 1),
            ],
            HeightStrategy::DeviceRatio,
            900.0,
            false,
        );
        assert_eq!(found.recommended.len(), 3);
        assert_eq!(found.also_found.len(), 1);
    }

    #[test]
    fn nothing_found_falls_back_to_the_standard_device_sizes_and_says_so() {
        let found = recommend(&[], HeightStrategy::DeviceRatio, 900.0, false);
        assert!(found.fallback);
        assert_eq!(found.recommended.len(), 5);
    }

    #[test]
    fn edge_testing_adds_the_pixel_below_each_breakpoint() {
        let found = recommend(
            &[bp(768.0, Some("md"), Kind::Configured, 1)],
            HeightStrategy::DeviceRatio,
            900.0,
            true,
        );
        let widths: Vec<f64> = found.recommended.iter().map(|c| c.viewport.width).collect();
        assert_eq!(widths, vec![767.0, 768.0]);
    }

    #[test]
    fn a_drupal_label_is_used_instead_of_the_band_name() {
        let mut discovery = bp(560.0, Some("narrow"), Kind::Configured, 1);
        discovery.source = "Drupal breakpoint mytheme.narrow".into();
        let found = recommend(&[discovery], HeightStrategy::DeviceRatio, 900.0, false);
        assert_eq!(found.recommended[0].viewport.name, "Narrow");
    }
}
