//! Panel screenshots.
//!
//! Panels are child webviews composited into one native window, so there is no
//! per-webview capture API. The window gets captured and cropped to the rect we
//! already track for each panel. That is faithful to what is on screen, which
//! is the point of a measuring instrument.
//!
//! Screenshots return a file path, not base64. Agents read image files fine and
//! it keeps megabyte blobs out of a JSON response.

use std::path::PathBuf;

use image::RgbaImage;
use tauri::{AppHandle, Manager};

use crate::canvas;
use crate::state::Shared;
use crate::{config, util};

/// Capture one panel. `full_page` renders the whole scroll height instead of
/// what is visible, at the cost of fidelity.
pub async fn capture(
    app: &AppHandle,
    state: &Shared,
    panel: &str,
    full_page: bool,
) -> Result<String, String> {
    let id = canvas::resolve_id(state, panel).ok_or_else(|| format!("no panel matches \"{panel}\""))?;

    if full_page {
        return capture_full_page(app, state, &id).await;
    }

    let (rect, name) = bring_into_view(app, state, &id).await?;

    let image = capture_window(app)?;
    let scale = window_scale(app);
    let cropped = crop(&image, &rect, scale)?;
    write_png(app, state, &id, &name, cropped)
}

/// The whole scroll height, captured a screenful at a time and stitched.
///
/// The alternative was drawing the page into a canvas element, which cannot see
/// cross-origin images and is not what the browser actually painted. This walks
/// the page instead: every tile is a real compositor capture of a real render,
/// which keeps the shot worth measuring against.
///
/// Three things make it look right rather than like six screenshots taped
/// together:
///
/// - Anything fixed or sticky is hidden after the first tile, or a sticky
///   header repeats down the whole image.
/// - The last tile cannot scroll a full screenful, so it is cropped to the
///   remainder instead of overlapping the one above it.
/// - Scroll sync is suspended, or every other panel follows this one down the
///   page and comes back somewhere else entirely.
async fn capture_full_page(
    app: &AppHandle,
    state: &Shared,
    id: &str,
) -> Result<String, String> {
    /// A page that grows as you scroll would otherwise never end.
    const MAX_TILES: usize = 20;
    /// Long enough for a lazily loaded image to arrive and paint.
    const SETTLE_MS: u64 = 220;

    let (rect, name) = bring_into_view(app, state, id).await?;
    let scale = window_scale(app);

    let measured = crate::tools::eval_js(
        state,
        id,
        "var el = document.documentElement; \
         return { total: Math.max(el.scrollHeight, document.body ? document.body.scrollHeight : 0), \
                  view: window.innerHeight, was: window.scrollY };",
    )
    .await?;
    let total_css = measured.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let view_css = measured.get("view").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let was = measured.get("was").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if view_css < 1.0 {
        return Err("the page reported no viewport height, so there is nothing to walk".into());
    }

    // Everything below is in the page's own CSS pixels; the capture is in
    // physical ones. The panel's on-screen height is the viewport times the
    // zoom, so one CSS pixel of page is `rect.height / view_css` of image.
    let per_css = rect.height / view_css;
    let (tiles, _, truncated) = stitch_plan(total_css, view_css, MAX_TILES);

    // Restore these whatever happens below.
    let sync_was_on = {
        let mut canvas = state.canvas.lock().unwrap();
        let was_on = canvas.sync_on;
        canvas.sync_on = false;
        was_on
    };

    let result = walk_and_stitch(
        app, state, id, &rect, scale, per_css, view_css, total_css, tiles, SETTLE_MS,
    )
    .await;

    {
        let mut canvas = state.canvas.lock().unwrap();
        canvas.sync_on = sync_was_on;
    }
    let _ = crate::tools::eval_js(
        state,
        id,
        &format!("window.__bpShowPinned && window.__bpShowPinned(); window.scrollTo(0, {was}); return 1;"),
    )
    .await;

    let stitched = result?;
    let path = write_png(app, state, id, &format!("{name}-full"), stitched)?;
    if truncated {
        eprintln!(
            "[breakpoints] {id} is taller than {MAX_TILES} screens; the shot stops there"
        );
    }
    Ok(path)
}

