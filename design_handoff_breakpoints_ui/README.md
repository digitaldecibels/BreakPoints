# Handoff: Break/Points — macOS multi-viewport testing app UI

## Overview
Break/Points is a macOS app that loads one URL into several webviews at once, each sized to a real
breakpoint from the project's own code (Tailwind config, SCSS variables, raw CSS media queries), so a
developer can see every breakpoint side by side. This handoff covers the app's five core screens:
the main window, the project scan sheet, the settings sheet, first launch, and the failure states.

Design intent, verbatim from the spec this was built to:
- **The content is the hero, and it isn't ours.** Chrome never draws attention it hasn't earned. No
  gradient washes, no shadows under panels, no accent color used decoratively.
- **Every number on screen is true.** If a panel is scaled to 62%, it says so.

Reference points: the Xcode debug bar, a machinist's rule, a film contact sheet. Not a browser, not a
dashboard, not a SaaS card grid.

## About the design files
`BreakPoints UI.dc.html` (plus its runtime `support.js`) is a **design reference created in HTML** — a
prototype showing intended look, metrics, and states. It is not production code to copy. The task is to
recreate these screens in the target environment using its established patterns. For a real macOS app
that means **SwiftUI or AppKit**, with `WKWebView` per panel inside a horizontally scrolling canvas; if
the app is being built with Electron/Tauri instead, recreate in the app's existing component layer
rather than lifting this markup.

Open the HTML file in a browser to see all five screens laid out on one canvas, each tagged with an id
badge (`1a`–`1e`) matching the section names below.

## Fidelity
**High-fidelity.** Colors, type, and metrics are final and taken from the spec. Recreate the UI
precisely. Two things are deliberately stand-ins:
- Webview interiors are diagonal-striped placeholders labeled `webview`. In the real app these are live
  `WKWebView`s rendering the target URL.
- Icons are Unicode glyphs (`→ ↻ ⚙ ◫ ⠿ ✕ ✓ ⚠`). Replace with **SF Symbols**: `arrow.right`,
  `arrow.clockwise`, `gearshape`, `doc.text` (project chip), `line.3.horizontal` (drag handle),
  `xmark`, `checkmark`, `exclamationmark.triangle`.

## Design tokens

### Color
| Token | Hex | Use |
|---|---|---|
| `--bg` | `#141416` | window background, behind and between panels |
| `--chrome` | `#1C1C1F` | toolbar bar, panels' own surfaces, sheets |
| `--chrome-raised` | `#252529` | inputs, buttons, dropdown surface |
| `--line` | `#323236` | hairlines, input borders, dividers |
| `--text` | `#E8E8EA` | primary text |
| `--text-dim` | `#8A8A92` | labels, dimensions, secondary lines |
| `--text-faint` | `#5C5C63` | scale percentages, file paths, counts |
| `--accent` | `#4C8DFF` | active states, selected checkboxes, focus rings, bridge dot |
| `--warn` | `#E0A030` | change-detected chips, low-confidence markers |
| `--error` | `#E0574A` | parse failures, dead dev URLs |
| `--ok` | `#4FB477` | detector success ticks |
| primary button ink | `#0E1520` | text on an `--accent` filled button |
| sheet scrim | `rgba(10,10,11,0.66)` | dim over canvas behind a sheet |

Dark only for MVP; light mode is post-MVP but nothing here blocks it. The accent appears on
interactive state and nothing else — never a background wash, never on a heading, never on more than
one thing at a time in a region.

### Type
One family: **SF Pro Text** (`-apple-system`). One exception: **SF Mono** for anything that is a
measurement or a path — dimensions, widths, percentages, config paths, log output. No all-caps labels,
no tracked-out eyebrows. Sentence case throughout, including buttons.

