
// Alpine takes over the markup, the store owns the shared state, and this file
// is the wiring plus the four components that have local state of their own.

import Alpine from "alpinejs";

import { api } from "./ui/api.js";
import { registerIconDirective } from "./ui/icons.js";
import { registerStore } from "./ui/store.js";
import { panelMeta, panelState, panelStatusCode } from "./ui/format.js";
import { hint, keys, match as matchShortcut, SHORTCUTS } from "./ui/shortcuts.js";

registerIconDirective(Alpine);

// `$hint('fit')` in markup writes "Scale the row to fit  (\u2318F)", from the
// same table the key handler reads. A tooltip and its shortcut cannot disagree
// because there is only one of them.
Alpine.magic("hint", () => hint);
Alpine.magic("keys", () => keys);
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

  /** What the scan could not look at, so an empty result is a report rather
   *  than a dead end. */
  get limits() {
    const report = Alpine.store("bp").scan.report;
    if (!report) return "";
    const notes = [];
    if (report.truncated) {
      notes.push(
        `The walk stopped at its file limit, so this is a partial answer. ${report.scannedFiles} files were indexed.`
      );
    }
    if (report.skippedFiles) {
      notes.push(`${report.skippedFiles} files were skipped as build output, dependencies or tooling.`);
    }
    return notes.join(" ");
  },

  /** What was looked at and not counted. The log has always recorded this and
   *  nothing showed it, so an empty result was a dead end. */
  get discarded() {
    const report = Alpine.store("bp").scan.report;
    if (!report?.discarded?.length) return [];
    return [...report.discarded]
      .sort((a, b) => b.count - a.count)
      .slice(0, 4)
      .map((d) => `${d.count} ${d.reason}`);
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
  recheckOnChange: false,
  // What is actually in /Applications, asked for rather than assumed. Offering
  // a browser that is not installed fails at the moment of use instead of at
  // the moment of choosing.
  browsers: [],
  browser: null,
  // Same thesis for terminals: only the ones actually on the machine.
  terminals: [],
  terminal: null,
  // What the session button runs. Editable so anything that speaks to the
  // bridge can be started instead of Claude Code.
  agentCommand: "",
  defaultAgentCommand: "",
  // Read from the same table the key handler uses, so the list in Settings
  // cannot document a key that no longer does anything.
  shortcuts: SHORTCUTS,
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
    this.recheckOnChange = store.config.recheckOnChange ?? false;
    this.reportPrompt = store.reportPrompt;

    api.listBrowsers().then((found) => {
      this.browsers = found.browsers ?? [];
      this.browser = found.active ?? null;
    });

    api.listTerminals().then((found) => {
      this.terminals = found.terminals ?? [];
      this.terminal = found.active ?? null;
      this.agentCommand = found.command ?? "";
      this.defaultAgentCommand = found.defaultCommand ?? "";
      Alpine.store("bp").terminals = found;
    });
  },

  /** Whether macOS will ask to let the app control the chosen terminal.
   *
   *  Only iTerm does, because AppleScript is the only way it takes a command.
   *  Worth saying in advance: the prompt appears over whatever you are doing
   *  and, on an unsigned build, has to be granted again after every rebuild. */
  get terminalNeedsPermission() {
    return this.terminals.find((t) => t.id === this.terminal)?.needsPermission ?? false;
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
    // `scannedFiles` is how many files the index kept, not how many were
    // opened, and after the walk stops early the two diverge a lot. Saying
    // "indexed" keeps it true, and a truncated walk says so rather than
    // presenting a partial answer with the confidence of a complete one.
    const parts = [
      name,
      `${report.breakpoints.length} breakpoints`,
      `${report.scannedFiles} files indexed in ${report.durationMs}ms`,
    ];
    if (report.truncated) parts.push("stopped early, so this is partial");
    return parts.join(", ");
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
      recheckOnChange: this.recheckOnChange,
      browser: this.browser,
      terminal: this.terminal,
      agentCommand: this.agentCommand,
    });
    // The status bar's button reads this, so it has to be told too or it goes
    // on running the command that was replaced.
    Alpine.store("bp").loadTerminals();
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
  // Every shortcut, dispatched from the one table that also writes the
  // tooltips. A shortcut that moves moves in both places or in neither.
  //
  // Command R is Report rather than reload. Marking a problem is what this app
  // is for and what you reach for most, and reload moves to Command Shift R,
  // which is where a browser keeps its harder reload anyway.
  window.addEventListener("keydown", (event) => {
    const hit = matchShortcut(event);
    if (!hit) return;
    const store = Alpine.store("bp");

    // A shortcut that types into a field has to lose to the field. Command R
    // in the address bar should reload, not arm Report behind your cursor.
    const typing =
      event.target instanceof HTMLInputElement ||
      event.target instanceof HTMLTextAreaElement;
    if (typing && hit.id !== "focusUrl" && hit.id !== "reloadAll") return;

    event.preventDefault();
    switch (hit.id) {
      case "focusUrl":
        document.getElementById("url")?.focus();
        document.getElementById("url")?.select();
        break;
      case "reloadAll":
        api.reloadAll();
        break;
      case "toggleReport":
        store.setPicking(!store.picking);
        break;
      case "runSkill":
        store.toggleSkills();
        break;
      case "settings":
        if (store.sheet === "settings") store.closeSheet();
        else store.openSettings();
        break;
      case "accessibility":
        store.runAccessibilityAudit();
        break;
      case "zoomOut":
        store.setRowZoom(Math.min(1, store.rowZoom + 0.1));
        break;
      case "zoomIn":
        store.setRowZoom(Math.max(0, store.rowZoom - 0.1));
        break;
      case "zoomReset":
        store.setRowZoom(0);
        break;
      case "fit":
        store.setFit(!store.fitEnabled);
        break;
      case "sync":
        store.setSync(!store.syncEnabled);
        break;
      case "follow":
        store.setFollow(!store.followEnabled);
        break;
      case "alignTop":
        store.setVerticalAlign("top");
        break;
      case "alignCenter":
        store.setVerticalAlign("center");
        break;
      case "alignStretch":
        store.setVerticalAlign("stretch");
        break;
      case "copyNotes":
        store.copyReports();
        break;
      case "inspectChrome":
        store.inspectChrome();
        break;
      default:
        break;
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