/// How tall the stitched image is, and how many screenfuls it takes.
///
/// Separate from the walking so the arithmetic can be tested. Getting it wrong
/// does not fail, it produces a torn or padded picture, which is worse.
fn stitch_plan(total_css: f64, view_css: f64, max_tiles: usize) -> (usize, f64, bool) {
    let wanted = (total_css / view_css).ceil() as usize;
    let tiles = wanted.clamp(1, max_tiles);
    let captured = (tiles as f64 * view_css).min(total_css);
    (tiles, captured, wanted > max_tiles)
}

#[allow(clippy::too_many_arguments)]
async fn walk_and_stitch(
    app: &AppHandle,
    state: &Shared,
    id: &str,
    rect: &Rect,
    scale: f64,
    per_css: f64,
    view_css: f64,
    total_css: f64,
    tiles: usize,
    settle_ms: u64,
) -> Result<RgbaImage, String> {
    let width_px = (rect.width * scale).round() as u32;
    let captured_css = (tiles as f64 * view_css).min(total_css);
    let height_px = (captured_css * per_css * scale).round().max(1.0) as u32;
    let mut out = RgbaImage::new(width_px, height_px);

    for tile in 0..tiles {
        let wanted = tile as f64 * view_css;
        // Ask the page to scroll and tell us where it actually landed. The last
        // tile always lands short, and guessing by how much is how a stitched
        // shot ends up with a repeated strip through it.
        let landed = crate::tools::eval_js(
            state,
            id,
            &format!(
                "window.scrollTo(0, {wanted}); \
                 if ({tile} === 1) {{ window.__bpHidePinned && window.__bpHidePinned(); }} \
                 return window.scrollY;"
            ),
        )
        .await?
        .as_f64()
        .unwrap_or(wanted);

        tokio::time::sleep(std::time::Duration::from_millis(settle_ms)).await;

        let shot = crop(&capture_window(app)?, rect, scale)?;
        let top_px = (landed * per_css * scale).round().max(0.0) as u32;
        if top_px >= height_px {
            break;
        }
        let rows = shot.height().min(height_px - top_px);
        for row in 0..rows {
            for column in 0..width_px.min(shot.width()) {
                out.put_pixel(column, top_px + row, *shot.get_pixel(column, row));
            }
        }
    }

    Ok(out)
}

/// Scroll the row so this panel is on screen, then report where it sits.
///
/// A panel scrolled off the right edge cannot be captured where it is, and the
/// compositor needs a moment to catch up once it has moved.
async fn bring_into_view(
    app: &AppHandle,
    state: &Shared,
    id: &str,
) -> Result<(Rect, String), String> {
    let (home_x, total_width) = {
        let canvas = state.canvas.lock().unwrap();
        let panel = canvas
            .panels
            .iter()
            .find(|p| p.viewport.id == id)
            .ok_or("panel disappeared")?;
        (panel.home_x, canvas.total_width)
    };
    let window_width = window_logical_size(app)?.0;
    let current = state.canvas.lock().unwrap().scroll_x;
    let needed = (home_x - 24.0).clamp(0.0, (total_width - window_width).max(0.0));
    if (current - needed).abs() > 1.0 {
        canvas::set_scroll(state, needed);
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    }

    let canvas = state.canvas.lock().unwrap();
    let panel = canvas
        .panels
        .iter()
        .find(|p| p.viewport.id == id)
        .ok_or("panel disappeared")?;
    Ok((
        Rect {
            x: panel.home_x - canvas.scroll_x,
            y: canvas::PANEL_TOP,
            width: panel.width,
            height: panel.height,
        },
        panel.viewport.name.clone(),
    ))
}

struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Is this window title or app name ours?
///
/// Three spellings are in play and they are all correct somewhere. The bundle
/// is `BreakPoints` because macOS will not take the slash in a filename, the
/// window title is `Break/Points`, and a dev build runs as plain lowercase
/// `breakpoints`. Matching case-sensitively against "Break" meant screenshots
/// never worked outside a release bundle.
fn is_ours(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("break/points") || lower.contains("breakpoints")
}

const PERMISSION_HINT: &str ="Grant it in System Settings > Privacy & Security > Screen & System Audio Recording, to whichever app launched Break/Points, then restart it.";

