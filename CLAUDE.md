# Break/Points

A Tauri v2 Mac app that reads a project's real breakpoints out of its code and
opens a live webview at each one. `breakpoints-plan.md` is the spec and
`design_handoff_breakpoints_ui/` is the visual source of truth. Where the plan's
prose and the design package disagree, the design package wins.

## Commands

```bash
npm run tauri dev                              # run it
npm run tauri build                            # bundle a .app and .dmg
cd src-tauri && cargo test                     # 150 tests, no window needed
cd src-tauri && cargo run --example scan -- <folder> [--log]
python3 scripts/smoke-test.py [--start]     # drive the running app
```

`cargo run` on its own opens a blank window. A debug build points the chrome
webview at the Vite dev URL, so the dev server has to be running, which is what
`npm run tauri dev` does.

The smoke test is the only thing that exercises the window. The unit tests
cover the arithmetic and the decisions; the failures this codebase actually
hits are a command without its permission file, a capability line left out, and
a panel that is not the width its label claims, and every one of those passes
`cargo test`. It needs a screen, so it is a local check rather than a CI one.

The scan example is the fastest way to work on a detector. It runs the whole
scanner against a real folder and prints what each one found, and `--log` adds
every skipped file and parse failure.

## How the window is put together

One window holds two kinds of webview.

- **The chrome** fills the window and draws the toolbar, the scroll strip, the
  label strip, the sheets and the empty states. Tailwind v4 and Alpine, no
  component framework.
- **Panels**, one per viewport, are child webviews positioned by Rust at
  absolute pixel coordinates.

Panels are created after the chrome, so they composite **on top of it**. Three
consequences, and all three are load bearing:

1. A sheet can only be seen if the panels get out of the way, so opening one
   calls `set_sheet_open`, which hides them. That is `canvas::set_panels_hidden`.
2. Chrome drawn over the canvas area is invisible while panels are up. Notices
   go in the label strip band, which is the lowest chrome that a panel never
   covers.
3. A failed panel is hidden so the chrome can draw the failed state at the
   panel's exact size. Never collapse a panel: its size is the information.
4. A wheel event over a panel goes to that panel's webview and the chrome never
   hears it, so horizontal panning has to be forwarded by the injected script.
   The chrome's own `@wheel.window` handler only ever covers the toolbar, the
   label strip and the bare canvas around a panel.

## One session across the whole row

Every panel shares cookies, `localStorage` and session storage. Verified by
setting a cookie and a storage key in one panel and reading both back in two
others. macOS gives every WKWebView the same default website data store unless
it is told otherwise, and nothing here tells it otherwise.

This is what makes the app usable on anything behind a login: sign in once in
any panel and the whole row is signed in, at every width, with no repetition.
It is worth knowing before changing how panels are created, because giving a
panel its own data store would quietly turn one login into seven.

The opposite is occasionally wanted, comparing a signed-in and a signed-out
render side by side, and it is not possible today.

## Panning the row

Three ways in, and they all end at `canvas::set_scroll`:

- A sideways trackpad swipe over a panel. The injected script sends the delta
  and `canvas::nudge_scroll` clamps it and emits `canvas:scroll` so the strip
  follows. A vertical swipe is left alone and scrolls the page under the
  pointer, and a sideways one over something with `overflow-x` that still has
  somewhere to go is left alone too, so a carousel keeps its own gesture.
- The same swipe over the chrome, handled by `onWheel` in `main.js`.
- The pan strip along the bottom edge, which drives Rust rather than following
  it. When the strip is being moved to match a wheel pan, `echoingScroll` stops
  it sending that position straight back.

## Following links across the row

Clicking a link in one panel takes every other panel to that page. Two things
report a URL. A real document load comes from Tauri's `on_page_load`, which is
the navigation delegate and needs nothing from the page; see the trap about a
panel that has never committed for why the injected script's own `/p/ready` is
not trusted for this. A URL change with no document load, which is a single page
app using the history API, comes from the injected script as `/p/nav`. Both end
at `canvas::follow_navigation`.

The whole difficulty is that **every panel we push looks exactly like someone
clicking a link.** Navigate five panels and five ready reports come back, each
of which would trigger another five. `canvas::Follow` is the bookkeeping that
tells them apart: a push writes down which panels it is waiting to hear from,
and every report is spent rather than acted on until that list empties. The row
is free again the moment the last one reports, so a fast second click is still
followed. A redirect landing inside that window is absorbed the same way, which
is right: each panel follows the redirect itself, at its own width.

