//! Before and after, across every width at once.
//!
//! This is the thing a row of panels can do that a browser cannot. Take a
//! picture of every width, make a change, take them again, and say which
//! widths moved. A change that fixes 1024 and breaks 375 is the failure this
//! app exists to catch, and until now catching it meant looking at six panels
//! and remembering what they used to be.
//!
//! The comparison is deliberately blunt: how many pixels differ, as a
//! proportion. It answers "did anything move here" rather than "what moved",
//! which is the question you want first when you have six widths and no idea
//! which one to look at. The difference image says where.

use std::path::PathBuf;

use serde::Serialize;
use tauri::AppHandle;

use crate::state::Shared;
use crate::{config, shots, tools, util};

/// What was captured for one panel, and where it was put.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    pub panel: String,
    pub panel_name: String,
    pub width: f64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Taken {
    pub shots: Vec<Shot>,
    pub failed: Vec<String>,
    pub summary: String,
}

/// One width, before and after.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub panel: String,
    pub panel_name: String,
    pub width: f64,
    pub changed_percent: f64,
    pub moved: bool,
    /// Where the difference image was written, when anything differed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difference_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compared {
    pub widths: Vec<Comparison>,
    pub moved: Vec<f64>,
    pub unchanged: Vec<f64>,
    pub summary: String,
}

/// Anything below this is compression noise and antialiasing, not a change.
const NOISE_FLOOR: f64 = 0.1;

fn baseline_dir(app: &AppHandle, state: &Shared) -> Result<PathBuf, String> {
    let dir = config::shot_dir(app, state)?.join("baseline");
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Capture every panel and keep the pictures as the state to compare against.
///
/// One capture per panel, in order, because the window can only be captured
/// once at a time and a row of seven takes a moment.
pub async fn take(app: &AppHandle, state: &Shared) -> Result<Taken, String> {
    let panels = tools::list_panels(state);
    if panels.is_empty() {
        return Err("no panels are open, so there is nothing to take a baseline of".into());
    }
    let dir = baseline_dir(app, state)?;

    let mut shots = Vec::new();
    let mut failed = Vec::new();
    for panel in &panels {
        match shots::capture(app, state, &panel.id, false).await {
            Ok(path) => {
                let kept = dir.join(format!("{}.png", util::slugify(&panel.id)));
                // Moved rather than copied, so a baseline does not leave a
                // second picture of every width in the screenshots folder.
                if let Err(err) = std::fs::rename(&path, &kept) {
                    failed.push(format!("{}: could not keep the picture: {err}", panel.name));
                    continue;
                }
                shots.push(Shot {
                    panel: panel.id.clone(),
                    panel_name: panel.name.clone(),
                    width: panel.width,
                    path: kept.to_string_lossy().to_string(),
                });
            }
            Err(err) => failed.push(format!("{}: {err}", panel.name)),
        }
    }

    // One reason, said once. Every panel fails for the same reason nearly
    // every time, and seven copies of a paragraph is not a report.
    let summary = if failed.is_empty() {
        format!("{} widths captured", shots.len())
    } else {
        let reasons: std::collections::BTreeSet<&str> = failed
            .iter()
            .map(|line| line.split_once(": ").map(|(_, rest)| rest).unwrap_or(line))
            .collect();
        if reasons.len() == 1 {
            format!(
                "{} widths captured, {} could not be, all for the same reason: {}",
                shots.len(),
                failed.len(),
                reasons.iter().next().unwrap()
            )
        } else {
            format!(
                "{} widths captured, {} could not be: {}",
                shots.len(),
                failed.len(),
                failed.join("; ")
            )
        }
    };
    Ok(Taken { shots, failed, summary })
}

/// Capture every panel again and say which widths moved.
pub async fn compare(app: &AppHandle, state: &Shared) -> Result<Compared, String> {
    let panels = tools::list_panels(state);
    if panels.is_empty() {
        return Err("no panels are open, so there is nothing to compare".into());
    }
    let dir = baseline_dir(app, state)?;

    let mut widths = Vec::new();
    for panel in &panels {
        let before = dir.join(format!("{}.png", util::slugify(&panel.id)));
        if !before.is_file() {
            widths.push(Comparison {
                panel: panel.id.clone(),
                panel_name: panel.name.clone(),
                width: panel.width,
                changed_percent: 0.0,
                moved: false,
                difference_image: None,
                error: Some("no baseline for this width; take one first".into()),
            });
            continue;
        }

        let after = match shots::capture(app, state, &panel.id, false).await {
            Ok(path) => path,
            Err(err) => {
                widths.push(Comparison {
                    panel: panel.id.clone(),
                    panel_name: panel.name.clone(),
                    width: panel.width,
                    changed_percent: 0.0,
                    moved: false,
                    difference_image: None,
                    error: Some(err),
                });
                continue;
            }
        };

        match difference(&before, &PathBuf::from(&after), app, state, &panel.id) {
            Ok((percent, diff_path)) => widths.push(Comparison {
                panel: panel.id.clone(),
                panel_name: panel.name.clone(),
                width: panel.width,
                changed_percent: percent,
                moved: percent > NOISE_FLOOR,
                difference_image: if percent > NOISE_FLOOR { Some(diff_path) } else { None },
                error: None,
            }),
            Err(err) => widths.push(Comparison {
                panel: panel.id.clone(),
                panel_name: panel.name.clone(),
                width: panel.width,
                changed_percent: 0.0,
                moved: false,
                difference_image: None,
                error: Some(err),
            }),
        }
    }

    let moved: Vec<f64> = widths.iter().filter(|w| w.moved).map(|w| w.width).collect();
    let unchanged: Vec<f64> = widths
        .iter()
        .filter(|w| !w.moved && w.error.is_none())
        .map(|w| w.width)
        .collect();

    let summary = if unchanged.is_empty() && moved.is_empty() {
        let why = widths
            .iter()
            .find_map(|w| w.error.clone())
            .unwrap_or_else(|| "no widths could be compared".into());
        format!("nothing could be compared: {why}")
    } else if moved.is_empty() {
        format!("nothing moved at any of the {} widths compared", unchanged.len())
    } else {
        format!(
            "{} of {} widths moved: {}",
            moved.len(),
            moved.len() + unchanged.len(),
            moved
                .iter()
                .map(|w| format!("{}px", w.round()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    Ok(Compared { widths, moved, unchanged, summary })
}

/// How much two pictures of the same panel differ, and where.
fn difference(
    before: &PathBuf,
    after: &PathBuf,
    app: &AppHandle,
    state: &Shared,
    id: &str,
) -> Result<(f64, String), String> {
    let old = image::open(before)
        .map_err(|e| format!("could not read the baseline: {e}"))?
        .to_rgba8();
    let new = image::open(after)
        .map_err(|e| format!("could not read the new picture: {e}"))?
        .to_rgba8();

    // A resize would compare two different things, so a size change is
    // reported as a change rather than smoothed away.
    if old.dimensions() != new.dimensions() {
        return Ok((100.0, format!("{} changed size", after.display())));
    }

    let (difference, changed) = crate::references::difference_image(&old, &new);
    let total = (old.width() * old.height()) as f64;
    let percent = ((changed as f64 / total) * 1000.0).round() / 10.0;

    if percent <= NOISE_FLOOR {
        return Ok((percent, String::new()));
    }

    let dir = config::shot_dir(app, state)?;
    let path = dir.join(format!("changed-{}-{}.png", util::slugify(id), util::now_ms()));
    difference
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|e| format!("could not write the difference image: {e}"))?;
    Ok((percent, path.to_string_lossy().to_string()))
}
