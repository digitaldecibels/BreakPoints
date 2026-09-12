# Break/Points tasks

## Open

### Performance

- [ ] **The pump asks panels nobody can see.** `canvas::start_pump` evaluates a
  drain script in every panel on every tick. Measured on 12 September 2026 with
  the Bucknell row: the row is 6365px wide in a 1400px window, so three panels
  of seven are on screen and the other four are asked for a scroll position
  that cannot have changed. Idle in the background costs 1.1% of a core, and
  the tick is ten times faster when the window has focus, which matches the 10
  to 12% already recorded for a focused row. More than half of that is waste.

  The fix is small and needs no new state: `Canvas` already holds `scroll_x`
  and every `Panel` holds `home_x` and `width`, so a panel is on screen when
  `home_x - scroll_x + width > 0` and `home_x - scroll_x < window width`. Keep
  asking a panel that still has messages queued from before it scrolled away,
  or its queue arrives late rather than never.

- [ ] **One wedged panel delays every panel behind it.** The pump drains panels
  in sequence, each with its own 500ms timeout, so a tick costs the sum of the
  slow ones rather than the slowest. Asking them concurrently bounds a tick at
  one timeout however many panels are open. Medium: the drain has to stay
  ordered per panel, and `dispatch_from` has to keep taking the panel id from
  the caller rather than from the message.

- [ ] **Console noise pins the poll loop at its fastest rate.** Any drained
  message resets `last_message` (`canvas.rs:1025`), and the injected script
  patches every `console` method, so a page that logs on a timer, a framework
  dev build, or a page that throws repeatedly keeps a message in the queue on
  every tick and holds the 16ms rate for the life of the session. That is
  roughly seven times the documented cost, sustained, on an app whose job is
  measuring how a page performs. Only scroll and wheel messages should refresh
  it, which is a filter over the drained array. Small, high value.

- [ ] **A window resize writes the whole config to disk every frame.**
  `WindowEvent::Resized` runs on the main thread and calls `remember_window`,
  which serialises every project record and every profile to a temp file and
  renames it, and macOS delivers that event continuously while you drag. The
  same frame also runs `relayout`, which is 21 inline webview operations and
  seven page re-layouts for a row of seven. This is the most visible stutter in
  the app and the only place it touches the disk on a per-frame path. Debounce
  the save to half a second after the last event and coalesce the layout to one
  per frame. Small.

- [ ] **The file watcher walks the entire project, including node_modules.**
  `add_root` on the debouncer walks the whole tree and stats every entry with
  no skip list, keeping the result for the life of the watch. On a Drupal
  project that is vendor plus `web/core` plus node_modules, so hundreds of
  thousands of stats and tens of megabytes resident, and it walks again on
  every directory that appears. The scanner has a careful skip list and
  `is_drupal_core`, and the watcher throws all of it away. Swapping to the
  no-cache debouncer is one constructor. The watcher's own skip list in
  `classify` also does not match the scanner's, so a `composer install` under
  `web/core`, or a checkout in `.claude/worktrees`, currently offers you a
  rescan of changes that are not the site.

- [ ] **Screenshot cropping and stitching are per-pixel loops.** `shots::crop`
  and the stitch in `walk_and_stitch` call `get_pixel` and `put_pixel` one
  pixel at a time, so a twenty-tile full-page capture on a Retina display is
  around 150 million bounds-checked calls, single-threaded. `crop_imm` and
  `replace` in the image library do both. Small, and worth roughly an order of
  magnitude on the slowest operation the app has. `references.rs` has the same
  pattern, and also writes a PNG and immediately reads and decodes it back.

### Correctness: silent failures

Each of these stops something working with nothing on screen to say so. The
first five were checked against the code by hand; the rest are from the review
and should be measured before they are fixed.

- [ ] **A panel can stop reporting for the whole session.** `ever_committed` is
  what tells the pump a panel is safe to ask. It is set only in the `Started`
  branch of `on_page_load` (`canvas.rs:815`), never in `Finished`, and only if
  the panel is already in `canvas.panels`. That lookup races the push that
  happens after `add_child` returns, so the task can find no panel and do
  nothing. Lose that race and the panel has no scroll sync, no console capture
  and no problem reports until the app restarts. The symptom is the Report box
  appearing to do nothing. Set it in both branches, and let `set_panel_state`
  own the lookup. Small, and the worst failure on this list.