A panel that never reports, because it failed or because it loaded while it was
hidden behind a sheet, would wedge that list forever, so there is a four second
deadline. The report that trips the deadline is **acted on, not spent**. Spending
it swallowed the first click after a panel went quiet, which is the exact case
the deadline exists for, and the URL comparison already catches it if it turns
out to have been a late echo after all.

`Follow` is a plain struct with no webview in it, so the decision is unit
tested. `follow_navigation` only does what the decision says.

A fragment is deliberately not a navigation. An anchor link changes only the
hash, scroll sync already carries the other panels there, and reloading six
webviews for a jump inside a page they already have would be worse than doing
nothing. Analytics code rewrites the hash constantly and every one of those
would otherwise be a page load.

A site that redirects on width puts two panels on different URLs and each one
sends the other back forever. Six broadcasts in five seconds is not a person
clicking, so `FOLLOW_BURST` turns Follow off and says why in the label strip.

## Traps

Each of these cost real time, and none of them announce themselves.

**A struct literal that locks the same mutex twice deadlocks against itself.**
Temporaries in a struct literal or a `json!` live until the whole expression
finishes, so `Snapshot { config: state.config.lock()..., bridge: status(state) }`
hangs forever, because `status` locks `config` again. This took down the entire
app at boot once. Read each lock into a local on its own line first.

**Panels must render at exactly the breakpoint width.** A webview frame is whole
pixels and the CSS viewport is frame width divided by zoom, so rounding the
frame from a nominal scale leaves the page at 767 when it should be at 768, and
a `min-width: 768px` query silently does not fire. `canvas::layout` derives the
zoom back from the rounded frame width instead. There is a test that asserts
`width / scale == declared width` at five different window heights. Do not
delete it.

**Store each panel as it spawns, not at the end.** A page can finish loading and
post back before the last panel has been created. Holding the list until the
whole row is built dropped those reports and left every panel but the last stuck
on "loading".

**Injected scripts cannot use `window.__TAURI__`.** A script injected into a page
we did not serve has no Tauri IPC. That is why there is a loopback callback
server on an ephemeral port (`callback.rs`) that panels POST to, with a
per-session nonce. It posts `text/plain` so the request stays simple and skips a
CORS preflight. Rust pushes the other way with `webview.eval`. Values come back
through `eval_with_callback`, which does not need the server at all.

**A new command needs TWO files, and the second one is not generated for you.**
`permissions/autogenerated/<command>.toml` is committed to the repo despite what
its header says, and nothing writes it: adding a command without writing that
file by hand fails the *build script*, not the compile, with "Permission
allow-x not found, expected one of ..." and a list that has every other command
in it. Worse, the build script validates capabilities before the crate compiles,
so there is no single build that can generate the file and then use it. Copy an
existing toml, change the two identifiers and the command name, then add the
capability line. The check at the end of this section only catches the second
half, so run this too:

```bash
python3 - <<'EOF'
import re, os
src = open('src-tauri/src/lib.rs').read()
cmds = [c.strip().split('::')[-1]
        for c in re.search(r'generate_handler!\[(.*?)\]', src, re.S).group(1).split(',')
        if c.strip()]
print([c for c in cmds
       if not os.path.exists(f'src-tauri/permissions/autogenerated/{c}.toml')] or 'all present')
EOF
```

**Every command the chrome calls also needs a line in `capabilities/default.json`,
and a missing one looks like the whole app is dead.** Each of this app's own
commands has a generated permission file in
`src-tauri/permissions/autogenerated/`, and having one is what puts that command
under Tauri's access control list. The capability granted only the core and
plugin permissions, so every app command was refused with "boot not allowed.
Permissions associated with this command: allow-boot".

Nothing in the window worked, because everything in the window is a command:
`boot` never ran, so no project was restored and no panels were opened, and the
URL bar, the Go button, Reload, Settings and the project menu all failed
silently. It reads as a dead UI rather than as a permission problem, because the
only place the refusal appears is the webview's own console, which is not the
terminal you are watching.

The bridge is not affected, because it is an HTTP server and not a webview. So
an agent can drive the whole app perfectly while the window does nothing, which
is exactly how this went unnoticed.

