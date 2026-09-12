# Break/Points tasks

## Open

### Correctness: silent failures

Each of these stops something working with nothing on screen to say so. The
first five were checked against the code by hand; the rest are from the review
and should be measured before they are fixed.

- [x] **A panel can stop reporting for the whole session.** `ever_committed` is
  what tells the pump a panel is safe to ask. It is set only in the `Started`
  branch of `on_page_load` (`canvas.rs:815`), never in `Finished`, and only if
  the panel is already in `canvas.panels`. That lookup races the push that
  happens after `add_child` returns, so the task can find no panel and do
  nothing. Lose that race and the panel has no scroll sync, no console capture
  and no problem reports until the app restarts. The symptom is the Report box
  appearing to do nothing. Set it in both branches, and let `set_panel_state`
  own the lookup. Small, and the worst failure on this list.

- [x] **Polling is keyed to the row's URL, not the panel's.** `start_pump`
  gates on `canvas.url.starts_with("https:")` (`canvas.rs:989`), and
  `canvas.url` only moves on spawn, navigate and a broadcast follow. A row
  opened on http that redirects to https keeps the old value, so every panel
  queues messages into `window.__bpOut` that nobody ever drains, until the
  queue hits its 300 cap and starts discarding. Reports, console capture and
  scroll sync die together and say nothing, which is the same failure already
  recorded as having cost three features at once. Give `Panel` its own flag set
  from the real per-document URL in `on_page_load`. Small.

- [x] **A screenshot taken while the panels are hidden captures the chrome.**
  Nothing in `shots.rs` checks `panels_hidden` or `inspecting`, and panels are
  hidden whenever a sheet is open or one panel is being inspected. The capture
  returns a valid PNG of the sheet and reports success. The accessibility audit
  dodges this by running before it opens its sheet, but the guard lives only in
  that one caller. One shared check used by `shots::capture`, `audit::run` and
  `access::audit_all`, erroring with "the panels are hidden, so there is
  nothing to measure". Small. For a measuring instrument a confidently wrong
  picture is worse than an error.

- [x] **A panel that fails on its own is marked loaded.** There is a complete
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

- [x] **Check the width instead of assuming it.** Every page already reports
  its own `innerWidth` on load, `callback::ready` forwards it to the window as
  `panel:ready`, and nothing anywhere listens to that event. The only code that
  compares it to the declared width is the note formatter, and only after a
  person has written a note. A `PanelState` for a mismatch, set in `ready` when
  the two differ by more than a pixel, is about thirty lines and would catch
  the next two items automatically, in production, on real sites. Small, and
  the highest value item in either review.

- [x] **The first layout happens at the scaled width.** `spawn` creates the
  child at `LogicalSize(place.width, place.height)` and the URL starts loading
  immediately; `set_zoom(place.scale)` is a separate message that lands after.
  With fit-to-width on, a 1024 panel therefore begins life as a 512 viewport.
  WebKit re-lays out when the zoom arrives, but anything the page decides once
  at load does not re-run: a media query checked at parse time, a
  `DOMContentLoaded` handler reading `innerWidth`, a responsive image picking a
  source, a framework choosing a breakpoint on boot. Build the child on
  `about:blank`, apply the zoom, then navigate. Medium, and it needs testing
  against the panel that has never committed a navigation.

- [x] **A width can be reported without being applied.** `relayout` updates the
  struct and skips the three webview calls while the panels are hidden
  (`canvas.rs:869`). `tools::set_viewport` changes a declared width, calls
  `relayout`, and returns a `PanelInfo` stating the new one. With a sheet open
  that is a claim about a webview still at the old frame and old zoom, so
  anything measuring in the interval gets the old width while the API reports
  the new. It heals when the sheet closes, which is what makes it hard to
  catch. Small to medium.

- [x] **Nothing stops two row rebuilds running at once.** `spawn` is async,
  sleeps 20ms per panel, holds no exclusion, and is reachable from
  `apply_viewports`, `project::apply`, `project::restore` and two bridge tools.
  Interleaved, the second drains and closes the first's half-built list while
  the first keeps pushing panels computed from a different layout: positions
  from two generations, a total width describing neither, labels misaligned
  from the panels they name, and orphaned webviews. A generation counter
  checked before each push, or an async spawn lock. Medium.

### Performance

- [x] **The pump asks panels nobody can see.** `canvas::start_pump` evaluates a
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

- [x] **One wedged panel delays every panel behind it.** The pump drains panels
  in sequence, each with its own 500ms timeout, so a tick costs the sum of the
  slow ones rather than the slowest. Asking them concurrently bounds a tick at
  one timeout however many panels are open. Medium: the drain has to stay
  ordered per panel, and `dispatch_from` has to keep taking the panel id from
  the caller rather than from the message.

