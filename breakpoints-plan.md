# Break/Points — Build Plan (rev. 2)

**Break/Points reads your codebase and automatically builds the responsive test environment your site actually uses.**

*Your code knows your breakpoints. Break/Points does too.*

A Tauri (Rust) Mac app. Drop in a project folder and it finds your framework, discovers your real breakpoints, figures out your dev URL, and opens a row of live webviews at exactly the sizes your site cares about. Then it hands that whole environment to Claude Code or Codex so an agent can see every breakpoint while it works.

---

## 1. What this is (and isn't)

There are already tools that show one URL at several widths. Break/Points does that too, and it's the visual centerpiece, but it isn't the point. The point is that every other tool makes you configure generic device sizes that have nothing to do with your CSS. You end up testing at 375 and 768 while your site actually breaks at 900 and 1180.

Break/Points reads the project instead. Tailwind config, Drupal breakpoints.yml, raw media queries in your SCSS, whatever's there. It builds the test environment from what the code says, asks you to approve it, and writes the result to a `breakpoints.md` you can commit.

Three sentences for the website:

- Drop in a project. Break/Points finds your framework, discovers your breakpoints, and builds your testing environment.
- Your code knows your breakpoints. Break/Points does too.
- Give Claude Code eyes on every breakpoint.

---

## 2. The core loop

```
PROJECT FOLDER
   → scan (framework, breakpoints, dev URL)
   → recommend
   → you approve
   → generate viewports
   → write breakpoints.md
   → live testing environment, immediately
```

Everything in this plan serves that loop. The multi-webview canvas is the destination; the scanner is what makes getting there feel automatic.

---

## 3. Feature hierarchy

**Core canvas (the foundation, sections 5–7)**
1. Multiple real WebViews side by side
2. Multiple viewport sizes
3. URL sync across all panels
4. Horizontal viewport canvas
5. Zoom-to-fit
6. Scroll synchronization
7. Custom viewport sizes

**Primary differentiator: project intelligence (sections 8–13)**
8. Project folder detection
9. Framework detection
10. Breakpoint detection from framework config
11. CSS/SCSS media query scanning
12. Development URL detection
13. Smart breakpoint recommendations
14. Automatic viewport generation
15. `breakpoints.md` generation
16. Project configuration watching
16a. Scan diagnostics log

**Project awareness (sections 13–15)**
17. Project profiles
18. `breakpoints.md` as project source of truth
19. `.breakpoints.json` support
20. Conflict detection
21. Configuration change notifications

**AI / developer workflow (section 16)**
22. Local agent bridge
23. MCP endpoint
24. Claude Code integration
25. Codex integration
26. Screenshots
27. DOM inspection
28. `eval_js`
29. Console capture
30. Agent-controlled viewport changes
31. Agent project/breakpoint tools
32. `audit_all` cross-viewport layout audit
33. Design reference mapping and Figma MCP pairing

---

## 4. Tech stack

- **Tauri v2** with the `unstable` feature flag (multiple webviews in one window)
- **Tailwind CSS v4** for every piece of app chrome: toolbar, label strip, scan panel, settings. Installed through the `@tailwindcss/vite` plugin, so there is no `tailwind.config.js`. The design tokens in section 23 are declared once in an `@theme` block in `src/styles.css`
- **Alpine.js** for interactivity: toolbar state, the scan panel, the settings sheet. No component framework, no build-time templating, no virtual DOM. State lives in one or two `Alpine.store()` objects and the markup stays readable HTML
- **tauri-plugin-store** for app config and profiles
- **tauri-plugin-fs** + **tauri-plugin-dialog** for folder picking and project file reads
- **notify** (Rust) for watching project config files
- **axum** for the local agent bridge server
- Mac's built-in WebKit renders the pages, nothing bundled

Scanning is all Rust. No Node subprocess, no bundled parsers beyond what a crate gives us.

The chrome is already designed. `design_handoff_breakpoints_ui/` holds a built design of all five screens plus a written handoff of tokens and metrics; section 23 explains how to use it and what to translate for a webview front end. Build the UI from that package, not from a fresh interpretation of the prose.

---

## 5. The viewport canvas

One window, three zones:

- **Toolbar (top):** project chip, profile dropdown, URL field, Go/reload, zoom-to-fit toggle, scroll sync toggle, settings gear
- **Label strip:** name and true dimensions above each panel, e.g. `md — 768 × 900 (74%)`
- **Viewport row:** one child webview per viewport, left to right at real pixel sizes, horizontally scrollable

All panels load the same URL. Change it once, they all navigate.

### Fallback viewports

When there's no project and no profile (someone testing a random URL), fall back to generic device sizes:

| Name | Width | Height |
|---|---|---|
| Mobile Small | 375 | 667 |
| Mobile Regular | 430 | 932 |
| Tablet | 768 | 1024 |
| Desktop | 1440 | 900 |
| Wide | 1920 | 1080 |

These are a **fallback, not the default experience**. In rev. 1 of this plan they were the headline. They're now what you get when Break/Points has nothing better to offer.

### Horizontal scroll

Child webviews are positioned at absolute pixel coordinates inside the window. They don't live in your HTML, so CSS `overflow-x` won't move them. The trick:

1. The toolbar webview draws a scroll strip sized to the total row width.
2. Scrolling it fires a JS `scroll` event.
3. That calls a Rust command with the offset.
4. Rust repositions every child: `x = home_x - offset`.

Throttle with `requestAnimationFrame`. Trackpad swipes work naturally.

### Zoom-to-fit

Toolbar toggle. Pages still render at their true viewport width (so media queries fire correctly), but each panel is scaled down to fit the window height.

1. Per-panel scale = `available_height / panel_height`, capped at 1.0
2. Rust sets the webview's zoom and shrinks its on-screen frame to match
3. Total row width recomputes, so the scroll strip shortens
4. Toggle off restores 1.0

Two modes: **fit height** (default, each panel fits vertically) and **uniform scale** (one factor for all, so relative proportions stay true). Recompute on window resize. Labels show the scale so a shrunk view is never mistaken for actual size.

### Scroll sync

Scroll inside one panel, the others follow. Different from the canvas scroll above (that slides panels around; this scrolls the page inside them).

1. Inject a snippet into each panel on load.
2. It reports scroll position as a **percentage** of page height, not pixels. Percentage matters because the same page is a different height at 640px wide than at 1536px.
3. Rust broadcasts to the other panels; each scrolls to match.
4. Guard against feedback loops with a short "ignore my own scroll events" window (~200ms), or panels fight forever.

Percentage sync isn't perfect when layouts reflow heavily. It's close, and that's fine.

---

## 6. Project intelligence: architecture

This is the part that makes Break/Points worth building.

```
ProjectScanner
  ├── TailwindDetector        (v3 config + v4 @theme)
  ├── DrupalDetector          (*.breakpoints.yml)
  ├── BootstrapDetector       (post-MVP)
  ├── FoundationDetector      (post-MVP)
  ├── BulmaDetector           (post-MVP)
  ├── CssMediaQueryDetector   (.css / .scss / .sass)
  └── DevServerDetector       (Lando, DDEV, Vite, package.json, docker-compose)
```

Every detector implements the same trait and returns structured results. The scanner runs them all, merges the output, and hands a single report to the UI. Adding Bootstrap later means writing one detector and registering it, not touching the merge logic.

### Return types

```
FrameworkDetection
  framework      "tailwind" | "drupal" | "bootstrap" | ...
  version        optional, e.g. "3" | "4"
  confidence     0.0–1.0
  source_file    path
  breakpoints    [BreakpointDiscovery]
  metadata       free-form (config path, theme name, etc.)

BreakpointDiscovery
  width          px
  name           "md", "narrow", or null
  source         "tailwind.config.js theme.screens"
  source_file    path
  confidence     0.0–1.0
  kind           configured | framework | css | inferred

DevServerDiscovery
  url            "http://my-project.lndo.site"
  port           optional
  source         ".lando.yml"
  confidence     0.0–1.0

ScanReport
  project_root
  frameworks     [FrameworkDetection]
  breakpoints    [BreakpointDiscovery]   (merged, deduped)
  dev_servers    [DevServerDiscovery]
  conflicts      [Conflict]
  scanned_files  count
  duration_ms
```

### Scan hygiene

Non-negotiable, or scanning a real project will hang:

- Skip `node_modules`, `vendor`, `.git`, `dist`, `build`, `.next`, `coverage`, and anything in `.gitignore`
- Cap at ~3000 files and ~10MB of CSS read
- Cap directory depth around 8
- Run off the UI thread, stream progress into the scan panel
- Hard timeout at 10 seconds, return partial results rather than hanging
- Cache the report keyed by project path plus a hash of the config files, so reopening a project is instant

---

## 7. What each detector looks for

### TailwindDetector

**v3:** `tailwind.config.js`, `.ts`, `.cjs`, `.mjs`. Read `theme.screens` and `theme.extend.screens`. Handle string values (`"768px"`), object values (`{ min: "768px" }`), and `rem` (multiply by 16). Values that aren't widths (`raw` queries, `max`-only entries) get flagged rather than silently dropped.

The config is JavaScript, which means it can technically compute anything. **Don't execute it.** Parse it statically and accept that a small percentage of exotic configs won't resolve. When parsing fails, say so and fall back to the CSS scan. Node evaluation is a post-MVP option, behind a prompt, never automatic.