**Adding a command is four steps, not one.** Write it in `commands.rs`, add it
to `generate_handler!` in `lib.rs`, write
`permissions/autogenerated/<command>.toml` beside the others, and add its
`allow-` line to the capability. Miss the last and that one command fails while
everything else keeps working. This check catches it:

```bash
python3 - <<'EOF'
import re, json
src = open('src-tauri/src/lib.rs').read()
cmds = [c.strip().split('::')[-1]
        for c in re.search(r'generate_handler!\[(.*?)\]', src, re.S).group(1).split(',')
        if c.strip()]
cap = json.load(open('src-tauri/capabilities/default.json'))['permissions']
print([c for c in cmds if 'allow-' + c.replace('_', '-') not in cap] or 'all granted')
EOF
```

**The URL box is the person's, not the canvas's.** `canvas:layout` carries the
row's current URL, and `absorbCanvas` used to write it straight into
`$store.bp.url`, which is what the URL input is bound to. A row of six panels
emits that event a dozen times per navigation, so anything typed in the box was
wiped mid-keystroke and Enter reloaded the page that was already open. It looked
exactly like the button being dead.

The box now ignores canvas events while it has focus, and Escape puts the
panels' real URL back the way a browser does. Anything else that writes to a
field a person can be standing in needs the same treatment.

**A panel that has never committed a navigation eats its own messages.** wry
holds a `pending_scripts` queue until the webview's first `did_commit_navigation`.
While it is set, `eval` pushes the script onto that queue as a bare string and
**throws the callback away**. A panel whose first load fails, which is every
panel when the dev URL is down or its certificate is not trusted, never commits,
so it stays in that state. Every pump drain is queued instead of run and comes
back as "panel closed before it answered", and when the next load finally
commits, all of those queued `__bpDrain()` calls run at once, empty the queue and
hand the contents to nobody.

The visible symptom is that entering a URL and pressing Go appeared to do
nothing: the pages loaded, but the one `/p/ready` each panel sent was drained
into the void, so every label stayed on "loading" forever and a followed link
went nowhere. A reload fixed it, which is what made it look like a page problem
rather than a runtime one.

Two things guard it now. The panel state and link following are driven by
Tauri's own `on_page_load`, which comes from the navigation delegate and needs
nothing from the page. And `Panel::ever_committed` keeps the pump from asking a
panel that has never committed, because asking is worse than not asking.

**An https panel cannot reach that server, and every real site is https.**
WebKit blocks an http request made by an https document and, unlike Chrome, it
does not exempt `127.0.0.1` as a potentially trustworthy origin. Every Lando,
DDEV and hosted URL is https, so on the sites this app exists for the POST
throws "Load failed" and nothing arrives. That silently killed three features at
once: panels stayed on "loading" forever, `get_console` always returned an empty
list, and scroll sync did nothing.

The fix is `canvas::start_pump`. An https page queues its messages in
`window.__bpOut` instead of posting, and one background task drains every panel
with `eval` and feeds the queue through `callback::dispatch_from`, which is the
same code a posted message reaches. An http page still posts directly and is
never pumped.

Asking a webview a question is not free: each drain is a script evaluation and a
callback through Tauri's own machinery, and six panels at a fixed rate cost real
CPU for as long as the app is open. That matters for an app whose job is
measuring how a page performs, so the tick follows what is happening: 16ms for
400ms after any message, 150ms while the window has focus, 600ms once it has
been quiet for five seconds, and 1.5s when the window is behind something else,
because nobody is scrolling a panel by hand from another app. A pan registers
within about 90ms even from the slowest of those.

The panel id comes from the caller, never from the message body. A page can put
whatever it likes in what it sends, so the one thing it must not be trusted with
is which panel it is.

Before assuming a panel-to-Rust message works, check the protocol of the page
you tested on. On `http://localhost:1420` everything works and nothing is
proved.

**`overflow: hidden` is still a scroll container.** The label strip clips the
labels that run off the side of the window, and every label is absolutely
positioned across the full width of the row. Focus one of the ghost buttons on
a label that is off to the right and WebKit scrolls the strip across to reveal
it. There is no scrollbar to show that it happened and nothing ever puts it
back, so the labels sit permanently offset from the panels they name. Use
`overflow: clip`, which creates no scroll container at all, everywhere the
chrome clips rather than scrolls. That is the label strip, the app wrapper and
`html, body`.

