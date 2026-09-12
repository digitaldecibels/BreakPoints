
// Alpine takes over the markup, the store owns the shared state, and this file
// is the wiring plus the four components that have local state of their own.

import Alpine from "alpinejs";

import { api } from "./ui/api.js";
import { registerIconDirective } from "./ui/icons.js";
import { registerStore } from "./ui/store.js";
import { panelMeta, panelState, panelStatusCode } from "./ui/format.js";

registerIconDirective(Alpine);
registerStore(Alpine);

Alpine.data("toolbar", () => ({
  get profileName() {
    const store = Alpine.store("bp");
    const profile = store.profiles.find((p) => p.active);
    return profile?.name ?? "Default";
  },
  openProjectFile: () => api.openProjectFile().catch(() => {}),
}));

Alpine.data("canvas", () => ({
  ticking: false,

  meta: panelMeta,

  failed() {
    return Alpine.store("bp").panels.filter((panel) => panelState(panel) === "failed");
  },

  isLoading(panel) {
    return panelState(panel) === "loading";
  },

  failureLine(panel) {
    const code = panelStatusCode(panel);
    return code ? `Failed to load · ${code}` : "Failed to load";
  },

  // Panels have to track the scroll bar exactly, so this does the least work
  // it can and never animates.
  onScroll() {
    // A wheel pan moves the panels first and then sets scrollLeft to match,
    // so sending that position back to Rust would be an echo, not an input.
    if (Alpine.store("bp").echoingScroll) return;
    if (this.ticking) return;
    this.ticking = true;
    requestAnimationFrame(() => {
      this.ticking = false;
      Alpine.store("bp").setScroll(this.$refs.scroller.scrollLeft);
    });
  },

  // A sideways trackpad swipe over the chrome. Panels cover most of the
  // window and handle their own (see the injected script in canvas.rs); this
  // catches the toolbar, the label strip and the bare canvas around a panel.
  onWheel(event) {
    if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    event.preventDefault();
    const store = Alpine.store("bp");
    const scroller = this.$refs.scroller;
    if (!scroller) return;
    const max = Math.max(0, scroller.scrollWidth - scroller.clientWidth);
    const next = Math.min(max, Math.max(0, store.scrollX + event.deltaX));
    if (Math.abs(next - store.scrollX) < 0.5) return;
    scroller.scrollLeft = next;
    store.setScroll(next);
  },
}));

Alpine.data("scanSheet", () => ({
  expanded: [],

  get heading() {
    const store = Alpine.store("bp");
    if (store.scan.fallback) return "No framework config found.";
    return store.projectName ? `Scanning ${store.projectName}` : "Scanning";
  },

  get recommendedLabel() {
    const store = Alpine.store("bp");
    return store.scan.fallback
      ? "Standard device sizes"
      : "Recommended testing environment";
  },

  toggle(index) {
    const at = this.expanded.indexOf(index);
    if (at === -1) this.expanded.push(index);
    else this.expanded.splice(at, 1);
  },

  renderLog(entry) {
    if (typeof entry === "string") return entry;
    switch (entry.event) {
      case "parse.failure":
        return `${entry.file}${entry.line ? `:${entry.line}` : ""}: ${entry.reason} (${entry.step})\n    ${entry.snippet ?? ""}`;
      case "value.discarded":
        return `${entry.file}: discarded ${entry.value}, ${entry.reason}`;
      case "file.skip":
        return `${entry.file}: skipped, ${entry.reason}`;
      case "file.read":
        return `${entry.file}: read ${entry.bytes} bytes`;
      case "devurl.candidate":
        return `${entry.url} from ${entry.source} (confidence ${entry.confidence})`;
      case "detector.start":
        return `\n${entry.name}`;
      case "detector.end":
        return `${entry.name} finished: ${entry.breakpointsFound} found in ${entry.tookMs}ms`;
      case "note":
        return entry.message;
      default:
        return JSON.stringify(entry);
    }
  },
}));