Every parse failure gets logged (section 8a) with the file, the line, and what the parser choked on. That log is how the detector gets better: you hit a config that fails, you read exactly why, you add the case.

**v4:** breakpoints live in CSS. Look for `@import "tailwindcss"` plus `@theme` blocks, and read `--breakpoint-*` custom properties. Simpler to parse than v3 since it's just CSS.

Detect presence first: `tailwindcss` in `package.json` dependencies, or the `@import` line. Version from the dependency range, or from which config style is present.

### DrupalDetector

Look for `composer.json` with `drupal/core*`, or a `web/`/`docroot/` layout with `core/`. Then find every `*.breakpoints.yml` in themes and modules. That format already gives names and media queries:

```yaml
mytheme.narrow:
  label: narrow
  mediaQuery: 'all and (min-width: 560px)'
  weight: 1
  multipliers: [1x]
```

Parse the `mediaQuery` for a width, keep the `label` as the viewport name. This is high-confidence data, better than anything we'd infer. Rick's Drupal work is the reason this is in the MVP.

### CssMediaQueryDetector

Scan `.css`, `.scss`, `.sass` under likely source directories. Match `@media` conditions containing `min-width` or `max-width`. Handle `px`, `em`, `rem` (assume 16px root). Also catch SCSS variables and mixin calls where the value is a literal in the same file; give up gracefully when it's indirection.

Then normalize:

- Convert everything to px integers
- Dedupe exact matches
- Cluster near-neighbors (768 and 767.98 are the same breakpoint; Bootstrap does this constantly). Collapse anything within ~2px, keep the round number
- Count how many files each width appears in. A width used in 9 files is a real breakpoint; one used once is probably a one-off tweak
- Rank by frequency, cap the list shown to the user at ~10

### DevServerDetector

Ordered by how reliable each source is:

| Source | Look for | Confidence |
|---|---|---|
| `.lando.yml` | `proxy:` entries, or `name:` → `https://{name}.lndo.site` | High |
| `.ddev/config.yaml` | `name:` → `https://{name}.ddev.site` | High |
| `vite.config.*` | `server.port`, `server.host` → `http://localhost:{port}` | High |
| `docker-compose.yml` | published port mappings on web-ish services | Medium |
| `package.json` | `scripts.dev` / `scripts.start`, parse `--port` | Medium |
| Framework default | Vite 5173, Next 3000, CRA 3000, Astro 4321 | Low |

High confidence → use it automatically. Medium or several candidates → show a short list and let you pick. Always let the user type any URL manually.

Nice touch: before auto-using a URL, do a quick HEAD request. If nothing's listening, still offer it but label it "not running."

---

## 8. Detection priority and confidence

Strict order, highest wins:

1. Existing Break/Points project config (`breakpoints.md` / `.breakpoints.json`)
2. Explicit framework breakpoint configuration (Tailwind `theme.screens`, `@theme`)
3. Framework source or convention (Bootstrap's `$grid-breakpoints`, etc.)
4. Drupal `*.breakpoints.yml`
5. Explicit CSS/SCSS media queries
6. Inferred or project-specific guesses

Two rules that matter more than the list:

- **Never silently replace a user's configuration with a lower-confidence discovery.** If `breakpoints.md` exists, it wins. Full stop. Detection results become a suggestion, not an action.
- **Surface conflicts instead of guessing.**

### Conflicts

A conflict is when two sources disagree about the same named or nearby breakpoint. Example the UI should show:

```
Two breakpoint configurations found

  Tailwind        lg = 1024px      tailwind.config.js
  CSS             lg layout at 992px    _layout.scss (4 files)

  [Use Tailwind]  [Use CSS]  [Keep both]
```

"Keep both" is a legitimate answer, since testing 992 and 1024 might be exactly what you want. Conflicts go in the `ScanReport` and render as an expandable section in the scan panel, not a blocking modal.

---

---

## 8a. Scan diagnostics log

Detection will fail on real projects. The goal isn't to prevent that, it's to make every failure legible so you can fix the detector instead of guessing why a project came up empty.

Every scan writes a log to the app's data folder as `scans/{project-slug}-{timestamp}.log`, keeping the last 20 per project. It records:

- Which detectors ran, in order, and how long each took
- Every file opened, and every file skipped with the reason (ignored dir, too large, depth cap, file cap)
- **Parse failures in full:** file path, line and column, the snippet that failed, and which parse step gave up (e.g. "theme.screens resolved to a spread of an imported identifier, cannot resolve statically")
- Values found but discarded, with the reason (`raw` query, no width component, unit not convertible, clustered into a neighbor)
- Dev URL candidates with their confidence and what produced them
- Final merged output and any conflicts

Two ways to reach it:

- **Scan panel:** a "View log" link under the results, and any parse failure surfaces inline as a one-line warning with a "why?" expander. So a failed Tailwind parse is visible in the moment, not buried three menus deep.
- **Agent bridge:** `get_scan_log` returns the structured version. Which means you can hand an agent a project that scanned badly and say "figure out why the Tailwind detector missed this and patch it." That's a real development loop for the detector itself.

Keep the log structured (JSON lines) with a readable rendering in the UI, so both eyes and agents can use it. Log failures at the same detail as successes: a detector that finds nothing should still explain what it looked at and why nothing qualified.

Never log file contents beyond the failing snippet, and never log anything from outside the project root.

## 9. Smart recommendations

Do not turn every discovered media query into a panel. A real project can have 30 media queries and you'd get an unusable wall of webviews.

Split findings into two groups. Configured breakpoints come pre-checked. Everything else is unchecked and available.

```
✓  Tailwind CSS detected                      tailwind.config.js
   5 configured breakpoints found

   ✓  640    sm
   ✓  768    md
   ✓  1024   lg
   ✓  1280   xl
   ✓  1536   2xl

   Additional CSS breakpoints found           12 media queries
   ☐  900     used in 3 files
   ☐  1100    used in 1 file
   ☐  1440    used in 6 files

✓  Vite detected                              vite.config.js
   Development server: http://localhost:5173

              [ Use Recommended ]   [ Customize ]
```

Recommendation rules:

- Framework-configured breakpoints: checked by default, always
- CSS-only widths: unchecked, sorted by how many files use them, showing that count
- Auto-check a CSS width only when no framework was found at all, and only the top few by frequency
- Show at most ~10 additional widths, with "show all" if there are more
- If nothing at all is found, fall back to the generic device set from section 5 and say so plainly

"Use Recommended" is one click to a working environment. "Customize" opens the same list with editable heights and the ability to add your own. The whole panel should read in about five seconds.

---

## 10. Generating viewports from breakpoints

A breakpoint is a width. A viewport needs a height and a name. Both get derived automatically, and both stay editable.

### Which width to render

A `min-width: 768px` breakpoint means "768 and up." Panels render at **exactly the breakpoint width**, which tests the first pixel of that range, and that's where layouts actually break.

`max-width` breakpoints are the mirror image: a `max-width: 767px` rule applies up to and including 767, so that panel renders at 767. When both a `min-width: 768` and a `max-width: 767` exist (the usual Bootstrap-style pair), they're the same boundary. Collapse them into one 768 panel rather than showing both.

Settings offer an **edge testing** option that generates two panels per breakpoint (767 and 768) so you see both sides of the transition. Off by default, since it doubles the panel count.

### Height: real device aspect ratios

Heights come from the aspect ratio of an actual device in that size class, not an arbitrary number. A 768-wide panel should look like a tablet, and a 1440-wide panel should look like a laptop.

| Width | Ratio (h ÷ w) | Reference device |
|---|---|---|
| ≤ 480 | 2.16 | iPhone 15 Pro, 393 × 852 |
| 481–900 | 1.33 | iPad portrait, 768 × 1024 |
| 901–1279 | 0.75 | iPad landscape, 1024 × 768 |
| ≥ 1280 | 0.625 | MacBook Air, 1440 × 900 |

Round to the nearest 10. Floor the result at 640px tall so nothing comes out comically short.

Worked example, a standard Tailwind set:

```
 640 × 850     (tablet portrait ratio)
 768 × 1020
1024 × 770     (landscape ratio kicks in)
1280 × 800     (laptop ratio)
1536 × 960
```

Settings can override the strategy per project: **fixed** (one height everywhere), or **manual** (you type them). Whatever the strategy, individual heights stay editable in the settings panel and in `breakpoints.md`.

### Naming: friendly by default

Panels get real-world names, because "Tablet" reads faster than "md" when you're scanning a row of five.

| Width | Name |
|---|---|
| ≤ 480 | Mobile |
| 481–767 | Large Mobile |
| 768–1023 | Tablet |
| 1024–1279 | Laptop |
| 1280–1535 | Desktop |
| ≥ 1536 | Wide |

Rules around it:

- **The framework name rides along as a subtitle.** The label strip shows `Tablet` on top and `md · 768 × 1020` underneath. You get the friendly name at a glance and the framework name when you need to go edit something.
- **Collisions get disambiguated by the framework name**, or by width when there isn't one. Two breakpoints in the Tablet band become `Tablet (md)` and `Tablet (lg)`, not `Tablet` and `Tablet 2`.
- **Every name is editable**, in the settings panel or by typing over it in `breakpoints.md`. Once you rename one, Break/Points never renames it back, even after a re-scan.
- **Drupal is a special case worth respecting.** A `*.breakpoints.yml` already has human-written labels (`narrow`, `wide`, `mobile`). Those are better than anything generated, so use them as the primary name and skip the band lookup.
- Agents match on either name, so "fix the tablet view" and "fix it at md" both resolve to the same panel.

## 11. breakpoints.md

The normalized, human-readable, Git-friendly project config. Lives in the project root, commits with the repo, and once it exists it is the **source of truth**.

```markdown
# Breakpoints

| Name | Width | Height | Source |
|------|------:|-------:|--------|
| Mobile | 640 | 850 | sm |
| Tablet | 768 | 1020 | md |
| Laptop | 1024 | 770 | lg |
| Desktop | 1280 | 800 | xl |
| Wide | 1536 | 960 | 2xl |

<!-- breakpoints:config
url: http://localhost:5173
zoomToFit: true
scrollSync: true
source: tailwind.config.js
sourceHash: 8f2a91c4
generated: 2026-09-09
-->
```

Notes on the format:

- Parse the first table with Name/Width/Height headers. A fourth `Source` column is optional and holds the framework name, so you keep the mapping back to `md` or `lg` without it cluttering the panel label. Ignore all other prose so people can document their reasoning above or below it.
- The HTML comment block holds settings and provenance. `sourceHash` is what makes change detection work (section 12).
- Also accept `.breakpoints.json` for anyone who prefers strict data. Same fields.
- If parsing fails, keep the previous config, show a warning chip in the toolbar, don't crash and don't overwrite.
- When Break/Points rewrites the file, **preserve existing prose**. Regenerate only the table and the comment block.

### Precedence

`breakpoints.md` exists → use it, mark the UI project-driven, skip auto-application of scan results. The scan still runs in the background so change detection works, but it never touches your file without asking.

---

## 12. Watching and re-scanning

Once a project is associated, watch:

- `breakpoints.md`, `.breakpoints.json`
- `tailwind.config.*`
- CSS files containing `@theme` (Tailwind v4)
- `*.breakpoints.yml`
- `.lando.yml`, `.ddev/config.yaml`, `vite.config.*`, `package.json`
- The CSS/SCSS source directories, debounced heavily

Two different behaviors, and the difference is the whole point:

**`breakpoints.md` changed** → reload viewports immediately, no prompt. You (or your agent) edited the source of truth deliberately.

**Framework config changed** → re-scan, compare against `sourceHash`. If the underlying breakpoints actually differ, show a **non-intrusive toolbar chip**:

```
⚠ Tailwind breakpoints changed — 1 added (1536)   [Review] [Dismiss]
```

Review opens a diff of old vs new with the same checkbox UI from section 9. Nothing changes until you say so. Never silently regenerate.

Debounce file events by ~500ms, and ignore CSS saves that don't change any media query width (compare a hash of extracted widths, not file contents). Otherwise every keystroke in your editor triggers a scan.

---

## 13. Profiles and project association

Profiles are named viewport sets saved in the app config. They cover the case where you're testing a URL with no local folder.

- **Default** ships with the generic set from section 5 and is read-only
- Create, rename, duplicate, delete your own
- Profile dropdown sits in the toolbar left of the URL bar
- A project file always overrides the selected profile; when project-driven, the dropdown shows the project name with a file badge and goes read-only. Clicking the badge opens `breakpoints.md`.

Editing rules:

- Edit sizes on Default → prompts "Save as new profile?"
- Edit sizes on a normal profile → unsaved-changes dot, Save Changes writes it
- Edit sizes while project-driven → offers to write back to `breakpoints.md`
- Delete always confirms; Default can't be deleted

### Associating a folder

- Drag a folder onto the window, or "Open Project..." in the gear menu
- Break/Points stores the folder-to-URL pairing, so typing that URL later reconnects the project automatically
- The agent bridge can set it too, so Claude Code working in a repo can point the app at itself

### Load order at startup

1. Read config → `lastUrl`, `activeProfile`, `projects` map
2. If `lastUrl` maps to a folder, look for `breakpoints.md` there
3. Found and parses → use it, mark project-driven, background-scan for change detection
4. Not found → run a scan, offer the recommendation panel
5. No folder → use `activeProfile`
6. Nothing at all → generic fallback set

---

## 14. First-run experience

The moment that sells the product. Keep it fast and keep it honest.

Screen `1d` in the design package is this screen drawn: resting toolbar, centered 400px drop target with a dashed `--line` border, `Open project` as the one primary button, and a quieter "or just test a URL" link. The ASCII below is the older sketch of the same thing; build the drawn one.

```
                  Break/Points

        Drop a project folder here
              or [ Open Project ]

        No project? [ Just test a URL ]
```

After a drop:

```
Scanning /Users/rick/Sites/bucknell...

✓ Tailwind CSS v3 detected            tailwind.config.js
✓ 5 configured breakpoints found
✓ 12 CSS media queries found          14 files scanned
✓ Lando detected                      .lando.yml
✓ Development server: https://bucknell.lndo.site

Recommended testing environment

   Mobile    640 × 850      sm
   Tablet    768 × 1020     md
   Laptop    1024 × 770     lg
   Desktop   1280 × 800     xl
   Wide      1536 × 960     2xl

           [ Use Recommended ]   [ Customize ]
```

Lines appear as each detector finishes, so the scan feels alive rather than frozen. On approval, in this order:

1. Generate `breakpoints.md` in the project root
2. Load the viewports
3. Associate the project with the detected URL
4. Save project config
5. Start the file watchers
6. Show the canvas, already loaded

Failure modes need to feel fine too. Nothing detected → "No framework config found. Here's what we saw in your CSS," with the media query list. Nothing at all → generic set, "Add your own sizes anytime."

---

## 15. Config format

`breakpoints.json` in the app config folder:

```json
{
  "activeProfile": "default",
  "profiles": {
    "default": { "name": "Default", "locked": true, "viewports": [ /* generic set */ ] },
    "p_k39x": { "name": "Client A", "viewports": [ /* ... */ ] }
  },
  "projects": {
    "/Users/rick/Sites/bucknell": {
      "url": "https://bucknell.lndo.site",
      "lastScan": "2026-09-09T14:22:00Z",
      "sourceHash": "8f2a91c4",
      "framework": "tailwind@3"
    }
  },
  "urlToProject": { "https://bucknell.lndo.site": "/Users/rick/Sites/bucknell" },
  "lastUrl": "https://bucknell.lndo.site",
  "heightStrategy": "device-ratio",
  "edgeTesting": false,
  "zoomToFit": true,
  "scrollSync": true,
  "agentBridge": false
}
```

Viewport configuration itself lives in the project's `breakpoints.md` whenever there is one. The app config holds preferences and the project index, not project data.

---

## 16. Agent bridge (Claude Code, Codex, MCP)

Same architecture as rev. 1, plus the project intelligence tools. Break/Points runs a local HTTP server on `127.0.0.1:7333` with an MCP endpoint on top. Two ways in:

- **HTTP transport:** `claude mcp add --transport http breakpoints http://127.0.0.1:7333/mcp`
- **Stdio shim:** a small Node script forwarding to the same API, for Codex and anything else that launches stdio processes. Codex config:

```toml
[mcp_servers.breakpoints]
command = "node"
args = ["/absolute/path/to/breakpoints/agent/stdio-shim.js"]
```

Build the HTTP API first; both transports are thin wrappers and you can curl it while debugging.

### Tools

**Canvas**

| Tool | What it does |
|---|---|
| `list_panels` | Every panel: index, name, width, height, URL, zoom |
| `screenshot_panel` | PNG of one panel by index or name. Optional `full_page` |
| `screenshot_all` | One PNG per panel |
| `navigate` | Point all panels at a URL |
| `reload` | Refresh all panels |
| `eval_js` | Run JS in one panel, return the result |
| `get_console` | Recent console errors and warnings from a panel |
| `get_dom` | Outer HTML of a selector, truncated |
| `set_viewport` | Change one panel's size on the fly |

**Project intelligence**

| Tool | What it does |
|---|---|
| `detect_project` | Point the app at a folder and run a full scan |
| `scan_breakpoints` | Re-run breakpoint detection, return the `ScanReport` |
| `get_project_info` | Current project: root, framework, dev URL, active config, last scan |
| `get_breakpoint_sources` | Where each active viewport came from: file, line, confidence |
| `get_scan_log` | Structured log of the last scan: what ran, what was skipped, what failed to parse and why |
| `generate_project_config` | Turn a scan report into a viewport set without writing anything |
| `write_project_file` | Write `breakpoints.md` and hot-reload the panels |
| `list_profiles` / `load_profile` | Profile switching |

`get_breakpoint_sources` is quietly the most useful one. An agent that knows `md` comes from line 12 of `tailwind.config.js` can go edit the right thing.

### Panel addressing

Accept both an index and a name, case-insensitive with partial matching. "Fix the overlap in window 4" and "fix it at md" both resolve. With detection in play, names come from the framework, so the agent's vocabulary already matches your codebase.

### Screenshots

Panels are child webviews composited into one native window, so there's no per-webview capture API.

**Viewport shot (default):** capture the app window with `xcap`, crop to the panel's known rect using the `home_x`/`width`/`height` already tracked. Multiply by scale factor for Retina. Needs macOS Screen Recording permission once. If the panel is scrolled off screen, auto-scroll to it, wait a frame, capture.

**Full-page shot (`full_page: true`):** inject a capture library and render the whole scroll height to a data URL. Works off screen and below the fold, less faithful. Fallback, not default.

Return a **file path**, not base64. Agents read image files fine and it keeps huge blobs out of the JSON.

### Getting values back out

`webview.eval()` in Tauri v2 is fire and forget. So `eval_js` needs a round trip: wrap the script in try/catch, POST the result to `127.0.0.1:7333/__result` with a request id, hold the HTTP response on a oneshot channel with a 10 second timeout.

Do **not** rely on `window.__TAURI__` in injected scripts. On macOS, WKWebView runs them in an isolated content world where that object doesn't exist. The scroll sync snippet has the same problem and needs the same fix: POST to the local server instead of calling `invoke()`.

### Console capture

Wrap `console.log/warn/error` in the injected snippet, keep a rolling buffer of 200, also catch `window.onerror`. Then errors are already collected when the agent asks instead of needing a reload.

### The workflow this enables

```
You:   The card grid breaks between md and lg. Fix it.
Agent: [get_project_info] → Tailwind v3, Bucknell theme, lando URL
       [get_breakpoint_sources] → md=768 and lg=1024 from tailwind.config.js
       [screenshot_panel md] [screenshot_panel lg] → sees the gap
       [eval_js md] getBoundingClientRect on .card-grid → 3-col at 768, should be 2
       [edits the theme CSS]
       [reload] [screenshot_all] → checks every breakpoint, not just the two
```

And the loop that closes it: the agent edits a Tailwind config, the watcher notices, Break/Points offers to update `breakpoints.md`, or the agent calls `write_project_file` directly and the panels change on the spot.

### Safety

A local server that runs JS in whatever page you have loaded, plus filesystem reads. Keep it tight:

- Bind `127.0.0.1` only, never `0.0.0.0`
- Off by default, toggled in settings, with a status dot in the toolbar when live
- Optional token header, written to config
- Scanner file reads confined to the associated project root, no traversal above it
- Consider limiting the bridge to debug builds if you ever ship this to other people

---

## 17. Build order

Project detection is MVP, not a post-launch bonus. But the canvas has to exist first or there's nowhere to put the results.

**Phase 1 — canvas (steps 1–7)**

1. Tauri scaffold, `unstable` feature flag
2. Toolbar page: URL bar, Go, reload. Build it to screen `1a`, including the toggles and the bridge dot slot, even though nothing is wired to them yet
3. Multiple webviews, hardcoded sizes, side by side
4. Navigation across all panels
5. Horizontal scroll strip → `set_scroll`
6. Zoom-to-fit toggle
7. Scroll sync with the feedback guard

*Checkpoint: a usable, if dumb, viewport tester, and it already looks like screen `1a`. Ship it to yourself.*

**Phase 2 — config and editing (steps 8–10)**

8. tauri-plugin-store, load/save config, generic fallback set
9. Settings sheet to screen `1c`: tab rail, drag-to-reorder viewport rows, add a row already in edit state, reset link at the bottom-left
10. Project folder selection (drag and drop, Open Project dialog), folder-to-URL map

**Phase 3 — the differentiator (steps 11–20)**

11. `ProjectScanner` skeleton: detector trait, merge logic, scan hygiene, progress events
12. `TailwindDetector` (v3 config parse, then v4 `@theme`)
13. `DrupalDetector` (`*.breakpoints.yml`)
14. `CssMediaQueryDetector` (scan, normalize, cluster, frequency rank)
15. `DevServerDetector` (Lando, DDEV, Vite, package.json, docker-compose)
16. Scan diagnostics log: structured output, in-UI viewer, inline parse warnings
17. Recommendation UI to screen `1b`: streaming detector rows, grouped checkboxes, confidence, source labels, inline parse warning with its expandable "why?"
18. Viewport generation: device-ratio heights, friendly naming, edge-testing option
19. `breakpoints.md` write and parse, prose preservation, `.breakpoints.json`
20. File watching, `sourceHash` comparison, change notification chip, conflict UI

*Checkpoint: drop a folder, get a working environment. This is the demo.*

**Phase 4 — polish and profiles (steps 21–22)**

21. Profiles: dropdown, new/rename/duplicate/delete, save changes
22. First-run experience (`1d`) and every failure state (`1e`): dev URL not responding, panel failed to load at its exact size, nothing detected. Plus the scan row stagger and panel labels with name + framework subtitle

**Phase 5 — agent bridge (steps 23–26)**

23. HTTP server, `list_panels`, `screenshot_panel`, test with curl
24. `eval_js` round trip, console capture, `get_dom`
25. MCP endpoint, stdio shim, Claude Code and Codex configs
26. Agent project tools: `detect_project`, `scan_breakpoints`, `get_breakpoint_sources`, `get_scan_log`, `write_project_file`

**Phase 6 — later**

Bootstrap, Foundation, Bulma detectors. Screenshot export for client emails. Per-panel user agents. Presets sharing.

### MVP scope

Do not build a universal static analysis engine. Version one needs:

1. Tailwind v3
2. Tailwind v4
3. Drupal `*.breakpoints.yml`
4. CSS/SCSS `@media` detection
5. Lando dev URL detection
6. Vite dev URL detection
7. `breakpoints.md`

The bar is "drop a modern web project in and it usually figures out the responsive setup." Usually. Not always.

---

## 18. Technical risks

Ranked by how likely they are to cost real time.

**Tailwind v3 configs are JavaScript.** They can import, spread, compute, and pull from other files. Static parsing will miss some. Mitigation: parse what's parseable, log exactly why it failed (section 8a), fall back to the CSS scan, offer Node evaluation as an explicit opt-in later. Do not execute project code by default. A responsive tester should never be a way to run arbitrary JS from a repo you just opened.

**Scan performance on big projects.** A Drupal site with contrib modules is enormous. Mitigation: the hygiene rules in section 6 are load-bearing, not suggestions. Test against a full Drupal install and a large monorepo before calling it done.

**Media query noise.** Real projects have print queries, `prefers-reduced-motion`, retina queries, and dozens of one-off tweaks. A naive scan produces garbage and destroys trust in the first thirty seconds. Mitigation: width-only filtering, near-neighbor clustering, frequency ranking, and everything unchecked by default.

**Dev URL guessing wrong.** Lando proxy config has several shapes; Docker port mappings are ambiguous. Mitigation: confidence scoring, HEAD check before auto-use, always show what was picked and where it came from, always allow manual override.

**File watching noise.** Watching a CSS tree during an active build fires constantly. Mitigation: heavy debounce, and compare extracted breakpoint hashes rather than file contents so a save that changes nothing relevant is a no-op.

**Multiwebview is behind an `unstable` flag.** The API can shift between Tauri releases. Mitigation: pin the version, read changelogs before upgrading. Also: panels sometimes render white on first load, usually fixed by staggering spawns a few ms apart.

**Injected scripts on macOS run in an isolated world** where `window.__TAURI__` doesn't exist. Affects scroll sync and every agent tool that needs a return value. Mitigation: route through the local HTTP server instead of `invoke()`. Solve it once, use it everywhere.

**Writing files into a user's repo.** `breakpoints.md` lands in a project root that might be a client's, and it's meant to be committed so the whole team gets the same test environment. Mitigation: always ask before the first write, show the exact path, never write outside the project root, and never touch `.gitignore` in either direction. Keeping it out of git is the user's call to make, not a default to assume.

**Pixel diffing oversells itself.** A live page never matches a comp exactly, and a percentage figure invites people to treat noise as a defect. Mitigation: present `diff_panel` as a pointer, lead with measured differences from `eval_js` rather than the image diff, and never show a pass/fail score.

**Scope creep, honestly.** Detection could eat the whole project. The canvas is what people see. Ship phase 1 to yourself early and keep using it while phase 3 gets built.

---

## 19. What changed from rev. 1

**Positioning inverted.** Rev. 1 was a viewport tester that could optionally guess breakpoints. Rev. 2 is a tool that reads your project and builds the environment, with the viewport canvas as how you see the result. Every section header and description reflects that.

**Detection promoted from a five-line bonus to sections 6–12.** Rev. 1 had one bullet ("Bonus: guess breakpoints from the project") buried inside the profiles section. That's now the detector architecture, priority rules, conflict handling, recommendations, generation strategy, and watching.

**Generic device sizes demoted to a fallback.** The 375/430/768/1440/1920 set was rev. 1's section 2 headline. It's now what you get when there's nothing better, and section 5 says so explicitly.

**Detection moved from build step 12-of-14 into phase 3 of 6**, ahead of profiles and the agent bridge. It's MVP now.

**breakpoints.md upgraded** from a hand-written convenience to a generated, normalized, provenance-carrying source of truth, with `sourceHash` for change detection and prose preservation on rewrite.

**Config format restructured.** Projects went from a flat URL-to-path map to real records holding framework, last scan, and source hash. Height strategy and edge testing added as preferences.

**Agent bridge gained six project tools**, including `get_breakpoint_sources`, which is what lets an agent trace a viewport back to the config line that defines it.

**New sections:** detector architecture (6), detector specs (7), priority and conflicts (8), scan diagnostics log (8a), recommendations (9), viewport generation (10), watching and re-scanning (12), first-run (14), risks (18).

**Decisions locked in rev. 2:** panels render at the exact `min-width` value (not mid-range); heights derive from real device aspect ratios rather than fixed tiers; viewport names are friendly by default with the framework name as a subtitle, and always editable; `breakpoints.md` is committed to the repo, not gitignored; Tailwind configs are parsed statically and never executed, with every failure logged in detail.

### Sections from rev. 1 to rewrite or drop

- **Old section 2 (Default viewports)** — rewritten. Same numbers, opposite framing. Now a fallback inside section 5.
- **Old section 4 (Config file format)** — replaced by section 15. The flat `viewports` array is gone entirely.
- **Old section 8 (Add/remove/edit viewports)** — folded into section 13. Still needed, no longer a headline feature.
- **Old section 9 "Bonus: guess breakpoints"** — deleted as a bonus, expanded into sections 6–12.
- **Old build order** — replaced wholesale by section 17. The old ordering treated detection as step 12 of 14, which is exactly backwards now.
- **Old section 12 starter code** — kept as section 20, still accurate for the canvas, but it now represents phase 1 only. The `SYNC_SCRIPT` in it needs the isolated-world fix before it works on macOS.
- **Old "Later nice-to-haves"** — Bootstrap/Foundation/Bulma detectors moved into phase 6; screenshot export and per-panel user agents stay there.

---

## 20. Project setup (phase 1)

```bash
npm create tauri-app@latest
# name: breakpoints   (no slash — see gotchas)
# frontend: Vanilla, TypeScript? No

cd breakpoints
npm install @tauri-apps/plugin-store
npm install tailwindcss @tailwindcss/vite alpinejs
cargo add tauri-plugin-store --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

File layout after setup:

```
breakpoints/
├── index.html          ← toolbar UI
├── vite.config.js      ← add the @tailwindcss/vite plugin here
├── src/main.js         ← toolbar logic
├── src/styles.css
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json
    └── src/
        ├── lib.rs      ← webview plumbing (phase 1)
        ├── scanner/    ← detectors (phase 3)
        └── bridge/     ← agent server (phase 5)
```

---

## 21. Starter code (phase 1 canvas)

### `src-tauri/Cargo.toml`

The `unstable` feature is the important line. Everything else is stock.

```toml
[package]
name = "breakpoints"
version = "0.1.0"
edition = "2021"

[lib]
name = "breakpoints_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["unstable"] }
tauri-plugin-store = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
url = "2"
```

### `src-tauri/src/lib.rs`

This is the whole engine: spawn panels, navigate, scroll, zoom, sync.

```rust
use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{
    webview::{Webview, WebviewBuilder},
    LogicalPosition, LogicalSize, Manager, WebviewUrl,
};

const TOOLBAR_H: f64 = 56.0;   // height of the toolbar strip
const GAP: f64 = 16.0;         // space between panels
const LABEL_H: f64 = 24.0;     // room above each panel for its label

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Viewport {
    pub id: String,
    pub name: String,
    pub width: f64,
    pub height: f64,
}

struct Panel {
    id: String,
    webview: Webview,
    home_x: f64,   // x position when scroll offset is 0
    width: f64,    // on-screen width (after zoom)
    height: f64,
}

#[derive(Default)]
struct AppState {
    panels: Mutex<Vec<Panel>>,
    scroll_x: Mutex<f64>,
    sync_on: Mutex<bool>,
}

// Injected into every panel. Reports scroll position as a percentage
// and listens for sync events coming back from Rust.
const SYNC_SCRIPT: &str = r#"
(function () {
  let ignoreUntil = 0;
  window.__bpApplyScroll = function (pct) {
    ignoreUntil = Date.now() + 200;
    const max = document.documentElement.scrollHeight - window.innerHeight;
    window.scrollTo(0, pct * Math.max(max, 0));
  };
  let queued = false;
  window.addEventListener('scroll', function () {
    if (Date.now() < ignoreUntil || queued) return;
    queued = true;
    requestAnimationFrame(function () {
      queued = false;
      const max = document.documentElement.scrollHeight - window.innerHeight;
      const pct = max > 0 ? window.scrollY / max : 0;
      if (window.__TAURI__) {
        window.__TAURI__.core.invoke('sync_scroll', {
          panelId: window.__BP_PANEL_ID__,
          pct: pct
        });
      }
    });
  }, { passive: true });
})();
"#;

/// Destroy any existing panels and build a fresh row.
#[tauri::command]
async fn spawn_viewports(
    app: tauri::AppHandle,
    viewports: Vec<Viewport>,
    url: String,
    zoom_to_fit: bool,
) -> Result<f64, String> {
    let state = app.state::<AppState>();
    let window = app.get_window("main").ok_or("no main window")?;

    // tear down old panels
    {
        let mut panels = state.panels.lock().unwrap();
        for p in panels.drain(..) {
            let _ = p.webview.close();
        }
    }

    let win_size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let avail_h = (win_size.height as f64 / scale) - TOOLBAR_H - LABEL_H - GAP;

    let target = normalize_url(&url);
    let mut x = GAP;
    let mut new_panels = Vec::new();

    for vp in viewports {
        let z = if zoom_to_fit {
            (avail_h / vp.height).min(1.0)
        } else {
            1.0
        };
        let w = vp.width * z;
        let h = vp.height * z;

        let builder = WebviewBuilder::new(
            format!("panel-{}", vp.id),
            WebviewUrl::External(target.clone()),
        )
        .initialization_script(&format!(
            "window.__BP_PANEL_ID__ = '{}';{}",
            vp.id, SYNC_SCRIPT
        ));

        let webview = window
            .add_child(
                builder,
                LogicalPosition::new(x, TOOLBAR_H + LABEL_H),
                LogicalSize::new(w, h),
            )
            .map_err(|e| e.to_string())?;

        let _ = webview.set_zoom(z);

        new_panels.push(Panel {
            id: vp.id.clone(),
            webview,
            home_x: x,
            width: w,
            height: h,
        });
        x += w + GAP;
    }

    *state.panels.lock().unwrap() = new_panels;
    *state.scroll_x.lock().unwrap() = 0.0;
    Ok(x) // total row width, so the UI can size its scroll bar
}

#[tauri::command]
fn navigate_all(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let target = normalize_url(&url);
    let state = app.state::<AppState>();
    for p in state.panels.lock().unwrap().iter() {
        let _ = p.webview.navigate(target.clone());
    }
    Ok(())
}

#[tauri::command]
fn reload_all(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    for p in state.panels.lock().unwrap().iter() {
        let _ = p.webview.eval("location.reload()");
    }
}

/// Slide the whole row left/right. Called from the toolbar's scroll handler.
#[tauri::command]
fn set_scroll(app: tauri::AppHandle, offset: f64) {
    let state = app.state::<AppState>();
    *state.scroll_x.lock().unwrap() = offset;
    for p in state.panels.lock().unwrap().iter() {
        let _ = p.webview.set_position(LogicalPosition::new(
            p.home_x - offset,
            TOOLBAR_H + LABEL_H,
        ));
    }
}

#[tauri::command]
fn set_sync_enabled(app: tauri::AppHandle, on: bool) {
    *app.state::<AppState>().sync_on.lock().unwrap() = on;
}

/// One panel scrolled — push its position to all the others.
#[tauri::command]
fn sync_scroll(app: tauri::AppHandle, panel_id: String, pct: f64) {
    let state = app.state::<AppState>();
    if !*state.sync_on.lock().unwrap() {
        return;
    }
    for p in state.panels.lock().unwrap().iter() {
        if p.id != panel_id {
            let _ = p.webview.eval(&format!("window.__bpApplyScroll({})", pct));
        }
    }
}

fn normalize_url(input: &str) -> url::Url {
    let s = input.trim();
    let full = if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{}", s)
    };
    url::Url::parse(&full).unwrap_or_else(|_| url::Url::parse("about:blank").unwrap())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("Break/Points")
                .inner_size(1400.0, 900.0)
                .build()?;

            // The toolbar is itself a webview, pinned across the top.
            let size = window.inner_size()?;
            let scale = window.scale_factor().unwrap_or(1.0);
            window.add_child(
                WebviewBuilder::new("toolbar", WebviewUrl::App("index.html".into())),
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(size.width as f64 / scale, TOOLBAR_H),
            )?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            spawn_viewports,
            navigate_all,
            reload_all,
            set_scroll,
            set_sync_enabled,
            sync_scroll
        ])
        .run(tauri::generate_context!())
        .expect("error running Break/Points");
}
```

### `index.html`

The toolbar. Note the scroll strip: it's an empty div sized to the total row width, which is what gives you the horizontal scroll bar.

```html
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <link rel="stylesheet" href="/src/styles.css" />
  </head>
  <body>
    <div id="bar">
      <input id="url" type="text" placeholder="example.com" spellcheck="false" />
      <button id="go">Go</button>
      <button id="reload">↻</button>
      <label><input type="checkbox" id="zoom" checked /> Fit</label>
      <label><input type="checkbox" id="sync" checked /> Sync</label>
      <button id="gear">⚙</button>
    </div>
    <div id="scroller"><div id="strip"></div></div>
    <script type="module" src="/src/main.js"></script>
  </body>