**A Vite reload of the chrome has twice ended the process, exit code 0.** Both
times immediately after a `[vite] (client) page reload` line, taking the window
with it. It has not been reproducible since: twelve reloads on 12 September
2026, including five issued while the panels were still loading, which is the
race the teardown in `boot` would lose, left the app up every time with all
seven panels intact. So it is either gone or very rare. If it happens again the
thing worth writing down is what the row was doing at the time, because none of
the states tried here were enough to cause it.

**`productName` cannot contain a slash.** macOS treats it specially in
filenames. The bundle is `BreakPoints`; "Break/Points" is the window title and
everything users read. Keep the slash out of crate names, bundle ids and
folders.

**Scan hygiene is two budgets, not one.** Looking at a directory entry is cheap;
keeping and reading are not. A flat 3000 file cap gets entirely spent on Drupal
core before it ever reaches the theme. `walk.rs` looks at up to 120000 entries,
keeps only the files a detector actually asks for, and caps those at 3000. On
cfcs that is the difference between 3000 files and nothing found, and 12 files
and the right answer.

**Hidden directories are tooling, except `.ddev`.** `.claude/worktrees/` holds a
whole second copy of some repos. Tailwind's own probe writes a CSS file in
`.twprobe/` that reads exactly like a second config.

**Drupal core is not the site.** `web/core` holds thousands of stylesheets and a
pile of `*.breakpoints.yml` that belong to Drupal rather than to this project.
`walk.rs` skips it, and skips `contrib` for the same reason.

**A bundler's dev server is not the site.** In a Drupal project under Lando,
Vite serves assets on 5173 and PHP serves the pages. A Lando proxy entry is
usually tooling too, unless its hostname starts with the project name. When
several candidates survive, `project::pick_dev_url` asks each one in turn and
takes the first that answers.

## Where things are

```
src-tauri/src/
  lib.rs           window, chrome webview, drag and drop, command registry
  canvas.rs        panel spawning, layout, scroll, zoom, scroll sync
  callback.rs      loopback server that injected panel scripts post to
  commands.rs      Tauri commands, all thin wrappers over tools/canvas/project
  tools.rs         the operations, written once, shared with the bridge
  bridge.rs        agent bridge: HTTP API plus MCP, and the tool definitions
  audit.rs         the cross viewport layout probes
  access.rs        axe-core in every panel, violations grouped by width
  shots.rs         window capture cropped to a panel rect
  references.rs    design references and diff_panel
  project.rs       opening a project, precedence, apply, write
  project_file.rs  breakpoints.md read and write, prose preserving
  generate.rs      breakpoints to viewports: heights, names, edge testing
  watcher.rs       debounced file watching, two categories of change
  scanner/         walk, log, units, jsobj, tailwind, drupal, css, devserver
  assets/axe.min.js  vendored axe-core, compiled into the binary
src/               the chrome: index.html at the root, ui/ for the Alpine parts
agent/stdio-shim.js  MCP over stdio for Codex, forwards to the HTTP bridge
```

## Reporting a problem

The Report toggle arms an element picker in every panel. Hover highlights what
is under the pointer, a click opens a small box, and what you type is sent back
with the width it happened at attached. A layout is only wrong at some widths,
so a note without the width is worth very little, and that is the one thing a
person will not remember to write down.

It has to be a mode. While it is armed a click describes an element instead of
following a link, and doing that to somebody by surprise would be worse than
having a button.

**Four ways out of the box, because one is never enough.** A close control, a
Cancel button, Escape, and clicking away. Escape is handled on the document
rather than on the text area: keying it off the focused element meant that the
moment the cursor left the box there was no way out of it at all, which is the
definition of a trap. Escape closes the form first and the mode second, so it
never does two things at once.

The picker lives inside the page, not in the chrome, because a panel is a child
webview that composites on top of the chrome and nothing the chrome draws over
a panel can be seen. Everything it adds is marked `data-bp-ui`, is ignored by
its own hit testing, and is taken out again when picking stops. It is also
re-armed after every page load, since the picker belongs to a document and a
new document has never heard of it.