Alpine.data("settingsSheet", () => ({
  tabs: [
    { id: "viewports", label: "Viewports" },
    { id: "project", label: "Project" },
    { id: "general", label: "General" },
    { id: "bridge", label: "Agent bridge" },
    { id: "about", label: "About" },
  ],
  rows: [],
  dragFrom: null,
  heightStrategy: "device-ratio",
  fixedHeight: 900,
  edgeTesting: false,
  uniformFit: false,
  // What is actually in /Applications, asked for rather than assumed. Offering
  // a browser that is not installed fails at the moment of use instead of at
  // the moment of choosing.
  browsers: [],
  browser: null,
  reportPrompt: "",
  promptSaved: false,

  init() {
    this.seed();

    // The sheet is hidden with `x-show`, which leaves it in the DOM, so this
    // component is created once at page load and never again. At that moment
    // `boot()` has not resolved and the store holds no panels yet, so seeding
    // only in `init` left the Viewports tab permanently empty: it captured an
    // empty list before the row existed and nothing ever went back for it.
    //
    // Re-seed every time the sheet opens instead. That also fixes the same
    // staleness in the preferences below it, which were reading a config that
    // had not loaded either.
    this.$watch("$store.bp.sheet", (sheet) => {
      if (sheet === "settings") this.seed();
    });

    // Seeding on open is not enough on its own. The sheet can already be open
    // when the page reloads, and then `seed` runs before `boot` has resolved,
    // fills the textarea from an empty store, and the watch above never fires
    // because the sheet never changed. So follow the value itself, and adopt
    // it whenever the box has not been typed in. `previous` is what makes that
    // safe: a box still holding the old saved text is untouched, a box holding
    // anything else is an edit in progress and is left alone.
    this.$watch("$store.bp.reportPrompt", (value, previous) => {
      if (!this.reportPrompt || this.reportPrompt === previous) {
        this.reportPrompt = value;
      }
    });
  },

  /** Fill the form from the app's current state. Safe to call repeatedly. */
  seed() {
    const store = Alpine.store("bp");
    this.rows = store.panels.length
      ? store.panels.map((panel) => ({
          id: panel.id,
          name: panel.name,
          width: Math.round(panel.width),
          height: Math.round(panel.height),
          source: panel.source,
          isNew: false,
        }))
      : store.scan.recommended.map((candidate) => ({ ...candidate, isNew: false }));

    this.heightStrategy = store.config.heightStrategy ?? "device-ratio";
    this.fixedHeight = store.config.fixedHeight ?? 900;
    this.edgeTesting = store.config.edgeTesting ?? false;
    this.uniformFit = store.config.fitMode === "uniform";
    this.reportPrompt = store.reportPrompt;

    api.listBrowsers().then((found) => {
      this.browsers = found.browsers ?? [];
      this.browser = found.active ?? null;
    });
  },

  /** Whether the chosen browser can be told what width to open at. */
  get browserCanSize() {
    return this.browsers.find((b) => b.id === this.browser)?.canSize ?? true;
  },

  get detectedSummary() {
    const report = Alpine.store("bp").scan.report;
    if (!report) return "";
    const framework = report.frameworks[0];
    const name = framework
      ? `${framework.framework}${framework.version ? ` v${framework.version}` : ""}`
      : "no framework";
    return `${name}, ${report.breakpoints.length} breakpoints, ${report.scannedFiles} files read in ${report.durationMs}ms`;
  },

  addRow() {
    // A new row arrives already in edit state, with the cursor in the name.
    this.rows.push({
      id: `custom-${Date.now()}`,
      name: "",
      width: 1024,
      height: 770,
      source: "custom",
      isNew: true,
    });
    this.$nextTick(() => {
      const inputs = this.$el.querySelectorAll('input[type="text"]');
      inputs[inputs.length - 1]?.focus();
    });
  },

  // Retyping a width should move the height with it, unless the height was
  // typed by hand.
  onWidthChange(row) {
    if (this.heightStrategy !== "device-ratio") return;
    row.height = heightFor(row.width);
  },

  move(from, to) {
    if (from === null || from === to) return;
    const [moved] = this.rows.splice(from, 1);
    this.rows.splice(to, 0, moved);
    this.dragFrom = null;
  },

  resetToDetected() {
    this.rows = Alpine.store("bp").scan.recommended.map((candidate) => ({
      ...candidate,
      isNew: false,
    }));
  },

  /** Whether the textarea differs from what is actually saved. Drives both the
   *  Save button and the "Unsaved changes" line, so the two can never disagree. */
  get promptDirty() {
    return this.reportPrompt !== Alpine.store("bp").reportPrompt;
  },

  /** Saved on its own, because a textarea should not trigger a relayout on
   *  every keystroke the way the layout preferences do. An empty value means
   *  "use the default", which is what the Reset button sends.
   *
   *  Rust resolves an empty string back to the default and hands the resolved
   *  text back, so after Reset the textarea fills with the default rather than
   *  going blank and leaving you to wonder what will be sent. */
  async savePrompt() {
    const store = Alpine.store("bp");
    await api.setPreferences({ reportPrompt: this.reportPrompt });
    const snapshot = await api.state();
    store.config = snapshot.config;
    store.reportPrompt = snapshot.reportPrompt;
    this.reportPrompt = snapshot.reportPrompt;
    this.promptSaved = true;
    store.say("Report instruction saved.");
  },

  async savePreferences() {
    const store = Alpine.store("bp");
    store.config = await api.setPreferences({
      heightStrategy: this.heightStrategy,
      fixedHeight: this.fixedHeight,
      edgeTesting: this.edgeTesting,
      fitMode: this.uniformFit ? "uniform" : "height",
      browser: this.browser,
    });
  },

  async save() {
    const viewports = this.rows
      .filter((row) => row.width > 0 && row.height > 0)
      .map((row) => ({
        id: row.id,
        name: row.name?.trim() || `${Math.round(row.width)}px`,
        width: Number(row.width),
        height: Number(row.height),
        source: row.source ?? "custom",
        enabled: true,
      }));
    await Alpine.store("bp").applyViewports(viewports);
  },
}));