</html>
```

### `src/styles.css`

```css
* { box-sizing: border-box; }
body {
  margin: 0;
  font: 13px -apple-system, system-ui, sans-serif;
  background: #1c1c1e;
  color: #eee;
  overflow: hidden;
}
#bar {
  display: flex;
  gap: 8px;
  align-items: center;
  padding: 6px 10px;
  height: 40px;
}
#url {
  flex: 1;
  padding: 6px 10px;
  border-radius: 6px;
  border: 1px solid #3a3a3c;
  background: #2c2c2e;
  color: #eee;
}
button {
  background: #3a3a3c;
  color: #eee;
  border: 0;
  border-radius: 6px;
  padding: 6px 12px;
  cursor: pointer;
}
label { display: flex; align-items: center; gap: 4px; white-space: nowrap; }
/* the horizontal scroll strip */
#scroller { height: 16px; overflow-x: auto; overflow-y: hidden; }
#strip { height: 1px; }
```

### `src/main.js`

```js
import { invoke } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";

const DEFAULTS = [
  { id: "a1", name: "Mobile Small",   width: 375,  height: 667  },
  { id: "a2", name: "Mobile Regular", width: 430,  height: 932  },
  { id: "a3", name: "Tablet",         width: 768,  height: 1024 },
  { id: "a4", name: "Desktop",        width: 1440, height: 900  },
  { id: "a5", name: "Wide",           width: 1920, height: 1080 },
];