- [ ] **Polling is keyed to the row's URL, not the panel's.** `start_pump`
  gates on `canvas.url.starts_with("https:")` (`canvas.rs:989`), and
  `canvas.url` only moves on spawn, navigate and a broadcast follow. A row
  opened on http that redirects to https keeps the old value, so every panel
  queues messages into `window.__bpOut` that nobody ever drains, until the
  queue hits its 300 cap and starts discarding. Reports, console capture and
  scroll sync die together and say nothing, which is the same failure already
  recorded as having cost three features at once. Give `Panel` its own flag set
  from the real per-document URL in `on_page_load`. Small.

- [ ] **A screenshot taken while the panels are hidden captures the chrome.**
  Nothing in `shots.rs` checks `panels_hidden` or `inspecting`, and panels are
  hidden whenever a sheet is open or one panel is being inspected. The capture
  returns a valid PNG of the sheet and reports success. The accessibility audit
  dodges this by running before it opens its sheet, but the guard lives only in
  that one caller. One shared check used by `shots::capture`, `audit::run` and
  `access::audit_all`, erroring with "the panels are hidden, so there is
  nothing to measure". Small. For a measuring instrument a confidently wrong
  picture is worse than an error.

- [ ] **A panel that fails on its own is marked loaded.** There is a complete
  mechanism for drawing a failed panel at its exact size, and the only thing
  that triggers it is the pre-flight URL probe at apply time
  (`project.rs:295`). A panel that fails afterwards, a followed link that 404s,
  a host that resolves for some panels only, or a TLS error on one, gets
  `Finished` on WebKit's own error page and is called loaded. You then see
  WebKit's error chrome instead of the app's, which loses the size. The
  injected script posts on every real document, so a panel that starts and
  never reports within a few seconds is failed. Medium.

### Correctness: the declared width

The declared width being exact is the property the whole app rests on. These
are the paths that can quietly break it.

- [ ] **Check the width instead of assuming it.** Every page already reports
  its own `innerWidth` on load, `callback::ready` forwards it to the window as
  `panel:ready`, and nothing anywhere listens to that event. The only code that
  compares it to the declared width is the note formatter, and only after a
  person has written a note. A `PanelState` for a mismatch, set in `ready` when
  the two differ by more than a pixel, is about thirty lines and would catch
  the next two items automatically, in production, on real sites. Small, and
  the highest value item in either review.

- [ ] **The first layout happens at the scaled width.** `spawn` creates the
  child at `LogicalSize(place.width, place.height)` and the URL starts loading
  immediately; `set_zoom(place.scale)` is a separate message that lands after.
  With fit-to-width on, a 1024 panel therefore begins life as a 512 viewport.
  WebKit re-lays out when the zoom arrives, but anything the page decides once
  at load does not re-run: a media query checked at parse time, a
  `DOMContentLoaded` handler reading `innerWidth`, a responsive image picking a
  source, a framework choosing a breakpoint on boot. Build the child on
  `about:blank`, apply the zoom, then navigate. Medium, and it needs testing
  against the panel that has never committed a navigation.

- [ ] **A width can be reported without being applied.** `relayout` updates the
  struct and skips the three webview calls while the panels are hidden
  (`canvas.rs:869`). `tools::set_viewport` changes a declared width, calls
  `relayout`, and returns a `PanelInfo` stating the new one. With a sheet open
  that is a claim about a webview still at the old frame and old zoom, so
  anything measuring in the interval gets the old width while the API reports
  the new. It heals when the sheet closes, which is what makes it hard to
  catch. Small to medium.

- [ ] **Nothing stops two row rebuilds running at once.** `spawn` is async,
  sleeps 20ms per panel, holds no exclusion, and is reachable from
  `apply_viewports`, `project::apply`, `project::restore` and two bridge tools.
  Interleaved, the second drains and closes the first's half-built list while
  the first keeps pushing panels computed from a different layout: positions
  from two generations, a total width describing neither, labels misaligned
  from the panels they name, and orphaned webviews. A generation counter
  checked before each push, or an async spawn lock. Medium.

### Small fixes worth taking