fn capture_window(app: &AppHandle) -> Result<RgbaImage, String> {
    let windows = xcap::Window::all()
        .map_err(|e| format!("could not list windows ({e}). macOS Screen Recording permission is needed once. {PERMISSION_HINT}"))?;

    let ours = windows
        .iter()
        .find(|w| w.title().map(|t| is_ours(&t)).unwrap_or(false))
        .or_else(|| {
            windows
                .iter()
                .find(|w| w.app_name().map(|a| is_ours(&a)).unwrap_or(false))
        })
        .ok_or_else(|| {
            // Without Screen Recording permission macOS still returns a window
            // list, but it holds only the system's own windows and no titles.
            // Reporting that as "window not found" sends you looking for a
            // window that is right there on the screen.
            let named = windows
                .iter()
                .filter(|w| w.title().map(|t| !t.is_empty()).unwrap_or(false))
                .count();
            if named <= 1 {
                format!(
                    "macOS is not letting Break/Points see the screen, so there is nothing to capture: it can list {named} window(s) and no titles. {PERMISSION_HINT}"
                )
            } else {
                "could not find the Break/Points window among the windows macOS listed".to_string()
            }
        })?;

    let _ = app;
    ours.capture_image()
        .map_err(|e| format!("capture failed ({e}). macOS Screen Recording permission is needed once."))
}

/// The window capture is in physical pixels and every rect we track is logical,
/// so the crop has to go through the scale factor or a Retina shot lands on the
/// wrong quarter of the image.
fn crop(image: &RgbaImage, rect: &Rect, scale: f64) -> Result<RgbaImage, String> {
    let x = (rect.x * scale).round().max(0.0) as u32;
    let y = (rect.y * scale).round().max(0.0) as u32;
    let width = (rect.width * scale).round() as u32;
    let height = (rect.height * scale).round() as u32;

    if x >= image.width() || y >= image.height() {
        return Err("panel is off screen, so there is nothing to capture".into());
    }
    let width = width.min(image.width() - x);
    let height = height.min(image.height() - y);
    if width == 0 || height == 0 {
        return Err("panel has no visible area".into());
    }

    let mut out = RgbaImage::new(width, height);
    for row in 0..height {
        for column in 0..width {
            out.put_pixel(column, row, *image.get_pixel(x + column, y + row));
        }
    }
    Ok(out)
}

fn write_png(
    app: &AppHandle,
    state: &Shared,
    id: &str,
    name: &str,
    image: RgbaImage,
) -> Result<String, String> {
    let dir = config::shot_dir(app, state)?;
    let file = dir.join(format!(
        "{}-{}-{}.png",
        util::slugify(name),
        id,
        util::now_ms()
    ));
    image
        .save_with_format(&file, image::ImageFormat::Png)
        .map_err(|e| format!("could not write {}: {e}", file.display()))?;
    Ok(file.to_string_lossy().to_string())
}

fn window_scale(app: &AppHandle) -> f64 {
    app.get_window("main")
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0)
}

fn window_logical_size(app: &AppHandle) -> Result<(f64, f64), String> {
    let window = app.get_window("main").ok_or("no main window")?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    Ok((size.width as f64 / scale, size.height as f64 / scale))
}