| Role | Size | Weight | Tracking | Color |
|---|---|---|---|---|
| Panel name | 12 | 590 | 0 | `--text` |
| Panel meta (mono) | 10 | 400 | 0.02em | `--text-dim` |
| Scale percentage (mono) | 10 | 400 | 0.02em | `--text-faint` |
| Toolbar input (mono) | 13 | 400 | 0 | `--text` |
| Button / control | 12 | 500 | 0 | `--text` |
| Sheet heading | 15 | 600 | −0.01em | `--text` |
| Sheet body | 13 | 400 | 0 | `--text-dim` |
| Section label in settings | 12 | 500 | 0 | `--text-dim` |

Line heights in the mock: panel name 15px, panel meta 13px, inline log lines 16px.

### Metrics
Base unit 4px; everything is a multiple.

- Toolbar 56px tall, 12px horizontal padding, 8px gap between controls
- Scroll strip 12px tall, directly under the toolbar, full width (4px `--line` thumb, 4px inset)
- Label strip 32px tall, two lines, left-aligned to each panel's left edge
- 16px between the toolbar/scroll strip and the label strip; 16px between label strip and panel top
- Gap between panels 24px; outer margin 24px left and right; 24px below panels
- Controls: 28px tall (toolbar buttons 28×28, square); settings row fields 26px; ghost buttons 20×20
- Checkboxes 14×14
- Window minimum 900 × 600
- Hairlines 1px `--line`, never doubled where two surfaces meet
- Radius: **0 on webview panels** (see below); 8px on sheets and the first-launch drop target
- Shadows: none, anywhere

**The single most important visual rule:** panels sit directly on `--bg` with a 1px `--line` border and
**no shadow and no border radius**. Web content has square corners; rounding the panel lies about where
the viewport edge is. Sheets are chrome, so 8px radius there is correct.

## Screens

### 1a — Main window (project loaded, zoom-to-fit on)
**Purpose:** the working surface. One URL rendered at every configured viewport, scrolled in lockstep.

**Layout, top to bottom:** toolbar (56px) → scroll strip (12px) → 16px → label strip (32px) → 16px →
panel row → 24px bottom margin. Panel row is a horizontal flex with 24px gaps inside a 24px side
margin, clipped by the window (horizontally scrollable in the real app).

**Toolbar, left to right:** project chip, URL field (flexes to fill), go, reload, hairline divider,
`Fit`, `Sync`, bridge dot, gear.

- **Project chip** — 28px tall, `--chrome-raised`, 1px `--line`, padding `0 10px 0 12px`, 8px gaps.
  Contents: file glyph, folder name (12/500), disclosure caret (9px, `--text-dim`). When
  project-driven it gets a **2px `--accent` left edge running full height** and the glyph is
  `--accent` — that is the entire visual signal that the app is reading your code. Copy: `Bucknell`.
  With no project it's the plain profile chip: no accent edge, no glyph, `Default` in `--text-dim`
  (see 1d). Clicking the chip opens the profile/project menu; clicking the glyph opens
  `breakpoints.md`.
- **URL field** — flex:1, min-width:0, 28px, `--chrome-raised`, 1px `--line`, 10px padding, mono 13px
  `--text`, single line, ellipsis on overflow. Copy: `https://bucknell.lndo.site`.
- **Go / reload** — 28×28, `--chrome-raised`, 1px `--line`, glyph `--text-dim`.
- **Divider** — 1px × 20px `--line`, 4px margin each side.
- **`Fit` / `Sync` toggles** — segmented-style buttons, 28px tall, 12px horizontal padding, 12/500.
  Off: `--chrome-raised` fill, `--line` border, `--text-dim` text. On: `--chrome-raised` fill,
  1px `--accent` border, `--accent` text. In 1a, `Fit` is on and `Sync` is off. **Do not use
  iOS-style switches** — too heavy for a toolbar this dense.
- **Bridge dot** — 6px circle in a 14px slot. Hidden when the agent bridge is off; static `--accent`
  when connected and idle; `--accent` gently pulsing when a connected agent is mid-request (opacity
  0.4→1.0, 2s ease-in-out). The only animated thing in the toolbar.
- **Gear** — 28×28, same treatment as go/reload, opens the settings sheet.

