// Every number on screen is true. These helpers exist so that stays cheap.

export const px = (value) => Math.round(value);

/** `md · 768×1020 · 42%`, with hair spaces around the middle dots. */
export function panelMeta(panel) {
  const parts = [];
  if (panel.source && panel.source !== "custom") parts.push(panel.source);
  parts.push(`${px(panel.width)}×${px(panel.height)}`);
  if (panel.scale < 0.999) parts.push(`${Math.round(panel.scale * 100)}%`);
  return parts.join(" · ");
}

export function dimensions(width, height) {
  return `${px(width)} × ${px(height)}`;
}

/** A folder path shortened to something that fits a chip. */
export function folderName(path) {
  if (!path) return "";
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] || path;
}

export function panelState(panel) {
  if (!panel.state) return "loading";
  return typeof panel.state === "string" ? panel.state : panel.state.state;
}

export function panelStatusCode(panel) {
  return panel.state && typeof panel.state === "object" ? panel.state.code : null;
}