- [ ] **A typo in the URL bar blanks the whole row.** `util::normalize_url`
  returns `about:blank` for anything the parser rejects, with no error channel,
  and a pasted space is enough. Every panel then navigates to nothing, the row
  is gone, and the app reports success. The bridge's `navigate` tool has the
  same behaviour. Return a `Result` and let both say "that is not a URL", and
  keep `about:blank` for the genuinely empty case. Small, and there are few
  callers. While there: `starts_with("localhost")` also matches
  `localhosting.example`.

- [ ] **Opening the inspector may be able to hang the app.** `inspect_panel`
  is sync, so it runs on the main thread, and it holds the canvas lock across
  `open_devtools`, which takes focus. The focus handler calls
  `canvas::restore_frames`, which locks the same non-reentrant mutex on the
  same thread. If AppKit delivers that focus event inline, it is a freeze with
  no output. Not reproduced, and tao may queue the event instead, but the fix
  costs nothing: clone the webview handle out of the lock, drop the guard, then
  call. `set_inspecting` and `set_panels_hidden` have the same shape and are
  less exposed only because they are called off the main thread.

- [ ] **The watcher outlives the project it was watching.** The handle is only
  replaced inside `if let Some(root)` (`project.rs:284`), so applying a bare
  URL profile after having a project open leaves the old project's watcher
  running, and an edit in a repo you are no longer looking at still reloads
  your panels. One `else` branch. While there, the old debouncer is dropped
  while the mutex is held, which joins its thread under the lock.

- [ ] **A design reference is written into the repo unchecked.**
  `references::attach` writes the path straight into `breakpoints.md`, which is
  a file in someone else's repository, without checking that it exists or that
  it is inside the project root. Both checks already exist and run at compare
  time, which can be days after the typo was committed. Move them into
  `attach`. Small, and it matters because writing that file is meant to be a
  deliberate act.

### Features

- [ ] **Attach a picture of the element to every note.** A note already carries
  the element's rectangle, and `shots::capture` already crops a window capture
  to a panel rectangle. Joining the two would make a note self-contained, so
  whoever reads it sees the thing rather than a selector. Decide where the
  image goes: a file beside the screenshots with its path in the note is the
  cheap version, and it matches how screenshots are already handed over.

- [ ] **Push console errors down the socket.** `callback::console` already
  emits `panel:console` for every error and warning a page logs, and the window
  ignores that event completely, so an error that only happens at one width is
  invisible unless an agent goes looking with `get_console`. The socket that
  carries notes can carry these too, and the label strip can show a per-panel
  count the same way it now shows accessibility violations. Keep the two kinds
  distinguishable: a note has a person's judgement in it and a console line
  does not.

- [ ] **Before and after, across every width.** `references::diff` compares a
  panel against a static design reference. The same image comparison pointed at
  two captures of the same panel, before and after a change, would say which
  widths moved and which did not. This is the thing this app can do that a
  browser cannot, and most of the machinery is already there. The work is
  storing a baseline per panel and deciding what counts as a difference worth
  reporting.

- [ ] **Re-run the checks when the dev server rebuilds.** `watcher.rs` already
  debounces file changes and sorts them into two categories. Running
  `audit_all` or the accessibility checks on a rebuild would turn both from
  something you remember to do into something that tells you. Needs a way to
  turn it off, because a rebuild every few seconds should not start an audit
  every few seconds.

- [ ] **A line length probe.** `audit.rs` covers overflow, overlap, small type,
  upscaled images and tap targets, but not measure. Text running to 140
  characters at 1536px and 25 at 375px is a real per-width typography problem,
  and neither end shows up in any existing probe. Roughly 45 to 85 characters
  is the usual range. Small: it is one more probe in the same script.

- [ ] **Reporting takes two steps.** You arm the mode, then click. A
  modifier-click that describes an element without arming anything would make
  it one, and there is no keyboard shortcut for the mode either, while the
  chrome already has several. Keep the mode itself: a click that describes
  instead of following a link has to be something you asked for.

## Not in this list

Three robustness findings from the same review are deliberately not here,
because they were not asked for. Say so and they go in: nothing tests the
window, note delivery is take on read rather than acknowledged, and the app is
neither signed nor notarised.

The signed-in and signed-out comparison is not here either. It was turned down
rather than missed.