**The width comes from the panel, never from the page.** A page can say anything
about itself. The note is about the breakpoint we put it at, so
`callback::report` reads the declared width out of the panel and only carries
`innerWidth` alongside it, because when those two disagree that is itself the
bug.

**A note is pushed into a session, and the clipboard is what happens when
there is no session to push to.** A watching agent holds a socket open on
`/ws/reports` and is handed each note as it is written; `await_reports` is the
same delivery for a client that would rather hold an HTTP request open.
`take_reports` still exists and still drains the queue, for a client that wants
to ask rather than wait. Connecting is the claim, so there is no separate
request and a session cannot end up listening while notes are addressed
somewhere else.

The clipboard is now a signal rather than a habit: a note only lands there when
nothing is listening, so a note on the clipboard is one no session received.
The toolbar says so at the time, and its count of what is waiting comes from
Rust on `reports:changed` rather than being counted in the window.

**The queue survives the app, and the claim deliberately does not.** Both used
to live only in memory, so a Rust edit, a Vite reload or a crash took every
uncollected note with it, and every note written afterwards was addressed to
nobody. `take_reports_for` matched only the exact session id, so those notes
sat in the queue while the session that wanted them polled straight past.

Now the queue is written to `reports.json` in the app data folder and read back
at boot. The claim is not restored, because it names a session that may have
ended while the app was down, and an unaddressed note goes to whoever asks
next, which is what makes a stranded note collectable at all.

**Delivery takes a note out of the queue, so a send that fails has to put the
rest back.** Otherwise a note written in the seconds after a watcher went away
is drained for a socket that cannot carry it and then dropped, which is the
same stranding in a new place. Found by disconnecting mid-test, not by reading
the code. For the same reason the socket is read as well as written: a watcher
going away has to be noticed at once, because the app suppresses the clipboard
while it believes somebody is listening.

**Claimed and listening are different facts.** The toolbar can name a session
while the arrow beside it is dim, which means the next note waits in the queue
rather than arriving anywhere.

The injected script is `r##"…"##` rather than `r#"…"#`, because the picker
builds id selectors with `"#" + id` and that sequence closes a single-hash raw
string.

## Inspecting a page

Every panel's label carries an Inspect button that opens WebKit's own Web
Inspector on that panel, so a page can be picked apart at the width it is
actually rendering at rather than at a width a browser is pretending to be.
Cmd+Alt+I does the same for the chrome itself, which is the only way to read
the app's own console; without it a failure inside the chrome is invisible from
the terminal, which is how a permissions problem once read as a dead window.

**Inspecting is a mode, because opening the inspector takes the window.**
WebKit docks its inspector into the webview it is inspecting and gives that
webview the window's full content area: measured on a 5120px display, a 1024px
panel's page reported `innerWidth` of 5120. So it covers the chrome and every
panel to its right, and the width on its label stops being true, which for this
app is the worst failure available.

`canvas::set_inspecting` handles both halves. It hides every other panel, since
they are behind a docked inspector and cannot be seen anyway, and it re-asserts
the inspected panel's frame every 400ms for as long as the mode lasts. Setting
the size back does win and does stick; it just does not stay won, because the
inspector re-takes the frame whenever it lays itself out. A one-shot restore
was the first attempt and it loses a race it cannot see.

**Leaving the mode is manual, and has to be.** Nothing reports that the
inspector has closed: WebKit raises no event and Tauri surfaces none. So the
chrome shows a bar in the label strip band with a way out, and pressing Inspect
again on the same panel toggles. Window focus was the tempting alternative and
it is worse, because you click between the inspector and the page constantly
while using it and every one of those would rebuild the row underneath you.

The inspector also **takes focus** when it opens, and WebKit offers no way to
ask otherwise, so this is the one part of the app that cannot stay behind what
you are doing.

`devtools` is in the `tauri` features in `Cargo.toml`: it is on automatically in
a debug build and off in release without it, so leaving it out would give a
bundled app two dead buttons.

**Chrome DevTools cannot attach to a panel, and no setting will change that.**
A panel is a WKWebView, because Tauri renders through WRY and WRY uses the
platform's native engine, which on macOS is WebKit with no alternative. Chrome
DevTools speaks the Chrome DevTools Protocol; a WKWebView speaks WebKit's own
remote inspector protocol. Different protocol, not a preference. `browser.rs`
is the answer instead: every label has a button that opens that panel's page in
Chrome, Brave, Firefox or Safari at the panel's declared width, where that
browser's own tools work. Chrome gets `--app` and its own profile, because
passing a size to an already-running Chrome is ignored.