const $ = (id) => document.getElementById(id);
let store, cfg;

async function boot() {
  store = await load("breakpoints.json", { autoSave: true });
  cfg = {
    viewports: (await store.get("viewports")) ?? DEFAULTS,
    lastUrl:   (await store.get("lastUrl"))   ?? "https://example.com",
    zoomToFit: (await store.get("zoomToFit")) ?? true,
    scrollSync:(await store.get("scrollSync"))?? true,
  };

  $("url").value = cfg.lastUrl;
  $("zoom").checked = cfg.zoomToFit;
  $("sync").checked = cfg.scrollSync;

  await respawn();
  await invoke("set_sync_enabled", { on: cfg.scrollSync });
}

async function respawn() {
  const totalWidth = await invoke("spawn_viewports", {
    viewports: cfg.viewports,
    url: cfg.lastUrl,
    zoomToFit: cfg.zoomToFit,
  });
  $("strip").style.width = totalWidth + "px";
  $("scroller").scrollLeft = 0;
}

async function save(key, value) {
  cfg[key] = value;
  await store.set(key, value);
}

// URL bar
$("go").onclick = go;
$("url").addEventListener("keydown", (e) => e.key === "Enter" && go());
async function go() {
  await save("lastUrl", $("url").value.trim());
  await invoke("navigate_all", { url: cfg.lastUrl });
}

