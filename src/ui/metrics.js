// The chrome's vertical rhythm. These four numbers also live in
// src-tauri/src/canvas.rs; they are the contract between the chrome layer and
// the Rust code that positions panels, so they have to agree exactly.
export const TOOLBAR_H = 56;
/** The pan strip, along the bottom edge rather than under the toolbar. */
export const STRIP_H = 12;
export const GAP_ABOVE_LABELS = 16;
export const LABEL_H = 32;
export const GAP_BELOW_LABELS = 16;

/** Y coordinate of the top of every panel. */
export const PANEL_TOP =
  TOOLBAR_H + GAP_ABOVE_LABELS + LABEL_H + GAP_BELOW_LABELS;

/** Y coordinate of the top of the label strip. */
export const LABEL_TOP = TOOLBAR_H + GAP_ABOVE_LABELS;

export const PANEL_GAP = 24;
export const OUTER_MARGIN = 24;
export const BOTTOM_MARGIN = 24;