/// Mirrors generate.rs, so a height typed here matches one the scanner derives.
function heightFor(width) {
  const ratio = width <= 480 ? 2.16 : width <= 900 ? 1.33 : width <= 1279 ? 0.75 : 0.625;
  return Math.max(640, Math.round((width * ratio) / 10) * 10);
}

window.Alpine = Alpine;
Alpine.start();

document.addEventListener("DOMContentLoaded", () => {
  // Focus the URL bar and reload every panel, the two shortcuts a browser
  // trains you to expect.
  window.addEventListener("keydown", (event) => {
    if (!event.metaKey) return;
    if (event.key === "l") {
      event.preventDefault();
      document.getElementById("url")?.focus();
      document.getElementById("url")?.select();
    }
    if (event.key === "r") {
      event.preventDefault();
      api.reloadAll();
    }
    // Cmd+Alt+I on the chrome itself, the way a browser does it. Without this
    // the app's own console is unreachable, which is how a permissions failure
    // once read as a completely dead window.
    if (event.altKey && (event.key === "i" || event.key === "\u02c6")) {
      event.preventDefault();
      Alpine.store("bp").inspectChrome();
    }
  });

  // Arrow keys pan the canvas, when the focus is not in a field.
  window.addEventListener("keydown", (event) => {
    if (event.target instanceof HTMLInputElement) return;
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    const store = Alpine.store("bp");
    if (!store.hasPanels) return;
    event.preventDefault();
    const step = event.shiftKey ? 400 : 120;
    const next = Math.max(
      0,
      Math.min(store.totalWidth, store.scrollX + (event.key === "ArrowRight" ? step : -step))
    );
    const scroller = document.querySelector('[role="scrollbar"]');
    if (scroller) scroller.scrollLeft = next;
    else store.setScroll(next);
  });
});

Alpine.store("bp")
  .boot()
  .catch((error) => console.error("[breakpoints] boot failed:", error));