$("reload").onclick = () => invoke("reload_all");

// Toggles
$("zoom").onchange = async (e) => {
  await save("zoomToFit", e.target.checked);
  await respawn();
};
$("sync").onchange = async (e) => {
  await save("scrollSync", e.target.checked);
  await invoke("set_sync_enabled", { on: e.target.checked });
};

// Horizontal scroll → reposition panels
let ticking = false;
$("scroller").addEventListener("scroll", () => {
  if (ticking) return;
  ticking = true;
  requestAnimationFrame(() => {
    ticking = false;
    invoke("set_scroll", { offset: $("scroller").scrollLeft });
  });
});

// Re-fit on window resize
let resizeTimer;
window.addEventListener("resize", () => {
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(respawn, 200);
});

boot();
```

### What's stubbed

The gear button isn't wired yet. That's section 8: an overlay listing `cfg.viewports` with editable name/width/height fields, an ✕ per row, a "+ Add" button, and Reset. On save it writes `cfg.viewports` to the store and calls `respawn()`. The respawn plumbing already exists, so the settings panel is pure UI work.

Panel labels aren't drawn yet either. The `LABEL_H` gap is already reserved above each panel, so it's a matter of absolutely-positioned divs in the toolbar webview... except the toolbar webview only covers the top strip. Easiest fix: make the toolbar webview full-window with a transparent body below the bar, and draw labels there.

---

> **Note on this code:** it predates rev. 2 and covers the phase 1 canvas only. It is also pre-Tailwind: `styles.css` above is hand-written CSS and `main.js` is hand-written DOM code. Both are superseded by section 23, which specifies Tailwind v4 tokens and Alpine.js. Read this listing for the Rust plumbing it exercises, not for how the UI should be written; it is still accurate for webview spawning, scrolling, zoom, and navigation. Two things to fix before relying on it: the `SYNC_SCRIPT` calls `window.__TAURI__`, which doesn't exist in WKWebView's isolated content world on macOS (route it through the local HTTP server instead), and `DEFAULTS` in `main.js` is now the fallback set, not the primary path. No detection code is written yet, by design.

---

## 23. Visual design spec

Written to be handed straight to Claude Design or Claude Code. Everything below is a decision, not a suggestion. Where it doesn't say, use judgment.

This section has since been drawn. `design_handoff_breakpoints_ui/` in this repo is a built
design of all five screens, produced from the spec below, and it is now the visual source of
truth. Where it and the prose here disagree, the design package wins: it settles the values
this section left to judgment.

### The design package

Three files, in `design_handoff_breakpoints_ui/`:

| File | What it is |
| --- | --- |
| `README.md` | The written handoff: tokens, per-screen metrics, interactions, state shape, motion, accessibility floor |
| `BreakPoints UI.dc.html` | All five screens on one canvas, each tagged with an id badge. Open it in a browser |
| `support.js` | Runtime the HTML needs in order to open locally. Not part of the design |

The five screens, and where each one gets built:

| Id | Screen | Built in |
| --- | --- | --- |
| `1a` | Main window: project loaded, zoom-to-fit on, one label hovered | Phase 1, steps 2 to 7 |
| `1b` | Scan sheet: detector rows streaming in, recommended viewports, also-found widths | Phase 3, steps 16 to 17 |
| `1c` | Settings sheet, Viewports tab, one row in edit state | Phase 2, step 9 |
| `1d` | First launch: no project, resting toolbar, drop target | Phase 4, step 22 |
| `1e` | Failure states: dev URL not responding, panel failed to load, nothing detected | Phase 4, step 22 |

**The HTML is a reference, not code to lift.** It carries inline hex values on purpose, so it
opens anywhere with no build. The app is Tailwind v4 tokens and Alpine, as the rest of this
section says, and no template should contain a literal color. Read the mock for metrics and
states, then rebuild it in the chrome layer.

#### Two things the package assumes that we have to translate

The handoff was written to cover a native build as well as this one, so two of its
instructions need adapting for a Tauri front end:

1. **Icons.** It maps the mock's Unicode glyphs to SF Symbols, which are Apple's built-in icon
   set. They are not available inside a webview, and an icon font is a heavy dependency for
   eight glyphs. Use inline SVG instead, one small partial per icon, 12px on a 28px control,
   `fill`/`stroke` set to `currentColor` so the toggle and hover states inherit. The SF Symbol
   names stay useful as the drawing reference: `arrow.right`, `arrow.clockwise`, `gearshape`,
   `doc.text`, `line.3.horizontal`, `xmark`, `checkmark`, `exclamationmark.triangle`.
2. **Panel interiors.** The mock fills each panel with a diagonal-striped placeholder. Those
   are live webviews positioned by Rust, and the chrome layer never draws inside one. The
   single exception is the failed-panel state in `1e`, which is chrome drawn at the panel's
   exact size because the size is the information.

#### What the package pins down that the prose below leaves open

Take these from the handoff README rather than inferring them:

- **Control heights.** Toolbar buttons 28x28 square; project chip, URL field and the two
  toggles 28px tall; settings row fields 26px; label hover ghost buttons 20x20; checkboxes
  14x14.
- **Vertical rhythm under the toolbar.** 16px between the scroll strip and the label strip,
  16px between the label strip and the panel tops, 24px below the panels.
- **Scroll strip internals.** 4px `--line` thumb, 4px inset, bottom hairline.
- **Sheet scrim.** `rgba(10,10,11,0.66)` over the canvas behind any sheet. Add it to the
  `@theme` block as `--color-scrim`; it is the one token the palette table is missing.
- **Sheet geometry.** 560px wide for the scan sheet, 640px for settings, 24px padding, 40px
  from the window top.
- **Settings tab rail.** 160px wide with a 1px `--line` right border. Active item is
  `--chrome-raised` with a 2px `--accent` left border and 16px left padding; inactive is 18px
  left padding at `--text-dim`, so the text never shifts.
- **Scan sheet columns.** 12px glyph column, name in an 80px column, dimensions mono and
  right-aligned in a 110px column, framework key pushed to the right edge. The fixed columns
  are what makes the numbers stack, and that stacking is the whole contact-sheet effect.
- **First launch is a resting state, not a disabled one.** The toolbar keeps its full
  structure with glyphs at `--text-faint`, no bridge dot, and neither the scroll strip nor
  the label strip, because there is nothing to measure yet.

One correction to the type table below: **the URL field is mono**, 13px. It holds a URL, and
the rule is that anything which is a measurement or a path is mono.

### The idea behind the look

Break/Points is a measuring instrument. The webviews inside it are full of someone else's design, in whatever colors a client picked, and the app has to sit around that content without competing with it. So the chrome is quiet, dark, and precise. The one place it's allowed personality is the label strip, which reads like a ruler's markings: small, exact, and the thing your eye uses to navigate.

Reference points: the Xcode debug bar, a machinist's rule, a film contact sheet. Not a browser, not a dashboard, not a SaaS card grid.

Two rules that override everything else:

1. **The content is the hero, and it isn't ours.** Chrome never draws attention it hasn't earned. No gradient washes, no shadows under panels, no accent color used decoratively.
2. **Every number on screen is true.** If a panel is scaled to 62%, it says so. An instrument that lies is worthless.

### How this gets built

Tailwind CSS v4 and Alpine.js. Both are decisions, not options.

Tailwind v4 is configured in CSS, not in JavaScript. There is no `tailwind.config.js`;
you add `@tailwindcss/vite` to `vite.config.js`, put `@import "tailwindcss";` at the top
of `src/styles.css`, and declare the design tokens in an `@theme` block underneath. Every
color, size, and spacing value in this section becomes a token there, and nothing in the
markup should carry a raw hex value. If a class name in a template contains a literal
color, that is a bug.

Alpine.js drives the behaviour. Toolbar toggles, the scan panel's streaming rows, the
settings tabs, and the drag-to-reorder viewport list are all `x-data` on the element that
owns them, with the small amount of shared state (current URL, viewport list, fit and sync
flags, bridge status) in an `Alpine.store()`. Anything that has to talk to Rust goes
through a thin wrapper around `invoke()` rather than being called from the markup.

Two consequences worth stating up front:

- **The base layer does the resetting, not a stylesheet of our own.** Tailwind's preflight
  already zeroes margins and sets `box-sizing`. Do not hand-write a reset next to it.
- **The panels are not styled by Tailwind at all.** They are native webviews positioned by
  Rust. Tailwind only ever styles the chrome around them, which is the same boundary the
  rest of this section draws.

There is one place where a utility class is the wrong tool: the scan panel's staggered row
reveal and the bridge dot's pulse. Those are two short `@keyframes` in the stylesheet,
wrapped in the reduced-motion guard from the Motion subsection below.

### Palette

Dark by default. Light mode is post-MVP, but nothing here should make it hard.

| Token | Hex | Use |
|---|---|---|
| `--bg` | `#141416` | window background, behind and between panels |
| `--chrome` | `#1C1C1F` | toolbar bar, panels' own surfaces, settings sheet |
| `--chrome-raised` | `#252529` | inputs, buttons, dropdown surface |
| `--line` | `#323236` | hairlines, input borders, dividers |
| `--text` | `#E8E8EA` | primary text |
| `--text-dim` | `#8A8A92` | labels, dimensions, secondary lines |
| `--text-faint` | `#5C5C63` | scale percentages, file paths, counts |
| `--accent` | `#4C8DFF` | active states, selected checkboxes, focus rings, the live-bridge dot |
| `--warn` | `#E0A030` | change-detected chips, low-confidence markers |
| `--error` | `#E0574A` | parse failures, dead dev URLs |
| `--ok` | `#4FB477` | detector success ticks |