`tools.rs` exists so an agent tool and a toolbar button run the same code. Add
an operation there, then wrap it in `commands.rs` and add a case to
`bridge::call_tool` with a definition in `tool_definitions`. A command the
chrome calls also needs a permission file and a line in
`capabilities/default.json`; see the trap about that, because forgetting it
fails silently.

## Adding a detector

Write a module in `src-tauri/src/scanner/` with a `run(index, budget, log) ->
DetectorOutput`, then register it in `scanner::scan`. The merge, the conflict
detection and the recommendation logic need no changes.

Three rules the existing detectors follow:

- **Never execute project code.** Parse statically, and when that fails, log
  exactly why (`ScanLog::parse_failure` takes the file, the line, the snippet
  and which step gave up). That log is how a detector gets better.
- **Explain what you discarded.** A detector that finds nothing still has to say
  what it looked at and why nothing qualified. `ScanLog::discarded` is half the
  job.
- **Never silently replace configuration with a lower confidence discovery.**
  Surface a disagreement as a conflict instead of picking a side.

## What is built

All six phases of the plan's build order, plus the two agent workflows in
section 24. The canvas, the settings and scan sheets, all four detectors,
the diagnostics log, viewport generation, `breakpoints.md`, file watching,
profiles, the first launch and failure states, the agent bridge with MCP and a
stdio shim, `audit_all`, references and `diff_panel`.

**A screenshot is answered with a path, not with the picture.** The MCP
specification has a shape for returning an image, which is the file written out
as text inside the reply, and `screenshot_panel` deliberately does not use it.
It saves the PNG and answers with where it went.

The reason is that the bridge only listens on loopback, so whatever is calling
it is on this machine and can open the file. Writing a full-page capture into
the reply instead would put several megabytes of text through a single tool
call, every time, to save a client from a read it can already do. Revisit only
if something turns up that can reach the bridge but not the disk.

`full_page: true` on `screenshot_panel` walks the page a screenful at a time
and stitches the tiles, rather than drawing into a canvas element. Every tile
is a real compositor capture of a real render, which is what keeps the shot
worth measuring against. Three details make it look like one picture instead of
six taped together, and all three are load bearing: anything fixed or sticky is
hidden after the first tile, the page is asked where it actually landed rather
than told where it should be (the last tile always lands short), and scroll
sync is suspended so the other panels do not follow this one down the page.

**Accessibility, per width.** `audit_accessibility` injects vendored axe-core
into every panel, runs it, and groups the violations by rule with the widths
each was broken at. A rule broken at some widths and not others is marked
`widthSpecific` and ranked first within its impact, because that is the kind a
tool testing one width cannot see. Panels are audited one at a time on purpose:
seven copies of axe walking seven DOMs at once makes every one of them slower,
and this is a measuring instrument. axe is vendored rather than fetched so the
audit works with no network and the version is the one the tests were written
against. The toolbar button runs it, the results open in their own sheet ranked worst
first, and each panel's label carries the count found at that width. The sheet
opens after the run and never before it: a sheet hides the panels, and a hidden
webview is not a laid out one, so contrast and target size would be measured
against nothing.

Violations deliberately do not join the queue `take_reports` drains. One queue
would be tidier, but a page with forty violations would bury the two notes a
person actually wrote, and those notes are the ones with a human judgement in
them.

Lighthouse is the wrong tool here and always will be: it drives Chrome over the
DevTools Protocol, and a panel is a WKWebView.

Not built, and deliberately so:

- **Bootstrap, Foundation and Bulma detectors.** Phase 6 in the plan.
- **Light mode.** Nothing in the palette blocks it.

## Working agreements

- **`tasks.md` at the repo root is the task list.** It is treated as text
  published under Rick's name, so no em dashes in it.
- **No em dashes** in the README, in `breakpoints-plan.md`, or in anything else
  that goes out under Rick's name. Chat replies and code comments are fine.
- **Commit permission has never been stated on this project.** Ask before
  committing.
- Writing `breakpoints.md` lands a file in someone's repo, possibly a client's.
  It is always an explicit action, never a timer, it never writes outside the
  project root, and it never touches `.gitignore` in either direction.