- [x] **Console noise pins the poll loop at its fastest rate.** Any drained
  message resets `last_message` (`canvas.rs:1025`), and the injected script
  patches every `console` method, so a page that logs on a timer, a framework
  dev build, or a page that throws repeatedly keeps a message in the queue on
  every tick and holds the 16ms rate for the life of the session. That is
  roughly seven times the documented cost, sustained, on an app whose job is
  measuring how a page performs. Only scroll and wheel messages should refresh
  it, which is a filter over the drained array. Small, high value.

- [x] **A window resize writes the whole config to disk every frame.**
  `WindowEvent::Resized` runs on the main thread and calls `remember_window`,
  which serialises every project record and every profile to a temp file and
  renames it, and macOS delivers that event continuously while you drag. The
  same frame also runs `relayout`, which is 21 inline webview operations and
  seven page re-layouts for a row of seven. This is the most visible stutter in
  the app and the only place it touches the disk on a per-frame path. Debounce
  the save to half a second after the last event and coalesce the layout to one
  per frame. Small.

- [x] **The file watcher walks the entire project, including node_modules.**
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

- [x] **Screenshot cropping and stitching are per-pixel loops.** `shots::crop`
  and the stitch in `walk_and_stitch` call `get_pixel` and `put_pixel` one
  pixel at a time, so a twenty-tile full-page capture on a Retina display is
  around 150 million bounds-checked calls, single-threaded. `crop_imm` and
  `replace` in the image library do both. Small, and worth roughly an order of
  magnitude on the slowest operation the app has. `references.rs` has the same
  pattern, and also writes a PNG and immediately reads and decodes it back.

- [ ] **Scroll sync tells every panel, unconditionally.** A scroll message from
  one panel eval-pushes `__bpApplyScroll` into every other panel with no check
  on where they already are, which for seven panels is roughly 42 evaluations
  per throttle window during a steady scroll. `Panel::last_scroll_pct` is
  exactly the state needed to skip a panel already within a pixel or two of the
  target: it is declared, written once when the panel is created, and never
  read or updated again. Populating it removes most of the fan-out, because the
  panels converge, and it also gives an agent a per-panel scroll position that
  currently needs an `eval_js` per panel to discover. Small.

- [ ] **The hot loop allocates for nothing.** Every drain calls `eval_js`,
  which starts with `require_panel` and `resolve_id`, and that clones every
  viewport in the row to match an id it was already handed. It then builds a
  900 byte wrapper with `format!` and parses the result JSON twice. At the
  fastest tick with seven panels that is about 440 times a second. It will not
  beat the cost of the round trip into WebKit, but an `eval_js_by_id` that
  skips resolution and a static drain script remove all the avoidable work
  around it. Small.

- [x] **A wheel event parks a worker thread twice.** `nudge_scroll` asks the
  window for its scale factor and inner size, and both of those send a message
  to the main thread and then block on an unbounded receive. Called off the
  main thread, which is always, that is two blocking round trips per wheel
  event against a main thread simultaneously laying out seven pages. If the
  main thread is inside a modal or the Web Inspector, the receive has no
  timeout and the worker is parked indefinitely. Cache the window's logical
  width in `AppState` from the resize handler that already runs, and the getter
  disappears. Small.

### Small fixes worth taking

- [ ] **A screenshot of a window that is behind something comes back blank.**
  Found on 12 September 2026 while replacing the per-pixel crop, not looked
  for. Every capture taken during the run came back as a single flat colour at
  the right dimensions, while one taken yesterday has real content, and the new
  crop was proved pixel-identical to the old loop by a unit test, so the
  blankness is in `capture_window` rather than anything downstream. The window
  was behind a terminal throughout, which is the obvious suspect and is not
  proven: confirm by taking one with Break/Points frontmost.

  A guard now refuses to write a capture that is one flat colour, so nothing
  hands over a blank PNG as a success any more. What is still unknown is
  whether an occluded window can be captured at all on this macOS version. If
  it cannot, the app should say so once rather than on every attempt, and the
  agent tools should say the window has to be visible. That matters because the
  window is deliberately opened unfocused and left behind whatever you are
  doing.

- [ ] **The window does not open where it was left.** Found on 12 September
  2026 while working on the resize path, not looked for. The config records
  `{x: 61, y: 32, width: 5059, height: 1331}`, the display is 5120 by 1440
  logical, and the window opens at 1400 by 872, which is the builder's default.
  Walking `fits_on_a_screen` by hand against those numbers returns true, so
  either `available_monitors()` is failing, in which case that function returns
  false for everything and the remembered geometry can never be used, or macOS
  is refusing the size and nothing notices. The saved value is right and the
  restore is what is broken. It matters more than it looks: the window is
  deliberately opened unfocused and in its remembered place so it never lands
  in front of what someone is doing.

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