The accent is a plain workmanlike blue on purpose. It appears on interactive state and nothing else. Never as a background wash, never on a heading, never on more than one thing at a time in a given region.

Panels sit directly on `--bg` with a 1px `--line` border and **no shadow and no border radius**. Web content has square corners; rounding the panel lies about where the viewport edge is. This is the single most important visual rule in the app.

As Tailwind v4 tokens, which is the only place these values are written down:

```css
@import "tailwindcss";

@theme {
  --color-bg:            #141416;
  --color-chrome:        #1C1C1F;
  --color-chrome-raised: #252529;
  --color-line:          #323236;
  --color-text:          #E8E8EA;
  --color-text-dim:      #8A8A92;
  --color-text-faint:    #5C5C63;
  --color-accent:        #4C8DFF;
  --color-accent-ink:    #0E1520;  /* text on an accent-filled button */
  --color-warn:          #E0A030;
  --color-error:         #E0574A;
  --color-ok:            #4FB477;

  --font-sans: -apple-system, "SF Pro Text", system-ui, sans-serif;
  --font-mono: "SF Mono", ui-monospace, monospace;

  --spacing: 4px;          /* base unit; every spacing utility is a multiple */
  --radius-sheet: 8px;     /* sheets and the drop target only, never a panel */
}
```

That gives you `bg-bg`, `text-text-dim`, `border-line`, `font-mono` and so on, and the 4px
base unit makes `p-3` exactly the 12px the toolbar wants and `gap-6` the 24px between
panels. Nothing else should be added to `@theme`: if a value is needed once, it is an
arbitrary value at the call site, not a token.


### Type

One family: **SF Pro Text** via `-apple-system`, because this is a Mac app and should look like one. One exception: **SF Mono** for anything that is a measurement or a file path. Dimensions, widths, percentages, config paths, log output. That's the whole typographic system.

| Role | Size | Weight | Tracking | Color |
|---|---|---|---|---|
| Panel name | 12px | 590 (semibold) | 0 | `--text` |
| Panel meta (mono) | 10px | 400 | 0.02em | `--text-dim` |
| Scale percentage (mono) | 10px | 400 | 0.02em | `--text-faint` |
| Toolbar input | 13px | 400 | 0 | `--text` |
| Button / control | 12px | 500 | 0 | `--text` |
| Scan panel heading | 15px | 600 | -0.01em | `--text` |
| Scan panel body | 13px | 400 | 0 | `--text-dim` |
| Section label in settings | 12px | 500 | 0 | `--text-dim` |

No all-caps labels anywhere. No tracked-out eyebrows. Sentence case throughout, including buttons.

### Layout and metrics

Base unit is 4px. Everything is a multiple.

```
┌────────────────────────────────────────────────────────────────┐
│  [Bucknell ▾]  [ https://bucknell.lndo.site      ] [→] [↻]     │  56px  toolbar
│                                     [Fit] [Sync] [•] [⚙]       │
├────────────────────────────────────────────────────────────────┤
│ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁                                 │  12px  scroll strip
├────────────────────────────────────────────────────────────────┤
│  Mobile          Tablet            Laptop                      │  32px  label strip
│  sm · 640×850    md · 768×1020     lg · 1024×770  (74%)        │
│ ┌─────────────┐ ┌───────────────┐ ┌────────────────────────┐   │
│ │             │ │               │ │                        │   │
│ │   webview   │ │    webview    │ │       webview          │   │  canvas
│ │             │ │               │ │                        │   │
│ └─────────────┘ └───────────────┘ └────────────────────────┘   │
└────────────────────────────────────────────────────────────────┘
```

- Toolbar: 56px tall, 12px horizontal padding, 8px gap between controls
- Scroll strip: 12px tall, directly under the toolbar, full width
- Label strip: 32px tall, two lines, left-aligned to each panel's left edge
- Gap between panels: 24px
- Outer margin: 24px left and right, 16px below the label strip
- Window minimum: 900 × 600
- Hairlines are 1px `--line`, never doubled where two surfaces meet

### Toolbar in detail (screen `1a`)

Left to right: **project chip**, **URL field** (flexes to fill), **go**, **reload**, **Fit**, **Sync**, **bridge dot**, **gear**.

The project chip is the app's one piece of real character. When a project is loaded it shows the folder name with a small file glyph, and it is the affordance for the whole project system:

```
  ┌──────────────────────────┐
  │ ◫  Bucknell           ▾  │     project-driven, breakpoints.md is source of truth
  └──────────────────────────┘

  ┌──────────────────────────┐
  │    Default            ▾  │     no project, plain profile
  └──────────────────────────┘
```

When project-driven, the chip gets a 1px `--accent` left edge (2px wide, full height) and the glyph is `--accent`. That's the entire visual signal that the app is reading your code, and it's enough. Clicking it opens the profile/project menu; clicking the glyph opens `breakpoints.md`.

Toggles (`Fit`, `Sync`) are segmented-style buttons: off is `--chrome-raised` with `--text-dim` text, on is `--chrome-raised` with `--accent` text and a 1px `--accent` border. Do not use iOS-style switches; they're too heavy for a toolbar this dense.

The bridge dot is a 6px circle. Hidden when the agent bridge is off, `--accent` and gently pulsing when a connected agent is mid-request, static `--accent` when connected and idle. It is the only animated thing in the toolbar.

### Label strip (screen `1a`)

Two lines per panel, left-aligned to the panel edge, and it scrolls in lockstep with the canvas.

```
Tablet
md · 768×1020 · 74%
```

Line one is the friendly name. Line two is mono, `--text-dim`, and holds framework name, true dimensions, and scale when zoom-to-fit is on. Separator is a middle dot with hair spaces. When a panel is off screen, its label clips at the window edge rather than sticking.

Hovering a label reveals two 20px ghost buttons at its right: reload this panel, and screenshot this panel. They fade in at 120ms, nothing else moves.

### Scan sheet (screen `1b`)

Called a sheet rather than a panel from here on, so that "panel" only ever means a webview.

The most important screen in the app, and the one most likely to end up ugly. It appears as a sheet over a dimmed canvas, 560px wide, centered, `--chrome` background, 1px `--line` border, 8px radius (this is chrome, so radius is fine here; panels are the exception).

Detector results stream in as rows, each with a status glyph, a label, and a right-aligned mono source path:

```
  Scanning bucknell

  ✓  Tailwind CSS v3                        tailwind.config.js
  ✓  5 configured breakpoints
  ✓  12 CSS media queries                   14 files
  ✓  Lando                                  .lando.yml
  ✓  https://bucknell.lndo.site             responding

  ─────────────────────────────────────────────────────

  Recommended testing environment

  ☑  Mobile      640 × 850        sm
  ☑  Tablet      768 × 1020       md
  ☑  Laptop      1024 × 770       lg
  ☑  Desktop     1280 × 800       xl
  ☑  Wide        1536 × 960       2xl

  Also found in your CSS

  ☐  900 px      used in 3 files
  ☐  1100 px     used in 1 file
  ☐  1440 px     used in 6 files

  View log                    [ Customize ]  [ Use these ]
```

