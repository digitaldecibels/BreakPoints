// The shared state, and the actions that change it.
//
// Anything two parts of the chrome both need lives here. Everything else is
// local x-data on the element that owns it.

import { api, listen } from "./api.js";
import { folderName, panelState } from "./format.js";
import { LABEL_TOP } from "./metrics.js";

/** Scan rows arrive as their detectors finish; the sheet reveals them 60ms apart. */
const ROW_STAGGER = 60;

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
    fullHeightEnabled: false,
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
      this.fullHeightEnabled = canvas.fullHeight ?? this.fullHeightEnabled;
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
      listen("url:status", (event) => (this.urlStatus = event.payload));
      listen("canvas:notice", (event) => this.say(event.payload));
      // How many notes are waiting, from the one place that knows. The chrome
      // used to keep this number itself and never heard about a note collected
      // over the bridge, so the badge sat there counting work that was already
      // done.
      listen("reports:changed", (event) => {
        this.reportCount = event.payload?.count ?? 0;
        this.reportWatching = event.payload?.watching ?? false;
      });
      listen("report:new", (event) => this.absorbReport(event.payload));
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
      // Fit makes panels short enough to see all of; full height makes them as
      // tall as the window allows. Asking for both at once means nothing, so
      // each one turns the other off rather than leaving a state where the
      // toolbar shows two contradictory things lit up.
      if (on && this.fullHeightEnabled) {
        this.fullHeightEnabled = false;
        await api.setFullHeight(false);
      }
      await api.setZoomToFit(on);
    },

    async setFullHeight(on) {
      this.fullHeightEnabled = on;
      if (on && this.fitEnabled) {
        this.fitEnabled = false;
        await api.setZoomToFit(false);
      }
      await api.setFullHeight(on);
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
      if (this.reportWatching) {
        const to = this.reportOwner?.name;
        this.say(`Sent ${Math.round(report.width)}px${to ? ` to ${to}` : ""}.`);
        return;
      }
      const text = this.reportText(report);
      try {
        await navigator.clipboard.writeText(text);
        this.say(`Noted ${Math.round(report.width)}px, and copied. Nothing is listening.`);
      } catch (error) {
        this.say(`Noted ${Math.round(report.width)}px. Nothing is listening.`);
      }
    },

    /** Everything not yet collected, as one block of text, and clears the list. */
    async copyReports() {
      const reports = await api.takeReports();
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