- [ ] **A capture leaves the row where it finished.** `bring_into_view` pans
  the row to put a panel on screen and nothing pans it back, so capturing every
  panel walks the row and leaves you wherever the last one was. The same
  function saves the scroll sync setting, forces it off, and writes the old
  value back unconditionally, so a toggle made during the several seconds a
  full-page capture takes is silently reverted. Remember and restore the scroll
  offset, and use a suspend counter rather than a save and restore for sync.
  Small.

### Robustness

- [ ] **Nothing tests the window.** There are 184 unit tests and not one that
  starts the app. The failures this codebase actually hits are a missing
  permission file, a missing capability line, and the panel width arithmetic,
  and a smoke test that boots the app, opens a project and asserts each panel's
  reported width against its declared width would catch all three. It is also
  the natural home for the width check above. Medium.

- [ ] **Note delivery is take on read.** Collecting a note removes it from the
  queue, so a note handed to an HTTP response that never arrives is gone. The
  socket path recovers correctly now, because a failed send puts the rest back,
  but `take_reports` and `await_reports` do not. An acknowledgement step, where
  a note is marked handed over and only cleared when the caller says it arrived,
  would make both paths at least once. Medium. Signing and notarising the app
  was considered alongside this and deliberately left out.

- [ ] **The scan deadline is only enforced in two of five detectors.** The walk
  and the CSS detector both check `index.out_of_time()`; the Tailwind, Drupal
  and dev server detectors never do. Since Tailwind runs first and reads the
  entire stylesheet corpus, the detector most likely to blow the ten second
  deadline is the one that cannot see it. On a network volume or a cold file
  system that is how the scan sheet hangs. Small.

### Scanner: wrong or missing widths

- [ ] **A named list of widths loses its names and its status.** `css.rs`
  correctly pulls a map like `(sm: 576px, md: 768px, lg: 992px)` out of a
  variable, then hands each entry to `Occurrence::named`, which is an empty
  function (`css.rs:170`). The names are discarded, so five deliberate,
  configured breakpoints arrive as five anonymous widths at low confidence.
  Downstream, `generate.rs` sorts by how many files each appears in, they all
  tie at one, and the three narrowest are the ones opened as panels while the
  two that designs actually break at are merely offered. The comment above that
  function states the intent; the code does the opposite. Small, and it is the
  most common convention outside Tailwind.

- [ ] **A variable defined in one file and used in fifty resolves nowhere.**
  `scss_lengths` is built per file, inside the per-file loop, so the universal
  layout of one variables file plus fifty partials finds nothing: the defining
  file has no queries, and every user logs "depends on a value this file does
  not define". The project reports zero CSS breakpoints. Map lookups and the
  mixins that Bootstrap and every sass-mq derivative are consumed through have
  the same problem. Two passes over text that is already read: one to collect
  names and lengths across the whole project, one to resolve. Medium, and it is
  the structural fix the two items below lean on.

- [ ] **A Tailwind v4 theme in its own file is never opened.** `tailwind.rs:69`
  skips any file that does not contain the string `tailwindcss`, and the
  documented v4 pattern is an entry stylesheet that imports both the framework
  and a separate theme file holding the `--breakpoint-*` block. The theme file
  never mentions the framework, so it is skipped, and the entry file is then
  read as declaring nothing, which makes the app report Tailwind's stock five
  widths as if the project had chosen them. Scan every non-compiled stylesheet
  for the custom properties and use the marker only to decide the framework
  label. Small.

  While there: a config with both `theme.screens` and `theme.extend.screens` is
  valid Tailwind and currently re-adds the five defaults the project deleted,
  because the extend flag is set from either site.

- [ ] **Two-sided range queries match nothing.** The pattern in `units.rs`
  handles `(width >= 48rem)` and `(768px <= width)`, but both alternatives
  require a closing bracket straight after the operand, so
  `(48rem <= width < 64rem)` matches neither. That is the form the range syntax
  exists for and what PostCSS Preset Env emits. It is logged as having no width
  component, which is untrue. Same regex, same size of fix. Small. A width
  written with `calc()` or a custom property also returns nothing and at least
  deserves its own log reason, because that line is what a person reads to
  decide whether the detector is broken.

### Scanner: whole kinds of project missed