/// Every panel, left to right. One PNG each.
pub async fn capture_all(app: &AppHandle, state: &Shared) -> Result<Vec<PathBuf>, String> {
    let ids: Vec<String> = state
        .canvas
        .lock()
        .unwrap()
        .panels
        .iter()
        .map(|p| p.viewport.id.clone())
        .collect();

    let mut out = Vec::new();
    for id in ids {
        match capture(app, state, &id, false).await {
            Ok(path) => out.push(PathBuf::from(path)),
            Err(err) => eprintln!("[breakpoints] could not capture {id}: {err}"),
        }
    }
    if out.is_empty() {
        return Err("no panel could be captured".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gradient, so a crop that lands in the wrong place is obvious rather
    /// than merely the wrong size.
    fn ramp(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 0, 255])
        })
    }

    #[test]
    fn all_three_spellings_of_our_own_name_are_recognised() {
        // The bundle cannot hold a slash, the window title does, and a dev
        // build is lowercase. Matching "Break" case-sensitively meant
        // screenshots silently never worked outside a release bundle.
        assert!(is_ours("Break/Points"));
        assert!(is_ours("BreakPoints"));
        assert!(is_ours("breakpoints"));
        assert!(is_ours("BreakPoints — bucknell-be-the-ray"));
    }

    #[test]
    fn other_apps_are_not_mistaken_for_us() {
        assert!(!is_ours("Safari"));
        assert!(!is_ours("Menubar"));
        assert!(!is_ours("Break Time"));
        assert!(!is_ours(""));
    }

    #[test]
    fn a_crop_at_one_to_one_takes_the_rect_it_was_given() {
        let image = ramp(100, 50);
        let rect = Rect { x: 10.0, y: 5.0, width: 20.0, height: 8.0 };
        let out = crop(&image, &rect, 1.0).unwrap();
        assert_eq!(out.dimensions(), (20, 8));
        assert_eq!(out.get_pixel(0, 0).0[0], 10, "starts at x = 10");
        assert_eq!(out.get_pixel(0, 0).0[1], 5, "starts at y = 5");
    }

    #[test]
    fn a_retina_crop_goes_through_the_scale_factor() {
        // The capture is in physical pixels and every rect the app tracks is
        // logical. Skipping the conversion lands the crop on the wrong quarter
        // of the image.
        let image = ramp(200, 100);
        let rect = Rect { x: 10.0, y: 5.0, width: 20.0, height: 8.0 };
        let out = crop(&image, &rect, 2.0).unwrap();
        assert_eq!(out.dimensions(), (40, 16), "twice the pixels");
        assert_eq!(out.get_pixel(0, 0).0[0], 20, "starts at physical x = 20");
        assert_eq!(out.get_pixel(0, 0).0[1], 10, "starts at physical y = 10");
    }

    #[test]
    fn a_rect_running_off_the_edge_is_trimmed_rather_than_read_past() {
        let image = ramp(50, 50);
        let rect = Rect { x: 40.0, y: 40.0, width: 30.0, height: 30.0 };
        let out = crop(&image, &rect, 1.0).unwrap();
        assert_eq!(out.dimensions(), (10, 10), "clipped to what exists");
    }

    #[test]
    fn a_panel_scrolled_off_screen_says_so_instead_of_returning_a_sliver() {
        let image = ramp(50, 50);
        let rect = Rect { x: 80.0, y: 0.0, width: 20.0, height: 20.0 };
        assert!(crop(&image, &rect, 1.0).is_err());
    }

    #[test]
    fn a_page_two_and_a_bit_screens_tall_takes_three_screenfuls() {
        let (tiles, captured, truncated) = stitch_plan(2600.0, 1000.0, 20);
        assert_eq!(tiles, 3);
        assert_eq!(captured, 2600.0, "the image is the page's height, not three screens");
        assert!(!truncated);
    }

    #[test]
    fn a_page_exactly_one_screen_tall_takes_one() {
        let (tiles, captured, truncated) = stitch_plan(1000.0, 1000.0, 20);
        assert_eq!(tiles, 1);
        assert_eq!(captured, 1000.0);
        assert!(!truncated);
    }

    #[test]
    fn a_page_shorter_than_the_viewport_still_takes_one() {
        let (tiles, captured, _) = stitch_plan(400.0, 1000.0, 20);
        assert_eq!(tiles, 1);
        assert_eq!(captured, 400.0, "no padding below the content");
    }

    #[test]
    fn a_page_that_grows_as_you_scroll_stops_and_says_so() {
        // Infinite scroll would otherwise walk until the disk filled.
        let (tiles, captured, truncated) = stitch_plan(500_000.0, 1000.0, 20);
        assert_eq!(tiles, 20);
        assert_eq!(captured, 20_000.0);
        assert!(truncated, "the caller has to be able to say the shot is partial");
    }

    #[test]
    fn a_rect_with_no_area_is_an_error_not_an_empty_png() {
        let image = ramp(50, 50);
        let rect = Rect { x: 0.0, y: 0.0, width: 0.0, height: 10.0 };
        assert!(crop(&image, &rect, 1.0).is_err());
    }
}