**Scroll strip** — 12px tall on `--bg`, 4px `--line` thumb, bottom hairline. It represents the
canvas's horizontal scroll; panels track it with **zero transition**.

**Label strip** — two lines per panel, left-aligned to that panel's left edge, scrolling in lockstep
with the canvas. Line one is the friendly name (12/590 `--text`). Line two is mono 10/400
`--text-dim` and holds framework name, true dimensions, and scale when zoom-to-fit is on, separated by
middle dots with hair spaces:

```
Tablet
md · 768×1020 · 42%
```

When a panel is off screen its label **clips at the window edge rather than sticking**.

**Label hover state** (shown on Laptop in the mock): two 20×20 ghost buttons appear at the label's
right — reload this panel, screenshot this panel. `--chrome-raised` fill, 1px `--line`, 10px glyph
`--text-dim`, 4px gap. They fade in at 120ms and **nothing else moves**. Both need real VoiceOver
labels ("Reload this panel", "Screenshot this panel").

**Panels** — 1px `--line` border, no radius, no shadow; interior is the live webview. The mock's four
panels are Mobile `sm · 640×850`, Tablet `md · 768×1020`, Laptop `lg · 1024×770`, Desktop
`xl · 1280×800`, all at 42% fit scale (269×357, 323×428, 430×323, 538×336 in the mock), with Desktop
clipping at the window edge. Panel height in the mock is the scaled viewport height; the real app may
let panels fill available height while keeping the declared width exact — width is the measurement that
matters.

### 1b — Scan sheet
**Purpose:** the most important screen in the app. After a project folder is opened, detectors report
what they found and propose a testing environment.

Sheet over a dimmed canvas: 560px wide, centered, 40px from the window top in the mock, `--chrome`
background, 1px `--line`, 8px radius, 24px padding. Scrim `rgba(10,10,11,0.66)` over the canvas.

