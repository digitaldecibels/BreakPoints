//! Turning what stylesheets say into px integers.
//!
//! Everything assumes a 16px root. If a project overrides `html { font-size }`
//! the rem-based widths will be off, which is why the CSS detector looks for
//! that and warns rather than quietly being wrong.

use regex::Regex;
use std::sync::OnceLock;

use super::types::Edge;

pub const ROOT_FONT_SIZE: f64 = 16.0;

/// One width condition, resolved.
///
/// `boundary` is the first pixel of the range on the far side of the condition,
/// which is the width a panel should render at. A `max-width: 767.98px` and a
/// `min-width: 768px` are the same decision written twice, and both come out of
/// here as a boundary of 768, which is what lets the pair collapse into one
/// panel instead of showing both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WidthHit {
    pub boundary: f64,
    pub edge: Edge,
    /// The number as it was written, for the log.
    pub raw: f64,
}

/// `"768px"`, `"48rem"`, `"768"`. Returns None for anything that is not a
/// length, which is how `raw` queries and keyword values get filtered out.
pub fn parse_length(input: &str) -> Option<f64> {
    let s = input.trim().trim_matches('"').trim_matches('\'').trim();
    if s.is_empty() {
        return None;
    }
    let (number, unit) = split_unit(s);
    let value: f64 = number.trim().parse().ok()?;
    let px = match unit {
        "" | "px" => value,
        "rem" | "em" => value * ROOT_FONT_SIZE,
        _ => return None,
    };
    if !(1.0..=10_000.0).contains(&px) {
        return None;
    }
    Some(px)
}

fn split_unit(s: &str) -> (&str, &str) {
    let idx = s
        .find(|c: char| c.is_ascii_alphabetic() || c == '%')
        .unwrap_or(s.len());
    (&s[..idx], &s[idx..])
}

/// A range that ends at `value` inclusive starts again at the next pixel up.
/// Bootstrap writes that next pixel as `.98` of the one below.
fn above(value: f64) -> f64 {
    if value.fract() > 0.0 {
        value.ceil()
    } else {
        value + 1.0
    }
}

fn width_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\(\s*(min|max)-width\s*:\s*([0-9.]+)\s*(px|rem|em)?\s*\)").unwrap()
    })
}

fn range_re() -> &'static Regex {
    // Media Queries Level 4: `(width >= 768px)` and `(768px <= width)`.
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\(\s*(?:width\s*(>=|<=|>|<)\s*([0-9.]+)\s*(px|rem|em)?|([0-9.]+)\s*(px|rem|em)?\s*(>=|<=|>|<)\s*width)\s*\)",
        )
        .unwrap()
    })
}

/// Turn `width <op> length` into the boundary worth rendering at.
///
/// Shared by the single-sided and double-ended forms so the two cannot drift.
fn hit_for(op: &str, px: f64) -> WidthHit {
    match op {
        ">=" => WidthHit { boundary: px.round(), edge: Edge::Min, raw: px },
        ">" => WidthHit { boundary: px.floor() + 1.0, edge: Edge::Min, raw: px },
        "<=" => WidthHit { boundary: above(px), edge: Edge::Max, raw: px },
        // `width < 768px` ends just below 768, so 768 is where the next range
        // starts, which is the pixel worth rendering.
        _ => WidthHit { boundary: px.round(), edge: Edge::Max, raw: px },
    }
}

/// `a <op> b` read from the other side: `48rem <= width` is `width >= 48rem`.
fn flip(op: &str) -> &'static str {
    match op {
        ">=" => "<=",
        ">" => "<",
        "<=" => ">=",
        _ => ">",
    }
}

/// The double-ended range: `(48rem <= width < 64rem)`.
///
/// This is the form the range syntax exists for and what PostCSS Preset Env
/// emits, and it matched neither single-sided pattern, because both of those
/// require the closing bracket straight after the operand. It was logged as
/// having no width component, which is untrue and is exactly the line somebody
/// reads to decide whether the detector is broken.
fn range_both_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\(\s*([0-9.]+)\s*(px|rem|em)?\s*(<=|<|>=|>)\s*width\s*(<=|<|>=|>)\s*([0-9.]+)\s*(px|rem|em)?\s*\)",
        )
        .unwrap()
    })
}

/// Every width condition in one media query. A query with no width component
/// returns nothing, which is how print, `prefers-reduced-motion` and retina
/// queries get dropped before they can become panels.
pub fn widths_in_query(query: &str) -> Vec<WidthHit> {
    let mut out: Vec<WidthHit> = Vec::new();

    for caps in width_re().captures_iter(query) {
        let raw = format!("{}{}", &caps[2], caps.get(3).map(|m| m.as_str()).unwrap_or(""));
        let Some(px) = parse_length(&raw) else { continue };
        if caps[1].eq_ignore_ascii_case("min") {
            out.push(WidthHit {
                boundary: px.round(),
                edge: Edge::Min,
                raw: px,
            });
        } else {
            out.push(WidthHit {
                boundary: above(px),
                edge: Edge::Max,
                raw: px,
            });
        }
    }

    for caps in range_re().captures_iter(query) {
        let (op, number, unit) = if caps.get(1).is_some() {
            (
                caps[1].to_string(),
                caps[2].to_string(),
                caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string(),
            )
        } else {
            // `768px <= width` is `width >= 768px` with the operands swapped.
            (
                flip(&caps[6]).to_string(),
                caps[4].to_string(),
                caps.get(5).map(|m| m.as_str()).unwrap_or("").to_string(),
            )
        };
        let Some(px) = parse_length(&format!("{number}{unit}")) else {
            continue;
        };
        out.push(hit_for(op.as_str(), px));
    }

    // Both ends of a double-ended range are real boundaries: the lower one is
    // where the range starts applying, the upper one where it stops.
    for caps in range_both_re().captures_iter(query) {
        let low = parse_length(&format!(
            "{}{}",
            &caps[1],
            caps.get(2).map(|m| m.as_str()).unwrap_or("")
        ));
        let high = parse_length(&format!(
            "{}{}",
            &caps[5],
            caps.get(6).map(|m| m.as_str()).unwrap_or("")
        ));
        let (Some(first), Some(second)) = (low, high) else { continue };
        let op1 = &caps[3];
        let op2 = &caps[4];

        // The first comparison is read from the other side and the second as
        // it stands, whichever direction the range is written in. Written
        // upwards that is `low <= width` then `width < high`; written
        // downwards, `high > width` then `width >= low`. Each operator stays
        // with the bound it was written against, which is what an earlier
        // attempt got wrong by assuming the first was always the lower.
        out.push(hit_for(flip(op1), first));
        out.push(hit_for(op2, second));
    }

    out.sort_by(|a, b| a.boundary.partial_cmp(&b.boundary).unwrap());
    out.dedup_by(|a, b| a.boundary == b.boundary && a.edge == b.edge);
    out
}

