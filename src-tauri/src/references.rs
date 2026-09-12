//! Design references, and being honest about what comparing to one is worth.
//!
//! Break/Points owns half of "compare it to the Figma": holding the mapping
//! from panel to reference frame, so it persists and commits with the repo.
//! Fetching the design is Figma's own MCP server's job, and we are not writing
//! a Figma client.

use image::RgbaImage;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::state::Shared;
use crate::{config, project, project_file, shots, util};

/// Bind a Figma node or a local image to a viewport and write it down.
pub fn attach(
    app: &AppHandle,
    state: &Shared,
    panel: &str,
    reference: &str,
) -> Result<Value, String> {
    let id = crate::canvas::resolve_id(state, panel)
        .ok_or_else(|| format!("no panel matches \"{panel}\""))?;

    let viewports: Vec<crate::model::Viewport> = {
        let mut canvas = state.canvas.lock().unwrap();
        for target in canvas.panels.iter_mut() {
            if target.viewport.id == id {
                target.viewport.reference = Some(reference.to_string());
            }
        }
        canvas.panels.iter().map(|p| p.viewport.clone()).collect()
    };

    // Same rule as write_project_file: the URL in a committed file has to be
    // this project's, not whatever the canvas happened to be showing.
    let url = crate::tools::url_for_this_project(state);
    let path = project::write_file(app, state, &viewports, url)?;
    Ok(json!({ "panel": id, "reference": reference, "writtenTo": path }))
}

pub fn list(state: &Shared) -> Vec<Value> {
    state
        .canvas
        .lock()
        .unwrap()
        .panels
        .iter()
        .map(|panel| {
            json!({
                "panel": panel.viewport.name,
                "id": panel.viewport.id,
                "width": panel.viewport.width,
                "height": panel.viewport.height,
                "reference": panel.viewport.reference,
            })
        })
        .collect()
}

/// Screenshot a panel and compare it to its local reference image.
///
/// A live page never matches a comp exactly: fonts render differently, content
/// is real rather than lorem, and images differ. So this returns a difference
/// image and a rough figure, and says in the response that the figure is a
/// pointer rather than a verdict. The useful signal is structural.
pub async fn diff(app: &AppHandle, state: &Shared, panel: &str) -> Result<Value, String> {
    let id = crate::canvas::resolve_id(state, panel)
        .ok_or_else(|| format!("no panel matches \"{panel}\""))?;

    let reference = state
        .canvas
        .lock()
        .unwrap()
        .panels
        .iter()
        .find(|p| p.viewport.id == id)
        .and_then(|p| p.viewport.reference.clone())
        .ok_or_else(|| format!("{id} has no reference attached; use attach_reference first"))?;

    if reference.starts_with("figma:") || reference.starts_with("http") {
        return Err(format!(
            "{reference} is a Figma node. Fetch the frame with Figma's own MCP server and compare both images in context; diff_panel only handles local image references."
        ));
    }

    let root = state
        .project
        .lock()
        .unwrap()
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or("no project is open, so a relative reference cannot be resolved")?;

    let reference_path = root.join(&reference);
    if !project::inside_project(&root, &reference_path) {
        return Err("a reference has to live inside the project root".into());
    }

    let shot_path = shots::capture(app, state, &id, false).await?;

    let live = image::open(&shot_path)
        .map_err(|e| format!("could not read the screenshot: {e}"))?
        .to_rgba8();
    let comp = image::open(&reference_path)
        .map_err(|e| format!("could not read {reference}: {e}"))?
        .to_rgba8();

    let comp = resize_nearest(&comp, live.width(), live.height());
    let (difference, changed) = difference_image(&live, &comp);
    let total = (live.width() * live.height()) as f64;

    let dir = config::shot_dir(app, state)?;
    let diff_path = dir.join(format!("diff-{}-{}.png", util::slugify(&id), util::now_ms()));
    difference
        .save_with_format(&diff_path, image::ImageFormat::Png)
        .map_err(|e| format!("could not write the difference image: {e}"))?;

    Ok(json!({
        "panel": id,
        "live": shot_path,
        "reference": reference_path.to_string_lossy(),
        "differenceImage": diff_path.to_string_lossy(),
        "roughlyDifferentPercent": ((changed as f64 / total) * 1000.0).round() / 10.0,
        "readThisFirst": "The percentage is a pointer, not a verdict. Fonts, real content and images all differ from a comp, so a live page never matches one. Lead with measured differences from eval_js: a section in the wrong order, a missing element, or spacing out by 40px rather than 2px.",
    }))
}

