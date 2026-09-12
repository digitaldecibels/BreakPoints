// The shared state, and the actions that change it.
//
// Anything two parts of the chrome both need lives here. Everything else is
// local x-data on the element that owns it.

import { api, listen } from "./api.js";
import { folderName, panelState } from "./format.js";
import { LABEL_TOP } from "./metrics.js";

/** Scan rows arrive as their detectors finish; the sheet reveals them 60ms apart. */
const ROW_STAGGER = 60;

/** What a running tool is doing, in words rather than in its own name.
 *
 * The status bar is read by somebody looking at their layout, not by somebody
 * reading an API. Anything not named here falls back to its own name with the
 * underscores taken out, which stays readable for tools added later. */
const TOOL_WORDS = {
  audit_all: "Checking every width",
  audit_accessibility: "Running the accessibility checks",
  verify_breakpoints: "Asking the page what it changes at",
  screenshot_panel: "Taking a screenshot",
  screenshot_all: "Photographing every width",
  take_baseline: "Taking a baseline",
  compare_to_baseline: "Comparing against the baseline",
  diff_panel: "Comparing against the design",
  scan_breakpoints: "Rescanning the project",
  detect_project: "Reading the project",
  write_project_file: "Writing breakpoints.md",
  navigate: "Pointing the row somewhere new",
  reload: "Reloading the row",
  eval_js: "Measuring a panel",
  eval_chrome: "Measuring the app itself",
  get_dom: "Reading the markup",
  get_console: "Reading the console",
  set_viewport: "Resizing a panel",
  load_profile: "Switching profile",
};