/// True when a stylesheet moves the root font size, which invalidates every
/// rem-based width we converted at 16px.
pub fn overrides_root_font_size(css: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?is):root\s*\{[^}]*?font-size\s*:\s*([^;}]+)|html\s*\{[^}]*?font-size\s*:\s*([^;}]+)")
            .unwrap()
    });
    let caps = re.captures(css)?;
    let value = caps
        .get(1)
        .or_else(|| caps.get(2))?
        .as_str()
        .trim()
        .to_string();
    let normalised = value.trim_end_matches(';').trim();
    if normalised == "100%" || normalised == "16px" || normalised == "1rem" {
        return None;
    }
    Some(normalised.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The form the range syntax exists for, and the one that matched nothing.
    #[test]
    fn a_double_ended_range_gives_both_boundaries() {
        let hits = widths_in_query("(48rem <= width < 64rem)");
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].boundary, 768.0);
        assert_eq!(hits[0].edge, Edge::Min);
        assert_eq!(hits[1].boundary, 1024.0);
        assert_eq!(hits[1].edge, Edge::Max);

        let pixels = widths_in_query("(400px <= width <= 700px)");
        assert_eq!(pixels.len(), 2, "{pixels:?}");
        assert_eq!(pixels[0].boundary, 400.0);
        assert_eq!(pixels[1].boundary, 701.0, "inclusive upper bound ends at 701");
    }

    /// Written the other way round, which is valid and rare.
    #[test]
    fn a_range_written_downwards_still_reads_left_to_right() {
        let hits = widths_in_query("(64rem > width >= 48rem)");
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].boundary, 768.0);
        assert_eq!(hits[0].edge, Edge::Min);
        assert_eq!(hits[1].boundary, 1024.0);
        assert_eq!(hits[1].edge, Edge::Max);
    }

    /// The single-sided forms have to keep working exactly as they did.
    #[test]
    fn single_sided_ranges_are_unchanged() {
        assert_eq!(widths_in_query("(width >= 48rem)")[0].boundary, 768.0);
        assert_eq!(widths_in_query("(768px <= width)")[0].boundary, 768.0);
        assert!(widths_in_query("(prefers-reduced-motion: reduce)").is_empty());
    }

    fn boundaries(query: &str) -> Vec<f64> {
        widths_in_query(query).into_iter().map(|h| h.boundary).collect()
    }

    #[test]
    fn lengths_convert_from_every_unit_that_is_a_width() {
        assert_eq!(parse_length("768px"), Some(768.0));
        assert_eq!(parse_length("48rem"), Some(768.0));
        assert_eq!(parse_length("'640px'"), Some(640.0));
        assert_eq!(parse_length("1024"), Some(1024.0));
        assert_eq!(parse_length("100vw"), None);
        assert_eq!(parse_length("infinite"), None);
    }

    #[test]
    fn queries_without_a_width_are_dropped_entirely() {
        assert!(widths_in_query("print").is_empty());
        assert!(widths_in_query("(prefers-reduced-motion: reduce)").is_empty());
        assert!(widths_in_query("(-webkit-min-device-pixel-ratio: 2)").is_empty());
        assert!(widths_in_query("(orientation: landscape)").is_empty());
    }

    #[test]
    fn a_bootstrap_style_pair_lands_on_the_same_boundary() {
        assert_eq!(boundaries("(max-width: 767.98px)"), vec![768.0]);
        assert_eq!(boundaries("screen and (min-width: 768px)"), vec![768.0]);
        assert_eq!(boundaries("(max-width: 767px)"), vec![768.0]);
    }

    #[test]
    fn both_media_query_syntaxes_are_understood() {
        assert_eq!(boundaries("(width >= 48rem)"), vec![768.0]);
        assert_eq!(boundaries("(768px <= width)"), vec![768.0]);
        assert_eq!(boundaries("(width < 768px)"), vec![768.0]);
    }

    #[test]
    fn a_range_query_yields_both_of_its_boundaries() {
        assert_eq!(
            boundaries("(min-width: 768px) and (max-width: 1023px)"),
            vec![768.0, 1024.0]
        );
    }

    #[test]
    fn a_moved_root_font_size_is_noticed_but_the_usual_values_are_not() {
        assert_eq!(
            overrides_root_font_size("html { font-size: 62.5%; }"),
            Some("62.5%".into())
        );
        assert_eq!(overrides_root_font_size("html { font-size: 100%; }"), None);
        assert_eq!(overrides_root_font_size("body { font-size: 14px; }"), None);
    }
}