- [ ] **Component styles are never read.** `walk.rs` indexes `css`, `scss` and
  `sass` only. A Nuxt, SvelteKit or Astro project keeps nearly every query in a
  component's own style block, so those projects find zero widths and fall
  through to generic device sizes. Extracting style blocks from `.vue`,
  `.svelte` and `.astro` is medium work and is the biggest miss by number of
  projects.

- [ ] **Common stylesheet extensions are left out.** `.pcss` and `.postcss` are
  the convention in PostCSS and Tailwind setups, including a v4 theme file, and
  `.less` covers a lot of older Drupal and WordPress themes. Small, and it is
  the same change as above without the extraction.

- [ ] **Commented-out queries count as real ones.** The prelude regex runs over
  raw text, so a query inside a comment matches and inflates the file count,
  which is the only confidence signal the CSS side has. `jsobj::blank_comments`
  already blanks comments while preserving offsets, so this is one line of
  reuse.

- [ ] **Unminified build output is read as source.** `looks_compiled` needs
  more than 400 bytes per line, so any build run without minification, which
  includes a plain Tailwind CLI run, most dev builds and Drupal's own
  unaggregated output, passes as source. For Tailwind that means generated
  values are read back as configuration at high confidence; for everything else
  the compiled file and its source both contribute and every width's confidence
  doubles. Cheaper tells exist: a framework banner comment, a `--tw-` property,
  a source map comment, or a `.css` whose stem matches a `.scss` elsewhere in
  the tree. Medium.

### Scanner: alarms that are not real

- [ ] **A Drupal breakpoint conflicts with itself.** Drupal's own documented
  form, `all and (min-width: 560px) and (max-width: 850px)`, yields two
  discoveries with the same label from the same line. `mod.rs` then groups by
  name, finds two entries called the same thing at different widths, and
  reports a conflict between a file and itself, while `generate.rs` produces
  two identically named panels. Skip the conflict when both sides share a
  source file and source, and disambiguate the second name. Small.

- [ ] **The near-miss conflict window is too wide.** Two widths are called a
  conflict when the gap is within five percent of the configured one, which at
  1536 is plus or minus 76 pixels, so a deliberate 1470 used in two files gets
  flagged. One CSS width can also conflict with two different configured widths
  and the loop does not dedupe, so a single near miss can produce several
  cards. Cap it at the smaller of five percent and about 32 pixels, which keeps
  the case it was written for. Small.

- [ ] **The root font size warning is wrong.** Media Queries Level 4 resolves
  relative units in a query against the initial font size, never against
  declarations, so the widespread 62.5% technique does not move any breakpoint.
  The warning fires on a large share of real projects and tells the user the
  numbers on screen may be wrong when they are not. Delete it, or reword it for
  the case it is right about, which is rem lengths elsewhere in the CSS rather
  than query boundaries. Small, and it matters for trust.

- [ ] **A config the scanner could not read outranks the CSS it could.** When
  Tailwind's screens cannot be resolved statically, `tailwind.rs` does the
  right thing and emits the shipped defaults with a warning, but it emits them
  as `Kind::Framework`. `generate.rs:120` asks only about the kind, so the
  automatic checks drop to nothing and the project's own widths, measured
  across nine files each, are left unchecked while five guesses are opened as
  panels. The confidence value set to record that uncertainty is never read by
  anything. Give a fallback its own kind and make the framework test mean "a
  framework whose values we actually read". Medium.

### Scanner: waste, and telling the truth about a scan

- [ ] **Every stylesheet is read twice and charged twice.** `ReadBudget` has no
  cache, and `tailwind::run` reads every CSS file in full before testing
  whether it wants it, so the whole corpus is read and billed, then read and
  billed again by the CSS detector that runs after it. The ten megabyte budget
  is really five, disk work is doubled, and on a heavy project the CSS detector
  can be handed a budget already spent, at which point it reports nothing and
  the log blames files nobody has opened. Memoise by relative path. Small, and
  it is what makes the cross-file variable pass cost nothing.

  Related: Drupal's aggregated CSS under `web/sites/*/files` is not in the
  walk's skip list, and `tailwind.rs` does not apply the ignore markers that
  `css.rs` does, so those files are read in full before being rejected. The
  gitignore rules cover it only inside a git repository, which a delivered zip
  or a fresh checkout is not.

- [ ] **A truncated scan looks exactly like a complete one.** `report.truncated`
  is set correctly, serialised, and read by nothing in the window, so a scan
  that hit the file cap presents its partial answer with full confidence. The
  scan row's "files read" is also the count of files the index kept, not the
  count read, and after a budget stop those two diverge a lot. That line is the
  one place a person learns how much of their project was looked at. Small.

