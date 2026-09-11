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
