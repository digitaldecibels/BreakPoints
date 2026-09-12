// Inline SVG, one per glyph. SF Symbols are not available inside a webview and
// an icon font is a heavy dependency for nine shapes, so these are hand drawn
// against the SF Symbol names the design handoff cites.
//
// Every path uses currentColor, so hover and toggle states inherit.

const svg = (body, box = 16) =>
  `<svg viewBox="0 0 ${box} ${box}" width="100%" height="100%" fill="none" ` +
  `stroke="currentColor" stroke-width="1.4" stroke-linecap="round" ` +
  `stroke-linejoin="round" aria-hidden="true" focusable="false">${body}</svg>`;

export const icons = {
  // arrow.right
  go: svg('<path d="M3 8h9.5"/><path d="M8.5 4l4 4-4 4"/>'),

  // arrow.clockwise
  reload: svg(
    '<path d="M13 8a5 5 0 1 1-1.6-3.7"/><path d="M13 2.6V5.4H10.2"/>'
  ),

  // gearshape
  gear: svg(
    '<circle cx="8" cy="8" r="2.1"/>' +
      '<path d="M8 1.8v1.6M8 12.6v1.6M14.2 8h-1.6M3.4 8H1.8' +
      'M12.4 3.6l-1.1 1.1M4.7 11.3l-1.1 1.1M12.4 12.4l-1.1-1.1M4.7 4.7L3.6 3.6"/>'
  ),

  // doc.text, the project chip glyph
  doc: svg(
    '<path d="M4 2.2h5L12 5v8.8H4z"/><path d="M9 2.2V5h3"/>' +
      '<path d="M5.8 8h4.4M5.8 10.4h4.4"/>'
  ),

  // line.3.horizontal, the settings drag handle
  drag: svg('<path d="M3.5 5h9M3.5 8h9M3.5 11h9"/>'),

  // xmark
  close: svg('<path d="M4.2 4.2l7.6 7.6M11.8 4.2l-7.6 7.6"/>'),

  // checkmark
  check: svg('<path d="M3.4 8.3l3 3 6.2-6.6"/>', 16),

  // exclamationmark.triangle
  warn: svg(
    '<path d="M8 2.4L14.4 13.4H1.6z"/><path d="M8 6.4v3.2"/>' +
      '<path d="M8 11.6h.01" stroke-width="1.8"/>'
  ),

  // camera, the per-panel screenshot ghost button
  camera: svg(
    '<path d="M2 5.6h2.6L5.6 4h4.8l1 1.6H14v7.2H2z"/><circle cx="8" cy="9" r="2.2"/>'
  ),

  // chevron.down, the chip disclosure caret
  caret: svg('<path d="M4.5 6.5L8 10l3.5-3.5"/>'),

  // magnifyingglass, for opening the Web Inspector on a panel.
  inspect: svg('<circle cx="7" cy="7" r="3.8"/><path d="M9.9 9.9l3 3"/>'),

  // arrow.left.and.right.to.line, for zoom to fit: the row squeezed to the
  // bounds of the window.
  fit: svg(
    '<path d="M2.5 3.5v9"/><path d="M13.5 3.5v9"/><path d="M5 8h6"/>' +
    '<path d="M7 6L5 8l2 2"/><path d="M9 6l2 2-2 2"/>'
  ),

  // Two arrows moving the same way, for scroll sync. Deliberately not a
  // circular refresh glyph, which reads as "reload" and is already taken.
  sync: svg(
    '<path d="M5.5 2.5v9"/><path d="M3.5 9.5l2 2 2-2"/>' +
    '<path d="M10.5 2.5v9"/><path d="M8.5 9.5l2 2 2-2"/>'
  ),

  // arrow.up.forward.square, for following a link out of one panel into all
  // of them.
  follow: svg(
    '<path d="M9.5 2.5h4v4"/><path d="M13.5 2.5l-5 5"/>' +
    '<path d="M11.5 9v2.5a2 2 0 0 1-2 2h-5a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2H7"/>'
  ),

  // Where the panels sit vertically, as three icons in one group. Each one
  // draws the same two boxes against the same line, so the difference between
  // them is only what the group is about. The line is solid where the panels
  // are pinned to it and dashed where they are not.

  // align.vertical.top: two boxes of different heights, hanging from the top.
  alignTop: svg(
    '<path d="M2 2.5h12"/>' +
    '<rect x="3" y="5" width="4" height="8.5" rx="1"/>' +
    '<rect x="9" y="5" width="4" height="5.5" rx="1"/>'
  ),

  // align.vertical.center: the same boxes, balanced about the middle.
  alignCenter: svg(
    '<path d="M2 8h1.4M12.6 8H14"/>' +
    '<rect x="3.6" y="2.6" width="4" height="10.8" rx="1"/>' +
    '<rect x="9" y="5.2" width="4" height="5.6" rx="1"/>'
  ),

  // arrow.up.and.down.to.line: both boxes pulled to fill the space.
  alignStretch: svg(
    '<path d="M2 2.5h12"/><path d="M2 13.5h12"/>' +
    '<rect x="3.6" y="5" width="4" height="6" rx="1"/>' +
    '<rect x="9" y="5" width="4" height="6" rx="1"/>' +
    '<path d="M5.6 5V3.8M5.6 11v1.2M11 5V3.8M11 11v1.2"/>'
  ),

  // arrow.up.and.down.to.line, kept for the settings row that talks about
  // panel heights rather than about where a panel sits.
  fullHeight: svg(
    '<path d="M2.5 2.5h11"/><path d="M2.5 13.5h11"/><path d="M8 4.5v7"/>' +
    '<path d="M6 6.5L8 4.5l2 2"/><path d="M6 9.5l2 2 2-2"/>'
  ),

  // arrow.counterclockwise into a line: back to where it started. Distinct
  // from `reload`, which is a full circle and means fetch the page again.
  resetZoom: svg(
    '<path d="M3.2 8a4.8 4.8 0 1 0 1.5-3.5"/>' +
    '<path d="M2.6 3.2v2.6h2.6"/>'
  ),

  // wand.and.stars, for asking a session to run a packaged workflow.
  skill: svg(
    '<path d="M2.6 13.4L9.4 6.6"/><path d="M10.4 3.2l.6 1.4 1.4.6-1.4.6-.6 1.4-.6-1.4-1.4-.6 1.4-.6z"/>' +
    '<path d="M13.4 8.2l.4.9.9.4-.9.4-.4.9-.4-.9-.9-.4.9-.4z"/>'
  ),

  // folder, for the project root.
  folder: svg(
    '<path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.3l1.4 1.6h5.3A1.5 1.5 0 0 1 14 6.1v5.4A1.5 1.5 0 0 1 12.5 13h-9A1.5 1.5 0 0 1 2 11.5z"/>'
  ),

  // key, for the bridge token.
  key: svg(
    '<circle cx="5.5" cy="10.5" r="2.6"/><path d="M7.4 8.6L13 3"/><path d="M11 5l1.6 1.6"/>'
  ),

  // exclamationmark.bubble, for marking a problem in a panel.
  report: svg(
    '<path d="M8 2.6c3 0 5.4 2 5.4 4.6S11 11.8 8 11.8c-.6 0-1.2-.1-1.7-.2l-2.7 1.3.7-2.2C3.2 9.9 2.6 8.8 2.6 7.2 2.6 4.6 5 2.6 8 2.6z"/><path d="M8 5.1v2.3"/><path d="M8 9.1v.1"/>'
  ),
};

/** Registers `x-icon="name"` so markup stays readable HTML. */
export function registerIconDirective(Alpine) {
  Alpine.directive("icon", (el, { expression }, { evaluateLater, effect }) => {
    const get = evaluateLater(expression);
    effect(() =>
      get((name) => {
        el.innerHTML = icons[name] ?? "";
      })
    );
  });
}