- [ ] **An answer can be stale on reopen.** The cache key is ten root-level
  filenames and includes no stylesheet, no `*.breakpoints.yml` and no nested
  framework config. Edit the file the breakpoints came from while the app is
  closed, reopen the project, and the previous answer comes back with no sign
  it is old. The rescan command bypasses the cache and the watcher catches
  changes while the app is open, so this only bites on reopen and on restore at
  boot, which is the path meant to be instant. Fold in the modification times
  of the files the breakpoints actually came from. Small.

- [ ] **Say what was discarded, in the window and not only in the log.** The
  scan log already records every skip and every discard with a reason, and the
  sheet shows none of it. Two counts in the empty state, "34 container queries,
  which are not viewport widths" and "18 queries needed a value defined in
  another file", turn "No breakpoints found" from a dead end into a bug report.
  This is the scanner's own explain-what-you-discarded rule applied to the case
  where it matters most. Small.

### Scanner: remaining gaps

- [ ] **A monorepo merges every config it finds.** Every framework config in
  the tree contributes its widths to one list, so four apps with four different
  screen sets produce their union while the sheet names a single file as the
  source. The same happens with Drupal breakpoint files across a theme, a
  subtheme and any custom modules. Prefer the config nearest the root, or the
  one whose directory contains the chosen dev server, and report the others as
  a conflict rather than merging them. That is what the scanner's own
  surface-a-disagreement rule asks for, and this is the case it does not cover.
  Medium.

- [ ] **Configs compete with stylesheets for the file cap.** The 3000 file keep
  limit is one pool covering both, and the walk is in directory order, so on a
  large monorepo the cap can be spent before the walk reaches the app whose
  config is the actual answer. Configs are never numerous, so they should never
  draw on that budget: two keep lists, one uncapped for configs and one capped
  for stylesheets. Small.

- [ ] **Rails is never detected, and a monorepo gets no default port.** Nothing
  looks for a Rails startup file, a `Procfile.dev` or a `Gemfile`, and those
  files are not indexed either, so a Rails app never gets its localhost URL. In
  a monorepo the root package file carries no framework dependency, so no
  default port is offered there either. Small each, medium if a port should be
  read out of an env file.

- [ ] **Desktop-first projects get panels at untested widths.** The scanner
  works out which side of each boundary a query applies to and then never uses
  it: the only consumer is a collapse helper that nothing calls. For a project
  written desktop-first, every boundary is one pixel below where its own rules
  take effect, so without edge testing switched on, not one panel renders at a
  width where the project's rules apply. Using the edge to place the panel, or
  to turn edge testing on by itself for max-side discoveries, is a few lines
  and changes what you actually see. Small.

### Scanner: raising confidence in every width

- [ ] **Confirm framework widths against real usage.** Index the templates and
  components, `.twig`, `.html`, `.jsx`, `.tsx`, `.vue`, and count how often
  each breakpoint is actually referenced, through Tailwind variant prefixes or
  mixin calls. A default that appears in 400 class attributes is evidence; one
  that appears nowhere is an assumption the project never uses. This converts a
  framework guess into a measurement, feeds the file count the merge already
  carries, and gives the recommender a reason to check the widths a project
  leans on rather than all five. It is also the signal the fallback problem
  needs in order to stop preferring guessed defaults. Medium, and the best of
  the three bets.

- [ ] **Rank by how much CSS sits behind a width.** Confidence today is how
  many files mention a width. A breakpoint with four kilobytes of rules behind
  it is a layout decision; one with a single `display: none` is a tweak. The
  prelude parser already returns where each query starts, so matching its
  closing brace gives the byte length of the body. Better signal, from data
  already in hand. Medium.

- [ ] **Ask the page, not the codebase.** There is a real browser open at every
  width holding the fully resolved stylesheet: the preprocessor has run,
  imports are resolved, custom media is expanded, and container queries are
  distinguishable from viewport ones. Reading the media rules out of the page is
  ground truth for the exact question the scanner infers. It cannot be a normal
  detector, because it needs a panel rather than a file index, but it could
  raise a CSS discovery to confirmed or flag a configured width the browser
  never sees. Large, and it is the direction everything else here points at.

- [ ] **Follow Tailwind presets.** A config that takes its screens from a
  shared preset package, which is the standard design-system setup, resolves to
  nothing, and the shipped defaults are emitted with only a log note. A preset
  that resolves to a path inside the project, a sibling workspace package, can
  be parsed by the same code. One outside the project should warn rather than
  quietly guess, since published packages are never indexed. Medium.

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

Two things were offered and turned down rather than missed: signing and
notarising the app, and comparing a signed-in and a signed-out render side by
side.
