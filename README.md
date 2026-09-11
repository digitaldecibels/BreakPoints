# Break/Points

**Break/Points reads your codebase and automatically builds the responsive test
environment your site actually uses.**

*Your code knows your breakpoints. Break/Points does too.*

A Tauri (Rust) Mac app. Drop in a project folder and it finds your framework,
discovers your real breakpoints, works out your dev URL, and opens a row of live
webviews at exactly the sizes your site cares about. Then it hands that whole
environment to Claude Code or Codex so an agent can see every breakpoint while
it works.

Other tools make you configure generic device sizes that have nothing to do with
your CSS. You end up testing at 375 and 768 while your site actually breaks at
900 and 1180.

## Running it

Requires macOS, Node 20 or later, and a Rust toolchain.

```bash
npm install
npm run tauri dev      # development
npm run tauri build    # a signed .app and .dmg in src-tauri/target/release/bundle
```

Use `npm run tauri dev` rather than `cargo run`. A debug build points the chrome
webview at the Vite dev server, so `cargo run` on its own opens a blank window.

## What it does with a project

Drop a folder on the window, or use Open project. Break/Points scans it and
proposes a testing environment:

1. **Tailwind**, both configuration styles. `theme.screens` in a v3 JavaScript
   config, and `--breakpoint-*` custom properties in a v4 `@theme` block.
2. **Drupal** `*.breakpoints.yml`. The labels a human wrote become the panel
   names, because they are better than anything generated.
3. **Raw media queries** in your CSS and SCSS, filtered to width conditions,
   with near neighbours collapsed and the rest ranked by how many files use
   them.
4. **Your dev URL**, from Lando, DDEV, Vite, Docker Compose or a package.json
   script, checked to see which one is actually answering.

Configured breakpoints come pre-ticked. Everything else is offered, unticked,
with the file count that says how much your codebase leans on it. One click
gives you a working environment.

Tailwind configs are JavaScript, so some of them compute their screens. Those
are parsed statically and never executed: a responsive tester should not be a
way to run arbitrary code from a repo you just opened. When a config will not
resolve, the scan sheet says so on the spot, with a "why?" that shows the line
and the reason.

## breakpoints.md

Approving a scan writes `breakpoints.md` into the project root. It commits with
the repo, so everyone on the team gets the same test environment, and once it
exists it is the source of truth. Detection keeps running in the background so
change detection works, but it never touches your file without asking.

```markdown
# Breakpoints

| Name | Width | Height | Source |
|------|------:|-------:|--------|
| Tablet | 768 | 1020 | md |
| Laptop | 1024 | 770 | lg |

<!-- breakpoints:config
url: https://mysite.lndo.site
zoomToFit: true
scrollSync: true
source: tailwind.config.js
sourceHash: 8f2a91c4
generated: 2026-09-09
-->
```

Prose around the table is preserved when the file is rewritten, so you can
document why a breakpoint is where it is.

Panels render at exactly the breakpoint width, which is the first pixel of the
range and where layouts actually break. Heights come from the aspect ratio of a
real device in that size class, so a 768 panel looks like a tablet and a 1440
panel looks like a laptop. Both stay editable.

Swipe sideways on the trackpad to pan the row, anywhere in the window including
over a panel. Scrolling up and down goes to the page under the pointer, and with
Sync on, every other panel follows it to the same point in the page rather than
the same pixel, because the page is a different height at 640 than at 1536. A
carousel or anything else that scrolls sideways keeps its own gesture. There is
also a pan strip along the bottom edge.

With Follow on, clicking a link in any one panel takes the whole row to that
page, so you can walk a site at six widths at once instead of loading each one
by hand. It works for a normal page load and for a single page app that changes
the URL through the history API. An anchor link is left alone: it only changes
the fragment, and Sync already carries the other panels to the same place
without reloading them. If a site redirects on width, two panels can end up
sending each other back and forth, and Follow notices that and turns itself off
rather than hammering your dev server.