export function registerStore(Alpine) {
  Alpine.store("bp", {
    ready: false,

    // Project
    project: null,
    projectDriven: false,
    projectFileError: null,
    sourceChanged: null,

    // Canvas
    url: "",
    urlStatus: "unknown",
    /** True while the URL box has focus, so nothing rewrites it under the cursor. */
    urlEditing: false,
    /** Where the panels actually are, which is what Escape puts back. */
    canvasUrl: "",
    panels: [],
    totalWidth: 0,
    scrollX: 0,
    /** True while the strip is being moved to match a wheel pan Rust already did. */
    echoingScroll: false,
    panelTop: 132,
    // Which panel's Web Inspector is open. While this is set the other panels
    // are hidden, because the inspector docks into the window and takes it.
    inspecting: null,
    // Top of the label strip band. The notice is positioned against this, and
    // it is the one horizontal band a panel never covers.
    labelTop: LABEL_TOP,
    fitEnabled: true,
    syncEnabled: true,
    /** A link followed in one panel is followed in all of them. */
    followEnabled: true,
    /** Clicking in a panel describes an element instead of following a link. */
    picking: false,
    // Every panel as tall as the window allows. Overrides Fit, which asks for
    // the opposite, so the two are kept out of step here as well as in Rust.
    /** Where the panels sit vertically: "top", "center" or "stretch". */
    verticalAlign: "top",
    /** How far the whole row is zoomed out, 0 to 1. Zero is actual size. */
    rowZoom: 0,
    /** The skills installed on this machine and in this project, with the
     *  description each one's author wrote. Read from disk, so a skill written
     *  five minutes ago is in the menu. */
    skills: [],
    skillsOpen: false,
    // Where this project's screenshots land. Resolved by Rust, so it is a real
    // path rather than "the default", and shown so nobody has to go looking.
    shotDir: "",
    /** How many notes are waiting to be collected. Owned by Rust and pushed
     *  here on `reports:changed`, never counted in the chrome. */
    reportCount: 0,
    // The standing instruction sent with every note, resolved to the default
    // by Rust so this is always real text.
    reportPrompt: "",
    // Which agent session notes are being addressed to, or null if none has
    // claimed. Shown in the toolbar so it is never a guess where a note went.
    reportOwner: null,
    /** True while a session is holding a socket open, waiting for the next
     *  note. That is the difference between a note being delivered and a note
     *  joining a queue, so it changes both what the toolbar says and whether
     *  the clipboard is touched. */
    reportWatching: false,
    /** The last note written, and what became of it. The status bar along the
     *  bottom shows this, because until now the only sign a note had gone
     *  anywhere was a notice that faded after five seconds, and the answer to
     *  "did that send" was to write another one. Null until the first note. */
    lastReport: null,
    /** True while the chrome is collecting the queue itself, so a queue
     *  reaching zero because of the copy button is not mistaken for a session
     *  having been handed the notes. */
    collecting: false,
    /** Which terminals are installed and what the session button runs. Read
     *  once at boot, because installing a terminal mid-session is not a case
     *  worth re-checking for. */
    terminals: { terminals: [], active: null, command: "", defaultCommand: "" },
    startingSession: false,
    /** The last thing a session said back about a note, so the answer arrives
     *  in the window rather than only in a terminal you are not looking at.
     *  Kept by Rust as well, so a chrome reload does not lose it. */
    lastReply: null,
    /** The last ten notes with what became of each, newest first, kept by Rust
     *  so a chrome reload does not lose the sitting. Drawn as the list that
     *  drops up from the status bar and as the dots on the panel labels. */
    recentNotes: [],
    /** Whether that list is open. */
    notesOpen: false,
    /** Set while a note is on its way and the app does not yet know whether it
     *  landed. Held for a moment on purpose so the ring is seen turning. */
    _sendingTimer: null,

    // The accessibility audit. `report` is kept after the sheet closes, so the
    // label strip can keep showing which panel had what.
    access: { running: false, report: null },

    // Sheets: "none", "scan", "settings" or "access"
    sheet: "none",
    settingsTab: "viewports",

    // Scan
    scan: {
      running: false,
      rows: [],
      pending: [],
      recommended: [],
      alsoFound: [],
      conflicts: [],
      report: null,
      fallback: false,
      devUrl: null,
      logOpen: false,
      logLines: [],
    },

    // Profiles and preferences
    profiles: [],
    activeProfile: "default",
    config: {},

    // Agent bridge
    bridge: { enabled: false, active: false, url: "", mcpUrl: "", token: null },

    // Transient
    dragging: false,
    notice: null,

    get hasPanels() {
      return this.panels.length > 0;
    },

    /** How many panels are still fetching, which is what the toolbar bar shows. */
    get loadingCount() {
      return this.panels.filter((panel) => {
        const state = panel.state;
        const name = typeof state === "string" ? state : state?.state;
        return !name || name === "loading";
      }).length;
    },

    get projectName() {
      return this.project ? folderName(this.project) : null;
    },

    // ---- boot ----------------------------------------------------------

    async boot() {
      this.listen();
      const boot = await api.boot();
      this.absorb(boot.snapshot);
      this.profiles = await api.listProfiles();
      // The status bar's button needs to know whether there is a terminal to
      // open before it offers to open one.
      this.loadTerminals();
      this.loadSkills();

      if (boot.restored) {
        await this.absorbProject(boot.restored);
      } else {
        // No project and no URL: this is first launch, and the canvas stays
        // empty until someone drops a folder or types a URL.
        this.ready = true;
      }
      this.ready = true;
    },

    absorb(snapshot) {
      if (!snapshot) return;
      this.config = snapshot.config ?? {};
      this.bridge = snapshot.bridge ?? this.bridge;
      // Read rather than defaulted: the chrome reloads, and an owner claimed
      // before the reload is one whose `reports:owner` event this page never
      // heard.
      this.reportOwner = snapshot.reportOwner ?? null;
      this.reportCount = snapshot.reportCount ?? 0;
      this.reportWatching = snapshot.bridge?.watching ?? false;
      this.lastReply = snapshot.lastReply ?? this.lastReply;
      this.recentNotes = snapshot.recentNotes ?? this.recentNotes;
      this.shotDir = snapshot.shotDir ?? this.shotDir;
      this.reportPrompt = snapshot.reportPrompt ?? this.reportPrompt;
      this.activeProfile = snapshot.config?.activeProfile ?? "default";
      this.absorbCanvas(snapshot.canvas);
      if (snapshot.project) {
        this.project = snapshot.project.root ?? null;
        this.projectDriven = snapshot.project.projectDriven ?? false;
        this.urlStatus = snapshot.project.urlStatus ?? "unknown";
      }
    },

    absorbCanvas(canvas) {
      if (!canvas) return;
      this.panels = canvas.panels ?? [];
      this.totalWidth = canvas.totalWidth ?? 0;
      this.scrollX = canvas.scrollX ?? 0;
      this.panelTop = canvas.panelTop ?? this.panelTop;
      this.fitEnabled = canvas.zoomToFit;
      this.syncEnabled = canvas.scrollSync;
      this.followEnabled = canvas.followLinks ?? this.followEnabled;
      this.picking = canvas.picking ?? this.picking;
      this.verticalAlign = canvas.verticalAlign ?? this.verticalAlign;
      this.rowZoom = canvas.rowZoom ?? this.rowZoom;
      this.inspecting = canvas.inspecting ?? null;
      if (canvas.url && canvas.url !== "about:blank") {
        this.canvasUrl = canvas.url;
        // A panel finishing a load emits this, and a row of six emits it a
        // dozen times per navigation. Writing the URL box from here while
        // somebody is typing in it wipes what they typed, so pressing Enter
        // reloads the page they were already on and the app looks broken.
        if (!this.urlEditing) this.url = canvas.url;
      }
    },

    /** Everything `open_project` returned, turned into what the sheet draws. */
    async absorbProject(opened) {
      this.project = opened.root;
      this.projectDriven = opened.projectDriven;
      this.projectFileError = opened.projectFileError ?? null;
      this.scan.report = opened.report;
      this.scan.conflicts = opened.report?.conflicts ?? [];
      this.scan.recommended = opened.recommendation?.recommended ?? [];
      this.scan.alsoFound = opened.recommendation?.alsoFound ?? [];
      this.scan.fallback = opened.recommendation?.fallback ?? false;
      this.scan.devUrl = opened.devUrl ?? null;
      this.urlStatus = opened.urlStatus ?? "unknown";

      if (opened.projectFile) {
        // A project file exists, so it wins. The scan still ran, because that
        // is what makes change detection work, but it does not get applied.
        this.url = opened.projectFile.url ?? opened.devUrl ?? this.url;
        await this.applyViewports(opened.projectFile.viewports);
        if (opened.sourceChanged) {
          this.sourceChanged = {
            message: "The code's breakpoints have moved since breakpoints.md was written",
            // The file the breakpoints came from, not the first framework
            // found. Those are different files on any project where one tool
            // is detected and another one holds the widths.
            file: opened.report?.breakpointSourceFile ?? "the framework config",
          };
        }
        this.closeSheet();
      } else {
        this.url = opened.devUrl ?? this.url;
        this.openSheet("scan");
      }
    },

    // ---- events --------------------------------------------------------

    listen() {
      listen("canvas:layout", (event) => this.absorbCanvas(event.payload));
      // A wheel over a panel is panned by Rust, so the strip has to follow it
      // rather than drive it. Panels have already moved by the time this
      // arrives, so nothing is sent back.
      listen("canvas:scroll", (event) => {
        this.scrollX = event.payload ?? 0;
        const scroller = document.querySelector("[data-scroller]");
        if (scroller && Math.abs(scroller.scrollLeft - this.scrollX) > 0.5) {
          this.echoingScroll = true;
          scroller.scrollLeft = this.scrollX;
          requestAnimationFrame(() => (this.echoingScroll = false));
        }
      });
      // The authoritative count comes from Rust with every layout, and this
      // keeps the badge current in between. Without it an error logged while
      // nothing else was happening did not appear until the next relayout.
      listen("panel:console", (event) => {
        if (event.payload?.level !== "error") return;
        const panel = this.panels.find((p) => p.id === event.payload.panel);
        if (panel) panel.consoleErrors = (panel.consoleErrors ?? 0) + 1;
      });
      // The layout checks, run again after a rebuild. Only ever a line in the
      // notice band: it is information, not something to interrupt for.
      listen("checks:done", (event) => this.say(event.payload));
      listen("url:status", (event) => (this.urlStatus = event.payload));
      listen("canvas:notice", (event) => this.say(event.payload));
      // How many notes are waiting, from the one place that knows. The chrome
      // used to keep this number itself and never heard about a note collected
      // over the bridge, so the badge sat there counting work that was already
      // done.
      listen("reports:changed", (event) => {
        this.reportCount = event.payload?.count ?? 0;
        this.reportWatching = event.payload?.watching ?? false;
        // A note that waited and then went. The queue emptying while a session
        // is listening is that session being handed everything in it, which is
        // the one case where a note's fate changes after it was written. The
        // copy button empties the queue too, and `collecting` is how that case
        // is told apart rather than reported as a delivery that never happened.
        if (
          this.lastReport?.fate === "queued" &&
          this.reportWatching &&
          this.reportCount === 0 &&
          !this.collecting
        ) {
          this.lastReport = { ...this.lastReport, fate: "handed over" };
        }
      });
      listen("report:new", (event) => this.absorbReport(event.payload));

      // The note reached a socket a session was holding open. This is the one
      // event that earns the tick, which is why the tick is drawn from here
      // and not from the note being written.
      listen("report:delivered", (event) => this.confirmDelivered(event.payload));

      // The record of the sitting, from the one place that knows. Three
      // different moments change it, and Rust emits this from all three.
      listen("notes:changed", (event) => (this.recentNotes = event.payload ?? []));

      // A new project can bring its own skills, so the menu is re-read rather
      // than left showing the last project's.
      listen("project:opened", () => this.loadSkills());

      // The return leg: a session saying something back.
      listen("report:reply", (event) => {
        this.lastReply = event.payload;
        this.say(`${event.payload.from}: ${event.payload.text}`);
      });
      listen("reports:owner", (event) => {
        this.reportOwner = event.payload;
        this.say(`Reports are going to ${event.payload.name}.`);
      });
      listen("bridge:status", (event) => {
        this.bridge = event.payload;
        this.reportWatching = event.payload?.watching ?? false;
      });

      listen("scan:row", (event) => this.queueRow(event.payload));

      listen("project:drag", (event) => (this.dragging = event.payload));
      listen("project:dropped", (event) => this.openProject(event.payload));
      listen("project:dropped-not-a-folder", () => {
        this.say("Break/Points reads a project folder, not a single file.");
      });
      listen("project:opened", (event) => this.absorbProject(event.payload));

      // An agent already applied this one, so absorb it without putting a
      // sheet over the panels it just opened.
      listen("project:applied", (event) => {
        const opened = event.payload;
        this.project = opened.root;
        this.projectDriven = opened.projectDriven;
        this.scan.report = opened.report;
        this.scan.recommended = opened.recommendation?.recommended ?? [];
        this.scan.alsoFound = opened.recommendation?.alsoFound ?? [];
        this.scan.conflicts = opened.report?.conflicts ?? [];
        this.url = opened.projectFile?.url ?? opened.devUrl ?? this.url;
        this.urlStatus = opened.urlStatus ?? "unknown";
        this.closeSheet();
      });

      // The source of truth changed deliberately, so reload without asking.
      listen("project:file-changed", () => this.reloadFromProjectFile());

      // The code changed, which is a question rather than an instruction.
      listen("project:config-changed", (event) => this.noticeConfigChange(event.payload));
    },

    queueRow(row) {
      this.scan.pending.push(row);
      if (this.scan.running) return;
      this.scan.running = true;
      const drain = () => {
        const next = this.scan.pending.shift();
        if (!next) {
          this.scan.running = false;
          return;
        }
        this.scan.rows.push(next);
        setTimeout(drain, ROW_STAGGER);
      };
      drain();
    },

    // ---- project -------------------------------------------------------

    async chooseProject() {
      const path = await api.chooseProject();
      if (path) await this.openProject(path);
    },

    async openProject(path) {
      this.dragging = false;
      this.scan.rows = [];
      this.scan.pending = [];
      this.scan.logOpen = false;
      this.openSheet("scan");
      try {
        const opened = await api.openProject(path);
        await this.absorbProject(opened);
      } catch (error) {
        this.closeSheet();
        this.say(error.message);
      }
    },

    async reloadFromProjectFile() {
      if (!this.project) return;
      const opened = await api.openProject(this.project);
      if (opened.projectFile) {
        this.url = opened.projectFile.url ?? this.url;
        await this.applyViewports(opened.projectFile.viewports);
        this.say("breakpoints.md changed, so the panels reloaded from it.");
      }
    },

    async noticeConfigChange(payload) {
      if (!this.project) return;
      const report = await api.rescan();
      const known = this.scan.report?.sourceHash;
      if (known && report.sourceHash === known) return;

      const before = new Set((this.scan.report?.breakpoints ?? []).map((b) => b.width));
      const added = report.breakpoints.map((b) => b.width).filter((w) => !before.has(w));
      this.scan.report = report;
      this.sourceChanged = {
        message: added.length
          ? `Breakpoints changed, ${added.length} added (${added.join(", ")})`
          : "Breakpoints changed",
        file: payload?.file ?? "the framework config",
      };
    },

    /** Open the scan sheet on the fresh results so a change can be reviewed. */
    async reviewChanges() {
      this.sourceChanged = null;
      if (!this.project) return;
      const opened = await api.openProject(this.project);
      this.scan.rows = [];
      this.scan.recommended = opened.recommendation.recommended;
      this.scan.alsoFound = opened.recommendation.alsoFound;
      this.scan.conflicts = opened.report.conflicts;
      this.scan.report = opened.report;
      this.openSheet("scan");
    },

    // ---- canvas --------------------------------------------------------

    async applyViewports(viewports) {
      if (!viewports?.length) return;
      const url = this.url || "about:blank";
      this.totalWidth = await api.applyViewports(viewports, url);
      this.closeSheet();
    },

    /** Commit whatever is ticked in the scan sheet. */
    async useSelected(alsoWriteFile) {
      const chosen = [...this.scan.recommended, ...this.scan.alsoFound]
        .filter((candidate) => candidate.checked)
        .map(({ id, name, width, height, source }) => ({
          id,
          name,
          width,
          height,
          source,
          enabled: true,
        }))
        .sort((a, b) => a.width - b.width);

      if (!chosen.length) {
        this.say("Tick at least one viewport first.");
        return;
      }
      if (this.scan.devUrl && !this.url) this.url = this.scan.devUrl;
      await this.applyViewports(chosen);
      if (alsoWriteFile && this.project) await this.writeProjectFile(chosen);
    },

    async writeProjectFile(viewports) {
      try {
        const path = await api.writeProjectFile(viewports);
        this.projectDriven = true;
        this.say(`Wrote ${path}`);
      } catch (error) {
        this.say(error.message);
      }
    },

    /** Escape gives the box back to the panels, the way a browser does. */
    cancelUrlEdit() {
      if (this.canvasUrl) this.url = this.canvasUrl;
      document.getElementById("url")?.blur();
    },

    async go(url) {
      this.url = (url ?? this.url).trim();
      if (!this.url) return;
      if (!this.hasPanels) {
        await this.applyViewports(this.config.profiles?.[this.activeProfile]?.viewports ?? []);
      }
      try {
        await api.navigate(this.url);
      } catch (error) {
        // An address that will not parse used to send every panel to a blank
        // page and report success, so the row vanished and the box gave no
        // clue why.
        this.say(error.message);
      }
    },

    reloadAll: () => api.reloadAll(),
    reloadPanel: (id) => api.reloadPanel(id),

    // WebKit's own inspector, in its own window. It takes focus when it opens
    // and there is no way to ask it not to, so say so rather than have a
    // window appear out of nowhere.
    async inspect(id) {
      try {
        await api.inspectPanel(id);
      } catch (error) {
        this.say(error.message);
      }
    },

    async inspectChrome() {
      try {
        await api.inspectChrome();
      } catch (error) {
        this.say(error.message);
      }
    },

    async screenshot(id) {
      try {
        const path = await api.screenshotPanel(id);
        this.say(`Saved ${path}`);
      } catch (error) {
        this.say(error.message);
      }
    },

    async setFit(on) {
      this.fitEnabled = on;
      // Fit makes panels short enough to see all of; stretch makes them as
      // tall as the window allows. Asking for both at once means nothing, so
      // turning fit on drops stretch back to the top rather than leaving a
      // toolbar showing two contradictory things lit up.
      if (on && this.verticalAlign === "stretch") {
        this.verticalAlign = "top";
        await api.setVerticalAlign("top");
      }
      await api.setZoomToFit(on);
    },

    /** How far the whole row is zoomed out, 0 to 1.
     *
     *  Dragging a slider fires a change per pixel, and each one relays the
     *  whole row in Rust. Sending every one of those would queue hundreds of
     *  relayouts behind a drag that took a second. The number on screen follows
     *  the thumb immediately and Rust hears the latest value every 60ms, so the
     *  slider stays smooth and the row keeps up. */
    setRowZoom(zoom) {
      this.rowZoom = Math.min(1, Math.max(0, zoom));
      if (this._zoomTimer) return;
      this._zoomTimer = setTimeout(() => {
        this._zoomTimer = null;
        api.setRowZoom(this.rowZoom).catch((error) => this.say(error.message));
      }, 60);
    },

    /** The zoom as a percentage, for the slider and its readout. */
    get rowZoomPercent() {
      return Math.round(this.rowZoom * 100);
    },

    /** What the row is actually drawn at, which is the number that means
     *  something: "everything is half size" rather than "the slider is at 30".
     *  Matches `model::row_scale` in Rust and has to stay in step with it. */
    get rowScaleLabel() {
      const scale = this.rowZoom === 0 ? 1 : Math.pow(0.1, this.rowZoom);
      if (scale >= 0.995) return "1:1";
      return `${Math.round(scale * 100)}%`;
    },

    /** Where the panels sit vertically: "top", "center" or "stretch".
     *
     *  One setting rather than three toggles, because the three are
     *  alternatives. Stretch is the only one that touches a panel's height, and
     *  it is the one that contradicts Fit. */
    async setVerticalAlign(align) {
      this.verticalAlign = align;
      if (align === "stretch" && this.fitEnabled) {
        this.fitEnabled = false;
        await api.setZoomToFit(false);
      }
      await api.setVerticalAlign(align);
    },

    async setSync(on) {
      this.syncEnabled = on;
      await api.setScrollSync(on);
    },

    async setPicking(on) {
      this.picking = on;
      await api.setPicking(on);
      if (on) this.say("Point at what is wrong in any panel, then describe it.");
    },

    /** The most recent errors a panel's page logged, in the notice band. */
    async showConsole(panelId) {
      try {
        const lines = await api.panelConsole(panelId);
        const errors = (lines ?? []).filter((line) => line.level === "error");
        if (!errors.length) {
          this.say("No errors in this panel now.");
          return;
        }
        const latest = errors[errors.length - 1];
        const count = `${errors.length} error${errors.length === 1 ? "" : "s"}`;
        this.say(`${count}. Latest: ${latest.text.slice(0, 140)}`);
      } catch (error) {
        this.say(error.message);
      }
    },

    /** Violations found in one panel, or null if the audit has not run. */
    accessCountFor(panelId) {
      const panels = this.access.report?.panels;
      if (!panels) return null;
      const found = panels.find((panel) => panel.panel === panelId);
      if (!found) return null;
      if (found.error) return "?";
      return found.violations.length;
    },

    /** Run the checks in every panel, then show what came back.
     *
     *  The sheet is opened afterwards rather than first, because opening one
     *  hides the panels and a hidden webview is not a laid out one. Contrast
     *  and target size are measured against layout, so auditing behind a sheet
     *  would measure nothing.
     */
    async runAccessibilityAudit() {
      if (this.access.running) return;
      if (!this.hasPanels) {
        this.say("Open a project or type a URL first.");
        return;
      }
      this.access.running = true;
      this.say(`Checking ${this.panels.length} panels. This takes a few seconds each.`);
      try {
        this.access.report = await api.auditAccessibility();
        const report = this.access.report;
        if (!report.violationsTotal) {
          this.say(`No violations at any of the ${report.widthsAudited.length} widths.`);
        }
        this.openSheet("access");
      } catch (error) {
        this.say(error.message);
      } finally {
        this.access.running = false;
      }
    },

    async setFollow(on) {
      this.followEnabled = on;
      await api.setFollowLinks(on);
    },

    setScroll(offset) {
      this.scrollX = offset;
      api.setScroll(offset);
    },

    // ---- sheets --------------------------------------------------------

    openSheet(which) {
      this.sheet = which;
      api.setSheetOpen(true);
    },

    closeSheet() {
      this.sheet = "none";
      api.setSheetOpen(false);
    },

    async openSettings() {
      this.profiles = await api.listProfiles();
      this.openSheet("settings");
    },

    async toggleLog() {
      this.scan.logOpen = !this.scan.logOpen;
      if (this.scan.logOpen && !this.scan.logLines.length) {
        try {
          this.scan.logLines = await api.scanLog();
        } catch (error) {
          this.scan.logLines = [{ event: "note", message: error.message }];
        }
      }
    },

    async chooseShotDir() {
      try {
        this.shotDir = await api.chooseShotDir();
        this.say(`Screenshots go to ${this.shotDir}`);
      } catch (error) {
        this.say(error.message);
      }
    },

    async resetShotDir() {
      try {
        this.shotDir = await api.resetShotDir();
        this.say(`Screenshots go to ${this.shotDir}`);
      } catch (error) {
        this.say(error.message);
      }
    },

    /** Put the row back after inspecting. Does not close the inspector: there
     *  is no way to do that from here, and no way to be told it happened. */
    async stopInspecting() {
      await api.stopInspecting();
    },

    /** Open a terminal at the project and start the agent in it.
     *
     *  Nothing here waits for the session to connect: the terminal is a
     *  separate process and the only sign it worked is the arrow in the
     *  toolbar lighting up when the socket opens, a few seconds later. */
    async startAgentSession() {
      if (this.startingSession) return;
      this.startingSession = true;
      try {
        const name = await api.startAgentSession();
        this.say(`Starting a session in ${name}. It claims your notes when it connects.`);
      } catch (error) {
        this.say(error.message ?? String(error));
      } finally {
        // Long enough that a second press cannot open two terminals by
        // accident, short enough that a failed launch can be retried.
        setTimeout(() => (this.startingSession = false), 2500);
      }
    },

    async loadSkills() {
      try {
        this.skills = await api.listSkills();
      } catch (error) {
        this.skills = [];
      }
    },

    /** Open the skills menu, and hide the panels while it is open.
     *
     *  A panel is a separate native webview composited on top of everything the
     *  chrome draws, so a menu that hangs below the toolbar is drawn behind the
     *  row and cannot be lifted above it: z-index does not cross webviews,
     *  because this is not one page with layers in it.
     *
     *  Only about 96px below the toolbar is never covered, and the menu needs
     *  three times that. So it borrows what the sheets already do and takes the
     *  panels out of the way while it is open. */
    toggleSkills() {
      this.skillsOpen = !this.skillsOpen;
      api.setSheetOpen(this.skillsOpen || this.sheet !== "none");
      if (this.skillsOpen) this.loadSkills();
    },

    closeSkills() {
      if (!this.skillsOpen) return;
      this.skillsOpen = false;
      // Only bring the row back if nothing else wants it hidden.
      api.setSheetOpen(this.sheet !== "none");
    },

    /** Ask the listening session to run one.
     *
     *  The app cannot run a skill: a skill is instructions for an agent, and
     *  the agent is in a terminal this app does not own. So this asks, down the
     *  same socket a note goes down, and says plainly when nobody was there to
     *  hear it. */
    async runSkill(skill) {
      this.closeSkills();
      try {
        const who = await api.runSkill(skill, null);
        this.say(`Asked ${who} to run ${skill}.`);
      } catch (error) {
        this.say(error.message ?? String(error));
      }
    },

    async loadTerminals() {
      try {
        this.terminals = await api.listTerminals();
      } catch (error) {
        this.terminals = { terminals: [], active: null, command: "", defaultCommand: "" };
      }
    },

    /** Hand a panel's page to a real browser, at that panel's width. */
    async openInBrowser(panel) {
      try {
        const name = await api.openPanelInBrowser(panel);
        this.say(`Opened in ${name}.`);
      } catch (error) {
        this.say(error.message);
      }
    },

    // ---- bridge --------------------------------------------------------

    async toggleBridge() {
      try {
        this.bridge = await api.setBridge(!this.bridge.enabled);
      } catch (error) {
        this.say(error.message);
      }
    },

    // ---- misc ----------------------------------------------------------

    panelStateOf: panelState,

    /** One note as prose. Rust writes this when the note is written, so the
     *  clipboard, the bridge and a watching session all say the same thing. The
     *  fallback covers a note left in the queue by an older version. */
    reportText(report) {
      if (report.text) return report.text;
      return `${report.note}\n\nAt the ${Math.round(report.width)}px breakpoint, in the ${report.panelName} panel.\nSelector: ${report.selector}\nPage: ${report.url}`;
    },

    // A note goes to a session that is listening, and to the clipboard when
    // none is.
    //
    // The clipboard used to be taken on every note, whichever way it was
    // delivered. That made it a backstop nobody needed and cost you whatever
    // you had copied. Now it only fires when nothing is listening, which makes
    // a full clipboard the signal that no session got the note.
    async absorbReport(report) {
      if (!report) return;
      // A note that answers nothing should not sit under a stale reply, so the
      // previous answer is cleared as soon as a new note goes out.
      this.lastReply = null;
      if (this.reportWatching) {
        const to = this.reportOwner?.name;
        // In flight, not landed. The tick waits for `report:delivered`, and
        // this is what turns while it does.
        this.noteLast(report, "sending", to ?? null);
        this.awaitDelivery(report.id);
        this.say(`Sending ${Math.round(report.width)}px${to ? ` to ${to}` : ""}.`);
        return;
      }
      const text = this.reportText(report);
      try {
        await navigator.clipboard.writeText(text);
        this.noteLast(report, "queued", null, true);
        this.say(`Noted ${Math.round(report.width)}px, and copied. Nothing is listening.`);
      } catch (error) {
        this.noteLast(report, "queued", null, false);
        this.say(`Noted ${Math.round(report.width)}px. Nothing is listening.`);
      }
    },

    /** Keep the last note for the status bar.
     *
     *  `note` is what was typed, without the standing instruction in front of
     *  it or the width and selector after it, because the bar is one line and
     *  the useful half is the sentence. */
    noteLast(report, fate, to, copied = false) {
      const said = (report.note ?? "").trim();
      this.lastReport = {
        id: report.id ?? null,
        width: Math.round(report.width),
        panel: report.panelName ?? "",
        note: said || "(no description)",
        fate,
        to,
        copied,
        at: Date.now(),
      };
    },

    /** One line saying what became of the last note. */
    get lastReportFate() {
      const last = this.lastReport;
      if (!last) return "";
      if (last.fate === "delivered") return last.to ? `sent to ${last.to}` : "sent";
      if (last.fate === "handed over") return "waited, then handed over";
      if (last.fate === "collected here") return "collected with the copy button";
      if (last.fate === "sending") return "sending";
      if (last.fate === "unconfirmed") return "sent, not confirmed";
      return last.copied ? "waiting, copied to the clipboard" : "waiting";
    },

    /** Notes still waiting for the session to say anything about them.
     *
     *  A panel's label carries a dot while one of its widths is in here, which
     *  ties the answer to the width it was about rather than to a bar at the
     *  bottom of the window. */
    get unansweredNotes() {
      return this.recentNotes.filter((note) => !note.reply);
    },

    /** How many notes from this panel are still unanswered. */
    unansweredFor(panelId) {
      return this.unansweredNotes.filter((note) => note.panel === panelId).length;
    },

    /** One line saying what became of a note in the list. */
    fateOf(note) {
      if (note.reply) return "answered";
      if (note.delivered) return note.deliveredTo ? `sent to ${note.deliveredTo}` : "sent";
      return "waiting";
    },

    toggleNotes() {
      if (!this.recentNotes.length) return;
      this.notesOpen = !this.notesOpen;
    },

    /** What the connected session is doing right now, in words.
     *
     *  The toolbar has a dot that pulses, which says something is happening and
     *  nothing about what. A tool name is the difference between "the app is
     *  busy" and "it is photographing every width". */
    get sessionDoing() {
      if (!this.bridge?.active) return null;
      const tool = this.bridge?.activeTool;
      if (!tool) return "Working";
      return TOOL_WORDS[tool] ?? tool.replace(/_/g, " ");
    },

    /** Which of the three marks the status bar draws. */
    get lastReportMark() {
      const fate = this.lastReport?.fate;
      if (fate === "sending") return "spinning";
      if (fate === "delivered" || fate === "handed over") return "tick";
      return "none";
    },

    /** Give a note that has gone out a bounded time to be confirmed.
     *
     *  Without this the ring turns for ever when a socket dies between the
     *  note being written and it being sent, which reads as "still trying"
     *  when nothing is trying. "Not confirmed" is the honest end state: the
     *  note left, and the app was never told it arrived. */
    awaitDelivery(id) {
      clearTimeout(this._sendingTimer);
      this._sendingTimer = setTimeout(() => {
        if (this.lastReport?.fate !== "sending") return;
        this.lastReport =
          this.reportCount > 0
            ? { ...this.lastReport, fate: "queued" }
            : { ...this.lastReport, fate: "unconfirmed" };
      }, 6000);
      void id;
    },

    /** A note reached a listening session. Held briefly so the ring is seen
     *  turning before it closes, which is the whole point of showing it. */
    confirmDelivered(payload) {
      if (!this.lastReport) return;
      // Match by id when both ends have one. An older note in the queue going
      // out should not tick the note now on screen.
      if (payload?.id && this.lastReport.id && payload.id !== this.lastReport.id) return;
      if (this.lastReport.fate !== "sending" && this.lastReport.fate !== "queued") return;

      const settle = () => {
        clearTimeout(this._sendingTimer);
        this.lastReport = {
          ...this.lastReport,
          fate: "delivered",
          to: this.lastReport.to ?? payload?.to ?? null,
        };
      };
      const spinningFor = Date.now() - (this.lastReport.at ?? 0);
      if (spinningFor >= 450) settle();
      else setTimeout(settle, 450 - spinningFor);
    },

    /** Bring the agent's terminal forward so the rest of its answer can be
     *  read. Focuses the session that is already running rather than starting
     *  another one. */
    async focusTerminal() {
      try {
        await api.focusTerminal();
      } catch (error) {
        this.say(error.message ?? String(error));
      }
    },

    /** Everything not yet collected, as one block of text, and clears the list. */
    async copyReports() {
      this.collecting = true;
      let reports;
      try {
        reports = await api.takeReports();
      } finally {
        // A tick, because the `reports:changed` event for this collection
        // arrives after the call returns rather than before it.
        setTimeout(() => (this.collecting = false), 0);
      }
      if (this.lastReport?.fate === "queued") {
        this.lastReport = { ...this.lastReport, fate: "collected here" };
      }
      if (!reports.length) {
        // The count comes from Rust, so an empty queue means the badge was
        // stale. Ask for the truth rather than leaving a number that lies.
        this.reportCount = 0;
        this.say("Nothing waiting. Notes already collected are gone from here.");
        return;
      }
      const text = reports.map((report) => this.reportText(report)).join("\n\n---\n\n");
      try {
        await navigator.clipboard.writeText(text);
        this.say(`${reports.length} note${reports.length === 1 ? "" : "s"} copied.`);
      } catch (error) {
        this.say("Could not reach the clipboard.");
      }
    },

    say(message) {
      this.notice = message;
      clearTimeout(this._noticeTimer);
      this._noticeTimer = setTimeout(() => (this.notice = null), 5000);
    },
  });
}