Content order:
1. Heading, 15/600, `Scanning bucknell`.
2. **Detector rows**, 20px below the heading, 8px apart. Each row: 12px mono status glyph in a 12px
   column, 10px gap, 13px label (flex:1), right-aligned mono 10px `--text-faint` source. Glyphs:
   `✓` `--ok`, `⚠` `--warn`, `✕` `--error`. Rows in the mock:
   - `✓ Tailwind CSS v3` → `tailwind.config.js`
   - `✓ 5 configured breakpoints`
   - `✓ 12 CSS media queries` → `14 files`
   - `✓ Lando` → `.lando.yml`
   - `✓ https://bucknell.lndo.site` (mono label, it's a URL) → `responding`
   - `⚠ Could not parse _breakpoints.scss` with a 12px `--accent` `why?` beside it, expanded to show
     mono 10/16px `--text-faint` log lines
3. Hairline divider, 20px above and below.
4. `Recommended testing environment` section label (12/500 `--text-dim`), then checkbox rows 12px
   below, 8px apart. Row: 14px checkbox, 12px gap, name (13px, 80px column), **mono 12px dimensions
   right-aligned in a 110px column** so the numbers stack visually — this is the contact-sheet
   feeling — then the framework key (mono 10px `--text-faint`) pushed to the right edge.
   Rows: Mobile 640 × 850 `sm`, Tablet 768 × 1020 `md`, Laptop 1024 × 770 `lg`, Desktop 1280 × 800
   `xl`, Wide 1536 × 960 `2xl`, all checked.
5. `Also found in your CSS` section, same row structure, unchecked: `900 px` used in 3 files,
   `1100 px` used in 1 file, `1440 px` used in 6 files.
6. Footer 24px below: `View log` link (12px `--accent`) on the left; `Customize` (secondary) and
   `Use these` (primary) on the right, 8px apart.

**Checkboxes:** checked is `--accent` fill with a white 10px check; unchecked is a 1px `--line` square
on `--chrome-raised`.

**Buttons:** primary is `--accent` background with `#0E1520` text; secondary is `--chrome-raised` with
a `--line` border. **One primary button per screen, ever.**

**Parse failures** appear inline as a `⚠` row with the file name and a small `why?` that expands into
the relevant log lines, mono at `--text-faint`.

### 1c — Settings sheet (Viewports tab)
640px wide sheet, same treatment as 1b, tabbed down the left.

**Tab rail:** 160px wide, 1px `--line` right border, 20px vertical padding. Items 12px, 8px vertical
padding. Inactive: 18px left padding, `--text-dim`. Active: `--chrome-raised` fill, 2px `--accent`
left border, 16px left padding, `--text`. Tabs: Viewports, Project, Agent bridge, About.

**Panel:** 20px padding. Heading `Viewports` (15/600), sub-line `Drag to reorder. Order sets the canvas
order.` (13px `--text-dim`).

**Viewport rows** — 6px apart, each a horizontal flex with 8px gaps: 12px drag handle (`--text-faint`,
`grab` cursor) → editable name field (flex:1, 26px, `--chrome-raised`, 1px `--line`, 8px padding,
12px) → width field (64px, mono 12px, right-aligned) → height field (64px, same) → source badge
(56px, centered, mono 10px `--text-faint`, 1px `--line`, 3px vertical padding) → 20×20 remove `✕`
(transparent, `--text-faint`).

Rows in the mock: Mobile 640/850 `sm`, Tablet 768/1020 `md`, Laptop 1024/770 `lg`, then a new row in
edit state — name field has a 1px `--accent` border with `Untitled` in `--text-dim` and a 1px
`--accent` caret, both number fields show a mono `—` in `--text-faint`, badge reads `custom`. Adding
appends a row **already in edit state with the cursor in the name field**.

`Add viewport` secondary button 12px below the rows (26px tall).

**Footer:** hairline 20px above, 12px below; `Reset to detected values` as a 12px `--accent` **link at
the bottom-left, separated by the hairline — never a button next to Save**; `Cancel` (secondary) and
`Save` (primary) at the right.

### 1d — First launch
No project, no bridge. Toolbar is the same 56px structure with everything in a resting state: plain
`Default` profile chip (no accent edge, no glyph), URL field placeholder `Enter a URL` in mono
`--text-faint`, go/reload/`Fit`/`Sync` glyphs and labels at `--text-faint`, **no bridge dot**, gear
still `--text-dim`. No scroll strip and no label strip — there is nothing to measure yet.

Canvas centers a drop target: 400px wide, `40px 24px` padding, **1px dashed `--line`**, 8px radius,
centered text. Heading 15/600 `Drop a project folder to get started`; primary `Open project` button
16px below; 14px below that, 12px `--text-faint` `or just` + `test a URL` as an `--accent` link.

**On drag-over:** the border becomes solid `--accent` and the background lifts to `--chrome`.

### 1e — Failure states
Shown side by side in the mock; in the app they are separate situations.

**Dev URL not responding (left):** the URL field gains a 6px `--warn` dot at its left and
`not responding` in 12px `--text-dim` to the right of the URL. Panels still load and show whatever the
browser shows — **don't block**.

**Panel failed to load (right panel in that window):** the panel **keeps its exact dimensions** and
shows a centered mono 12px `--text-faint` message with the status code (`Failed to load · 502`) plus a
mono 10px `--text-faint` second line confirming the size is preserved (`768 × 1020 kept`). Interior is
flat `--chrome` rather than the striped placeholder. **Never collapse a panel; its size is
information.**

**Nothing detected (right sheet):** 560px sheet, same treatment. Heading `No framework config found.`,
sub-line `Break/Points read 9 stylesheets and found these widths in your media queries.`, then the
media-query width list as checkbox rows (mono widths, `used in N files` counts in `--text-faint`),
hairline, then `Or start with standard device sizes.` (13px `--text-dim`), then footer with `View log`,
secondary `Standard sizes`, primary `Use these`. **Never a shrug, always a path.**

## Interactions & behavior
- Project chip → profile/project menu. Chip glyph → opens `breakpoints.md`.
- `Fit` toggles zoom-to-fit; when on, every label's second line appends the true scale percentage. When
  off, panels render 1:1 and the percentage is omitted.
- `Sync` toggles lockstep scrolling/interaction across panels.
- Go / ⌘L-focused URL field loads the URL into every panel; reload / ⌘R reloads all panels; per-panel
  reload and screenshot live in the label hover buttons.
- Canvas pans with arrow keys and scrolls horizontally; the scroll strip and the label strip track the
  canvas exactly.
- Scan sheet: rows appear one at a time as detectors finish, 60ms apart. Checkbox state feeds the
  viewport set; `Customize` opens the settings sheet pre-filled; `Use these` commits and dismisses.
- Settings: rows drag to reorder, name/width/height edit in place, `✕` removes, `Add viewport` appends
  a row in edit state, reset restores detected values.
- Bridge dot pulses only while a connected agent is mid-request.

## State
- `project` — folder path, display name, whether `breakpoints.md` is the source of truth (drives the
  chip's accent edge and glyph)
- `url`, `urlStatus` — `ok` | `notResponding` (drives the `--warn` dot and the trailing label)
- `viewports[]` — `{ id, name, width, height, source ('sm'|'md'|'lg'|'xl'|'2xl'|'custom'), enabled }`,
  ordered; order is canvas order
- `fitEnabled`, `syncEnabled`, `fitScale` (computed; must be displayed truthfully)
- `scan` — `{ running, rows[] (status, label, source), recommended[], alsoFound[], logLines[] }`
- `panelStates[viewportId]` — `loading` | `loaded` | `failed(statusCode)`
- `bridge` — `off` | `idle` | `active`
- `sheet` — `none` | `scan` | `settings`
- `scrollX` — shared by canvas, scroll strip, and label strip

Data needs: filesystem read of the project folder (Tailwind/SCSS/CSS parsing, `.lando.yml` and similar
dev-env config, `breakpoints.md`), a reachability check on the dev URL, and a local agent-bridge
connection.

## Motion
Almost none, by design.

- Scan rows: 60ms stagger, 120ms fade, **no slide**. This is the one orchestrated motion moment in the
  app; nothing else animates on entrance.
- Sheets: 160ms fade with a 4px rise
- Hover reveals: 120ms fade
- Bridge dot pulse: 2s ease-in-out, opacity 0.4 → 1.0
- Panel repositioning during scroll: **zero transition** — it must track the scroll bar exactly
- **No animated panel transitions when sizes change**

Respect `prefers-reduced-motion` (macOS "Reduce motion"): drop the stagger and the pulse, keep the
fades.

## Accessibility floor
Full keyboard navigation: ⌘L focuses the URL bar, ⌘R reloads all, arrow keys pan the canvas. Visible
2px `--accent` focus rings on every control. All text meets 4.5:1 against its background —
`--text-faint` on `--bg` is 4.6:1, so **don't darken it further**. Every icon-only button gets a real
label for VoiceOver.

## What to avoid
These are the defaults a design tool or a component library will drift toward, and each one actively
hurts this app:
- Rounded corners or drop shadows on the webview panels
- A colored or gradient window background
- Card-style containers around panels
- All-caps labels or tracked-out eyebrows
- An accent color used decoratively rather than for state
- Device chrome (phone bezels, notches) drawn around panels — cute for a screenshot, wrong for
  measurement, since it obscures where the viewport edge actually is
- Animated panel transitions when sizes change

## Assets
None. No images, no icon files. Glyphs in the prototype are Unicode placeholders for SF Symbols
(mapping listed under Fidelity); the striped panel interiors are CSS `repeating-linear-gradient`
placeholders standing in for live webviews.

## Files
- `BreakPoints UI.dc.html` — all five screens on one canvas, tagged `1a`–`1e`
- `support.js` — runtime needed to open the HTML file locally; not part of the design