/// Nearest neighbour on purpose: the comparison is structural, and smoothing
/// would blur exactly the edges being compared.
fn resize_nearest(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if source.width() == width && source.height() == height {
        return source.clone();
    }
    // An empty source has no pixel to sample, and `width() - 1` on it would
    // panic rather than return a blank image.
    if source.width() == 0 || source.height() == 0 {
        return RgbaImage::new(width, height);
    }
    let mut out = RgbaImage::new(width, height);
    for y in 0..height {
        let sy = (y as u64 * source.height() as u64 / height.max(1) as u64) as u32;
        for x in 0..width {
            let sx = (x as u64 * source.width() as u64 / width.max(1) as u64) as u32;
            out.put_pixel(
                x,
                y,
                *source.get_pixel(sx.min(source.width() - 1), sy.min(source.height() - 1)),
            );
        }
    }
    out
}

/// Red where the two differ by more than a threshold, faded original elsewhere.
fn difference_image(live: &RgbaImage, comp: &RgbaImage) -> (RgbaImage, u64) {
    const THRESHOLD: i32 = 28;
    let mut out = RgbaImage::new(live.width(), live.height());
    let mut changed = 0u64;

    for y in 0..live.height() {
        for x in 0..live.width() {
            let a = live.get_pixel(x, y).0;
            let b = comp.get_pixel(x, y).0;
            let delta = (0..3)
                .map(|i| (a[i] as i32 - b[i] as i32).abs())
                .max()
                .unwrap_or(0);
            if delta > THRESHOLD {
                changed += 1;
                out.put_pixel(x, y, image::Rgba([224, 87, 74, 255]));
            } else {
                let grey = (a[0] as u32 + a[1] as u32 + a[2] as u32) / 3;
                // Fade the original towards a light grey so the red reads
                // against it. Saturating on purpose: any pixel brighter than
                // 200 used to underflow here and panic the worker thread, so
                // a diff against a screenshot with a white area never
                // returned at all.
                let faded = (200 - 200u32.saturating_sub(grey).min(120)) as u8;
                out.put_pixel(x, y, image::Rgba([faded, faded, faded, 255]));
            }
        }
    }
    (out, changed)
}

/// The references recorded in `breakpoints.md`, whether or not panels are open.
pub fn from_project_file(state: &Shared) -> Vec<Value> {
    let Some(project) = state.project.lock().unwrap().clone() else {
        return Vec::new();
    };
    match project_file::read(&project.root) {
        Some(Ok(file)) => file
            .viewports
            .iter()
            .filter(|v| v.reference.is_some())
            .map(|v| json!({ "panel": v.name, "width": v.width, "reference": v.reference }))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, grey: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, image::Rgba([grey, grey, grey, 255]))
    }

    #[test]
    fn a_white_pixel_does_not_bring_the_diff_down() {
        // This is the bug. Fading the original towards a light grey did
        // `200 - grey` on an unsigned type, so anything brighter than 200
        // underflowed and panicked the worker thread. A screenshot of almost
        // any real site has white in it, so diff_panel returned nothing at all.
        let live = solid(4, 4, 255);
        let comp = solid(4, 4, 255);
        let (image, changed) = difference_image(&live, &comp);
        assert_eq!(changed, 0, "identical images differ nowhere");
        assert_eq!(image.dimensions(), (4, 4));
    }

    #[test]
    fn every_brightness_survives_the_fade() {
        for grey in 0..=255u8 {
            let frame = solid(1, 1, grey);
            let (image, _) = difference_image(&frame, &frame);
            let faded = image.get_pixel(0, 0).0[0];
            assert!(
                (80..=200).contains(&faded),
                "grey {grey} faded to {faded}, outside the intended range"
            );
        }
    }

    #[test]
    fn pixels_further_apart_than_the_threshold_are_counted() {
        let live = solid(2, 2, 0);
        let comp = solid(2, 2, 255);
        let (image, changed) = difference_image(&live, &comp);
        assert_eq!(changed, 4, "every pixel differs");
        assert_eq!(image.get_pixel(0, 0).0, [224, 87, 74, 255], "marked red");
    }

    #[test]
    fn a_difference_inside_the_threshold_is_not_a_difference() {
        // A live page never matches a comp exactly, so near-identical pixels
        // have to stay quiet or the figure is noise.
        let live = solid(2, 2, 100);
        let comp = solid(2, 2, 120);
        let (_, changed) = difference_image(&live, &comp);
        assert_eq!(changed, 0);
    }

    #[test]
    fn resizing_reaches_the_asked_for_size_without_reading_past_the_source() {
        let source = solid(3, 7, 40);
        let out = resize_nearest(&source, 10, 4);
        assert_eq!(out.dimensions(), (10, 4));
        assert_eq!(out.get_pixel(9, 3).0[0], 40, "sampled a real pixel");
    }

    #[test]
    fn an_empty_reference_gives_a_blank_image_rather_than_a_panic() {
        let source = RgbaImage::new(0, 0);
        let out = resize_nearest(&source, 5, 5);
        assert_eq!(out.dimensions(), (5, 5));
    }

    #[test]
    fn a_source_already_the_right_size_is_passed_straight_through() {
        let source = solid(6, 6, 12);
        let out = resize_nearest(&source, 6, 6);
        assert_eq!(out.get_pixel(3, 3).0[0], 12);
    }
}