- Rows appear one at a time as detectors finish, 60ms apart. This is the one orchestrated motion moment in the app; nothing else animates on entrance.
- Status glyphs: `✓` `--ok`, `⚠` `--warn`, `✕` `--error`. 12px, mono.
- Checked boxes use `--accent` fill with a white check. Unchecked are a 1px `--line` square on `--chrome-raised`.
- Widths and dimensions are mono and right-aligned within their column so the numbers stack visually. This is the contact-sheet feeling.
- Primary button (`Use these`) is `--accent` background, `#0E1520` text. Secondary is `--chrome-raised` with `--line` border. One primary button per screen, ever.
- Parse failures appear inline as a `⚠` row with the file name and a small "why?" that expands into the relevant log lines in mono at `--text-faint`.

### Settings sheet (screen `1c`)

Same sheet treatment, 640px wide, tabbed down the left: Viewports, Project, Agent bridge, About.

Viewport rows are a drag handle, an editable name field, width and height number fields (mono), a source badge, and a remove `✕`. Adding appends a row already in edit state with the cursor in the name field. The reset link sits at the bottom-left, separated by a hairline, never as a button next to Save.

### Empty and failure states (screens `1d` and `1e`)

Direction, not mood. Every one of them offers the next action.

**First launch:** a centered drop target, 400px wide, dashed 1px `--line` border, 8px radius. Copy: "Drop a project folder to get started" with "Open project" beneath, and a quieter "or just test a URL" link. On drag-over the border becomes solid `--accent` and the background lifts to `--chrome`.

**Nothing detected:** "No framework config found." Then the CSS media query list, if any, with checkboxes. Then "Or start with standard device sizes." Never a shrug, always a path.

**Dev URL not responding:** the URL field shows a `--warn` dot and "not responding" in `--text-dim` to its right. Panels still load and show whatever the browser shows. Don't block.

**Panel failed to load:** the panel keeps its exact dimensions and shows a centered `--text-faint` message with the status code. Never collapse a panel; its size is information.

### Motion

Almost none, by design.

- Scan rows: 60ms stagger, 120ms fade, no slide
- Sheets: 160ms fade with a 4px rise
- Hover reveals: 120ms fade
- Bridge dot pulse: 2s ease-in-out, opacity 0.4 to 1.0
- Panel repositioning during scroll: **zero transition**, it must track the scroll bar exactly

Respect `prefers-reduced-motion`: drop the stagger and the pulse, keep the fades.

### Accessibility floor

Full keyboard navigation (⌘L focuses the URL bar, ⌘R reloads all, arrow keys pan the canvas). Visible 2px `--accent` focus rings on every control. All text meets 4.5:1 against its background; `--text-faint` on `--bg` is 4.6:1, so don't darken it further. Every icon-only button gets a real label for VoiceOver.

### What to avoid

Called out because these are the defaults a design tool will drift toward, and each one actively hurts this app:

- Rounded corners or drop shadows on the webview panels
- A colored or gradient window background
- Card-style containers around panels
- All-caps labels or tracked-out eyebrows
- An accent color used decoratively rather than for state
- Device chrome (phone bezels, notches) drawn around panels. It's cute for a screenshot and wrong for measurement, since it obscures where the viewport edge actually is
- Animated panel transitions when sizes change

---

## 24. Agent workflows

The tools in section 16 are primitives. These are the jobs people will actually ask for, and they need to work in one sentence.

### "Test my project at every viewport and fix what's obviously broken"

This should be a single agent turn that runs to completion. The bridge exposes one composite tool that does the tedious part:

**`audit_all`** — for every panel: screenshot it, capture console errors, and run a standard set of layout probes via `eval_js`. Returns one structured report.

The probes are the part that makes this useful, because a screenshot alone tells an agent "something looks off" without saying what. Run these in each panel:

| Probe | Catches |
|---|---|
| `document.documentElement.scrollWidth > innerWidth` | horizontal overflow, the single most common responsive bug |
| Elements whose right edge exceeds viewport width | which element is causing it |
| Overlapping bounding boxes among siblings | collisions like nav over logo |
| Text nodes with computed font-size below 12px | unreadable mobile type |
| Images with `naturalWidth` far below rendered width | blurry upscaled assets |
| Tap targets under 44px in panels below 768 | mobile usability |
| Elements with `position: fixed` taller than 30% of viewport | headers eating the screen on short viewports |
| Console errors and failed network requests | the obvious stuff |

Each finding carries the panel name, a CSS selector, the bounding box, and the measured value. So the agent gets "Laptop 1024: `.card-grid` right edge at 1043, overflows by 19px" rather than a picture and a hunch.

The report is deliberately opinionated about severity: overflow and overlap are `high`, small type and tap targets are `medium`, everything else is `low`. Agents triage badly without a ranking.

Typical run:

```
You:   Test my project at all viewports and fix any obvious issues.
Agent: [get_project_info] → Tailwind v3, Bucknell theme, lando URL, 5 panels
       [audit_all] → 3 high, 2 medium across 5 panels
                     Tablet 768: .card-grid overflows by 19px
                     Tablet 768: .site-nav overlaps .logo
                     Mobile 640: 6 tap targets under 44px
       [get_breakpoint_sources] → md comes from tailwind.config.js:12
       [reads and edits the theme CSS]
       [reload] [audit_all] → 0 high, 2 medium
       [screenshot_all] → attaches five images
       "Fixed the grid overflow and the nav overlap at md. The tap
        targets need a design call, so I left those. Here's every
        breakpoint after the change."
```

The re-audit after the fix is the part that matters. It's what makes this different from an agent editing CSS blind.

### "Compare it to the Figma"

Two halves, and Break/Points only owns one of them.

**Getting the design in.** That's Figma's job, and Figma ships a Dev Mode MCP server. The agent connects to both servers at once, pulls frames and specs from Figma, and pulls live screenshots from Break/Points. Nothing to build on our side beyond documenting the setup, which is the right call: we are not writing a Figma client.

**Holding the reference.** That part is ours, because the agent needs to know which Figma frame corresponds to which breakpoint, and it needs that mapping to persist.

Add a reference layer to the project config:

```markdown
| Name | Width | Height | Source | Reference |
|------|------:|-------:|--------|-----------|
| Mobile | 640 | 850 | sm | figma:Ab3x?node-id=12-45 |
| Tablet | 768 | 1020 | md | figma:Ab3x?node-id=12-88 |
| Laptop | 1024 | 770 | lg | .breakpoints/refs/laptop.png |
```

A reference is either a Figma node URL or a local image path under `.breakpoints/refs/`. Both commit with the repo, so the mapping is shared with the team and survives a machine change.

New tools:

- **`attach_reference(panel, ref)`** — bind a Figma node or image to a viewport, write it to `breakpoints.md`
- **`get_references()`** — return the mapping, so an agent knows which frame to fetch for which panel
- **`diff_panel(panel)`** — screenshot the panel, load its local reference image, return a side-by-side plus a difference image and a rough percentage. Only works for local image references; for Figma nodes, the agent fetches the frame via Figma's MCP and does the comparison itself with both images in context

Be honest about what `diff_panel` is. Pixel diffing a live site against a design comp produces noise, because fonts render differently, content is real instead of lorem, and images differ. It's a **pointer**, not a verdict. The output should say so, and the useful signal is structural: a section in the wrong order, a missing element, spacing that's off by 40px rather than 2px.

The workflow:

```
You:   Compare every breakpoint to the Figma and tell me what's off.
Agent: [get_references] → Figma node per panel
       [figma MCP: get frames] → design specs and images
       [screenshot_all] → live at all five sizes
       [compares each pair, plus eval_js for measured spacing]
       "Tablet is close. Laptop has the sidebar at 280px but the
        design says 320px, and the hero padding is 48px instead of 64.
        Mobile is missing the secondary nav entirely."
```

The measured comparison via `eval_js` matters more than the visual one. "Sidebar is 280px, should be 320px" is actionable; "it looks a bit narrow" isn't.

### Setup for both servers

Claude Code, both at once:

```bash
claude mcp add --transport http breakpoints http://127.0.0.1:7333/mcp
claude mcp add --transport sse figma http://127.0.0.1:3845/sse
```

Worth putting this exact pair in the README, since "Break/Points plus Figma" is the flagship story. A `CLAUDE.md` snippet in the project root helps too: tell the agent that Break/Points is running, which breakpoints exist, and that it should re-audit after any responsive change.

### Build order addition

These slot into phase 5, after the base agent tools:

27. `audit_all` with the layout probe set
28. Reference column in `breakpoints.md`, `attach_reference`, `get_references`
29. `diff_panel` for local image references
30. Documented Figma MCP pairing, `CLAUDE.md` template

---

## 25. Known gotchas

- Keep the slash out of technical names. Use `breakpoints` for the crate, bundle ID, and folder; "Break/Points" only where users see it. macOS treats "/" specially in filenames.
- The multiwebview API is behind the `unstable` flag. Pin your Tauri version and read the changelog before upgrading.
- Panels sometimes render white on first load. Staggering spawns a few ms apart usually clears it.
- Injected scripts on macOS run in WKWebView's isolated content world, where `window.__TAURI__` doesn't exist. Affects scroll sync and every agent tool that needs a return value. Route through the local server.
- Screenshots need macOS Screen Recording permission the first time, and the panel must be on screen.
- `rem`-based breakpoints assume a 16px root. If a project overrides the root font size, widths will be off. Worth a warning in the scan output when a non-default `html { font-size }` is spotted.
