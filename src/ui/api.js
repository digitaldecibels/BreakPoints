// The only place the chrome talks to Rust. Markup calls store actions; store
// actions call these. Nothing in a template invokes a command directly.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

async function call(name, args) {
  try {
    return await invoke(name, args);
  } catch (error) {
    console.error(`[breakpoints] ${name} failed:`, error);
    throw typeof error === "string" ? new Error(error) : error;
  }
}

export const api = {
  boot: () => call("boot"),
  state: () => call("app_state"),

  applyViewports: (viewports, url) => call("apply_viewports", { viewports, url }),
  navigate: (url) => call("navigate", { url }),
  reloadAll: () => call("reload_all"),
  reloadPanel: (panel) => call("reload_panel", { panel }),
  setScroll: (offset) => call("set_scroll", { offset }),
  setZoomToFit: (on) => call("set_zoom_to_fit", { on }),
  setVerticalAlign: (align) => call("set_vertical_align", { align }),
  setRowZoom: (zoom) => call("set_row_zoom", { zoom }),
  setScrollSync: (on) => call("set_scroll_sync", { on }),
  setFollowLinks: (on) => call("set_follow_links", { on }),
  setPicking: (on) => call("set_picking", { on }),
  takeReports: () => call("take_reports"),
  inspectPanel: (panel) => call("inspect_panel", { panel }),
  stopInspecting: () => call("stop_inspecting"),
  openPanelInBrowser: (panel) => call("open_panel_in_browser", { panel }),
  listBrowsers: () => call("list_browsers"),
  listTerminals: () => call("list_terminals"),
  startAgentSession: () => call("start_agent_session"),
  focusTerminal: () => call("focus_terminal"),
  listSkills: () => call("list_skills"),
  runSkill: (skill, context) => call("run_skill", { skill, context }),
  inspectChrome: () => call("inspect_chrome"),
  setSheetOpen: (open) => call("set_sheet_open", { open }),
  relayout: () => call("relayout"),

  chooseProject: () => call("choose_project"),
  openProject: (path) => call("open_project", { path }),
  rescan: () => call("rescan"),
  writeProjectFile: (viewports) => call("write_project_file", { viewports }),
  openProjectFile: () => call("open_project_file"),
  chooseShotDir: () => call("choose_shot_dir"),
  resetShotDir: () => call("reset_shot_dir"),

  setPreferences: (prefs) => call("set_preferences", { prefs }),
  listProfiles: () => call("list_profiles"),
  selectProfile: (id) => call("select_profile", { id }),
  saveProfile: (id, name, viewports) => call("save_profile", { id, name, viewports }),
  deleteProfile: (id) => call("delete_profile", { id }),

  scanLog: () => call("scan_log"),
  breakpointSources: () => call("breakpoint_sources"),
  setBridge: (on) => call("set_bridge", { on }),
  screenshotPanel: (panel) => call("screenshot_panel", { panel }),
  panelConsole: (panel) => call("panel_console", { panel }),
  auditAll: () => call("audit_all"),
  auditAccessibility: () => call("audit_accessibility"),
};

export { listen };