## Giving an agent eyes on every breakpoint

Turn the agent bridge on in Settings. It listens on `127.0.0.1:7333` only, is
off until you turn it on, and requires a token that the app generates for you.

The token has to go with the request, so pass it as a header:

```bash
TOKEN=$(python3 -c "import json,os;print(json.load(open(os.path.expanduser(
  '~/Library/Application Support/com.digitaldecibels.breakpoints/breakpoints.json'
)))['bridgeToken'])")
claude mcp add --transport http breakpoints http://127.0.0.1:7333/mcp \
  --header "Authorization: Bearer $TOKEN"
```

Run that from the project you want to test in, and `claude mcp list` should show
it connected. Then you can say things like "frame 2 is broken" and the agent can
go and look: `list_panels` gives every panel a `position` counted from the left
starting at one, so "the second one" resolves to something specific.

For Codex, or anything else that launches a process and talks over stdin and
stdout:

```toml
[mcp_servers.breakpoints]
command = "node"
args = ["/absolute/path/to/BreakPoints/agent/stdio-shim.js"]
```

The shim reads the token from the app's own config, so there is nothing to copy
by hand.

Twenty two tools, in three groups.

**The canvas:** `list_panels`, `navigate`, `reload`, `eval_js`, `get_console`,
`get_dom`, `set_viewport`, `screenshot_panel`, `screenshot_all`.

**Project intelligence:** `detect_project`, `scan_breakpoints`,
`get_project_info`, `get_breakpoint_sources`, `get_scan_log`,
`generate_project_config`, `write_project_file`, `list_profiles`,
`load_profile`.

**Composite and design:** `audit_all`, `attach_reference`, `get_references`,
`diff_panel`.

`get_breakpoint_sources` is quietly the most useful one. An agent that knows
`md` comes from line 12 of `tailwind.config.js` can go and edit the right thing.

`audit_all` is the one that turns "test my project at every viewport and fix
what's obviously broken" into a single turn. For every panel it captures console
errors and runs layout probes for horizontal overflow, overlapping siblings,
type under 12px, upscaled images, tap targets under 44px on small viewports, and
fixed elements eating a short viewport. Each finding carries the panel, a CSS
selector, the bounding box and the measured value, so an agent gets "Laptop
1024: `.card-grid` right edge at 1043, overflows by 19px" rather than a picture
and a hunch. Findings are ranked, because agents triage badly without one.

The bridge can also be curled directly, which is the easiest way to see what a
tool returns:

```bash
TOKEN=$(python3 -c "import json,os;print(json.load(open(os.path.expanduser(
  '~/Library/Application Support/com.digitaldecibels.breakpoints/breakpoints.json'
)))['bridgeToken'])")
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:7333/api/list_panels
```

### Comparing against a design

Break/Points owns half of this. It holds the mapping from panel to reference
frame, in `breakpoints.md`, so it commits with the repo and survives a machine
change. Fetching the design is Figma's own MCP server's job.

```bash
claude mcp add --transport sse figma http://127.0.0.1:3845/sse
```

`diff_panel` works on local image references and returns a difference image and
a rough figure. Be honest about what that figure is worth: a live page never
matches a comp exactly, because fonts render differently, content is real
instead of lorem, and images differ. It is a pointer, not a verdict. The useful
signal is structural, and measured differences from `eval_js` beat the picture
every time.

## Improving a detector

Detection will fail on real projects. The point is to make every failure legible
enough to fix rather than guess at. Every scan writes a JSON lines log to
`~/Library/Application Support/com.digitaldecibels.breakpoints/scans/`, keeping
the last 20 per project, recording which detectors ran, every file skipped and
why, every parse failure with its line and snippet, and every value found and
discarded with the reason.

Reach it three ways: "View log" in the scan sheet, `get_scan_log` from an agent,
or the command line, which is the fastest loop when you are working on a
detector:

```bash
cd src-tauri
cargo run --example scan -- /path/to/project --log
```
