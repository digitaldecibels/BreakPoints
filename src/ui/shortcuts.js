// Every keyboard shortcut, in one place.
//
// One table rather than a switch in the handler and a string in each tooltip,
// because those two drift: a shortcut gets moved and the tooltip keeps telling
// people the old key for months. Here the handler and the label are the same
// row, so they cannot disagree.
//
// `mac` is what the label shows. `key` is what the event carries, lowercased.
// Option and Shift change what `event.key` is on macOS, so anything using them
// lists every spelling it can arrive as.

export const SHORTCUTS = [
  {
    id: "focusUrl",
    mac: "\u2318L",
    key: ["l"],
    label: "Focus the address bar",
  },
  {
    id: "toggleReport",
    // Command R, because marking a problem is the thing this app is for and
    // the thing you reach for most. Reload moves to Command Shift R, which is
    // where a browser puts its harder reload anyway.
    mac: "\u2318R",
    key: ["r"],
    label: "Mark a problem",
  },
  {
    id: "reloadAll",
    mac: "\u21e7\u2318R",
    key: ["r", "R"],
    shift: true,
    label: "Reload every panel",
  },
  {
    id: "runSkill",
    mac: "\u2318K",
    key: ["k"],
    label: "Run a skill",
  },
  {
    id: "settings",
    mac: "\u2318,",
    key: [","],
    label: "Settings",
  },
  {
    id: "accessibility",
    // A for accessibility, and Shift because it runs axe-core in every panel
    // and takes a moment, so it should not be one key away from a typo.
    mac: "\u21e7\u2318A",
    key: ["a", "A"],
    shift: true,
    label: "Accessibility at every width",
  },
  {
    id: "zoomOut",
    mac: "\u2318-",
    key: ["-"],
    label: "Zoom the row out",
  },
  {
    id: "zoomIn",
    mac: "\u2318+",
    key: ["+", "="],
    label: "Zoom the row in",
  },
  {
    id: "zoomReset",
    mac: "\u23180",
    key: ["0"],
    label: "Back to actual size",
  },
  {
    id: "fit",
    mac: "\u2318F",
    key: ["f"],
    label: "Scale the row to fit",
  },
  {
    id: "sync",
    mac: "\u2318U",
    key: ["u"],
    label: "Scroll every panel together",
  },
  {
    id: "follow",
    mac: "\u2318J",
    key: ["j"],
    label: "Follow links in every panel",
  },
  {
    id: "alignTop",
    mac: "\u23181",
    key: ["1"],
    label: "Panels at the top",
  },
  {
    id: "alignCenter",
    mac: "\u23182",
    key: ["2"],
    label: "Panels centred",
  },
  {
    id: "alignStretch",
    mac: "\u23183",
    key: ["3"],
    label: "Panels stretched to the window",
  },
  {
    id: "copyNotes",
    mac: "\u21e7\u2318C",
    key: ["c", "C"],
    shift: true,
    label: "Copy the waiting notes",
  },
  {
    id: "inspectChrome",
    mac: "\u2325\u2318I",
    // Option changes what the key is: on a US layout Option-i produces a dead
    // circumflex rather than an "i".
    key: ["i", "\u02c6"],
    alt: true,
    label: "Inspect the app itself",
  },
];

const BY_ID = Object.fromEntries(SHORTCUTS.map((s) => [s.id, s]));

/** The label for a tooltip: "Mark a problem  \u2318R". */
export function hint(id, text) {
  const shortcut = BY_ID[id];
  if (!shortcut) return text ?? "";
  return text ? `${text}  (${shortcut.mac})` : `${shortcut.label}  (${shortcut.mac})`;
}

/** Just the keys, for a menu that shows them in their own column. */
export function keys(id) {
  return BY_ID[id]?.mac ?? "";
}

/** Which shortcut, if any, an event is. */
export function match(event) {
  if (!event.metaKey) return null;
  return (
    SHORTCUTS.find((s) => {
      if (!s.key.includes(event.key)) return false;
      // Shift and Option have to match exactly, or Command Shift R also fires
      // Command R and the row reloads every time you toggle Report.
      if (Boolean(s.shift) !== event.shiftKey) return false;
      if (Boolean(s.alt) !== event.altKey) return false;
      return true;
    }) ?? null
  );
}
