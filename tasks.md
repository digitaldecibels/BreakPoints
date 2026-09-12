# Break/Points tasks

## Open

- [ ] **Reports, step 1: make notes survive and never get stranded.** The
  claim (which session notes are addressed to) and the queue of notes live only
  in memory, so any edit under `src-tauri`, a Vite reload or a crash wipes both.
  After that, new notes are addressed to nobody, and `take_reports_for` only
  returns exact matches, so a session polling by id never sees them again. The
  clipboard has the note; the session never does. Three changes, all in Rust:
  treat a note with no `client` as anyone's, so a session asking by id also
  gets notes written while nobody owned them; write the queue and the owner to
  a small file in the app data folder and read it back at boot; and move the
  prose formatting out of `store.js` into Rust as a `text` field on each note,
  so the clipboard, the bridge and the WebSocket below all say the same thing.

- [ ] **Reports, step 2: the toolbar count lies.** The chrome counts up on
  `report:new` and only counts down when the button is clicked. A session
  collecting over the bridge never tells the chrome, and clicking the count on
  an empty queue says "Nothing reported yet" and leaves the number standing.
  Have Rust emit a `reports:changed` event carrying the real count whenever a
  note is added or taken, and have the chrome show that number rather than
  keeping its own. Clear it on an empty copy too.

- [ ] **Reports, step 3: push instead of poll.** The whole delivery design
  rests on "the app cannot push into a session", and that is no longer true.
  Claude Code's Monitor tool has a WebSocket mode: it opens a socket and every
  incoming text frame arrives in the session as a notification, with no loop
  and no polling. Add a WebSocket route to the bridge, something like
  `/ws/reports?client=ID&name=NAME&token=TOKEN`. Connecting is the claim, so
  the toolbar name appears the moment the session connects. On connect, flush
  anything already waiting for that session; after that, send each note's
  `text` as one frame at the moment it is written. The token goes in the query
  string because the Monitor tool cannot send headers. This needs the `ws`
  feature on the axum line in `Cargo.toml` and a broadcast channel in
  `AppState`. While here, stop a socket from flipping `bridge_active`, which
  is what makes the toolbar dot pulse on every request (a two second poll
  blinks it thirty times a minute all day), and show a steady connected state
  instead.

- [ ] **Reports, step 4: rewrite `/run-breakpoints` down to three steps.**
  Check the port, start the app if it is down, then one Monitor call with the
  WebSocket URL and `persistent: true`. No claim request, no shell loop, and no
  "python3 <format each note as one line>" placeholder that every session
  fills in differently. Keep the curl route in a Traps section as the fallback
  for when the socket is unavailable. The skill lives at
  `~/.claude/skills/run-breakpoints/SKILL.md`.

- [ ] **Reports, step 5: run the built app for everyday use.** There is no
  built app anywhere; every session starts `npm run tauri dev`, which restarts
  on every Rust edit and dies on a Vite reload (see the task above about
  reloading the chrome). Run `npm run tauri build` once, keep the `.app`
  somewhere stable, and have the skill launch it with `open -g` so it never
  takes focus, falling back to the dev build only when the built one is
  missing. That removes the restart and reload problems for every project that
  is not Break/Points itself.

- [ ] **Reports, two decisions once push delivery works.** Whether the
  clipboard should still be overwritten on every note, which a preference
  would settle. And whether to keep the MCP registration in the Bucknell
  project's Claude Code config: it fails to connect whenever the app is not
  already running at session start, so the tools never appear, and the skill
  uses the HTTP API rather than MCP anyway.

- [ ] **Test the MCP server.** The agent bridge's HTTP API is exercised often and
  works, but `/mcp` and the stdio shim in `agent/stdio-shim.js` have not been
  driven by a real client end to end. Register it with Claude Code
  (`claude mcp add --transport http breakpoints http://127.0.0.1:7333/mcp
  --header "Authorization: Bearer $TOKEN"`), confirm `initialize` and
  `tools/list` answer, then call a read tool, a write tool and one that returns
  an image (`screenshot_panel`) and check each comes back in the shape a client
  expects. Do the same through the stdio shim for Codex. Cover the failure
  paths too: no token, a wrong token, and a tool called while no panels are
  open.

- [ ] **Reloading the chrome kills the app.** A Vite full page reload of the
  chrome webview ends the process cleanly, exit code 0, taking the window with
  it. It reproduced twice, each time immediately after a `[vite] (client) page
  reload` line. The chrome's `boot` runs again on reload, which tears the panel
  row down and rebuilds it, so the teardown is the place to look. Until this is
  fixed, any edit that triggers an HMR reload during a session ends the session.

- [ ] **The source-changed notice names the wrong file.** It shows
  `frameworks[0].sourceFile`, which on the Bucknell project is the Tailwind
  stylesheet, while every breakpoint it is talking about actually came from
  `be_the_ray.breakpoints.yml`. The Tailwind file declares no `--breakpoint-*`
  at all. Name the file the breakpoints came from, not the first framework
  found.

- [ ] **The source-changed notice fires when nothing has changed.** On the
  Bucknell project the scanner finds exactly the six widths written in
  `breakpoints.md`, but the recorded `sourceHash` does not match the one those
  widths produce, so the notice appears on every open. Find out what the hash
  was written from. The scanner now also reads `docs/starlight/src/styles/*.css`
  in that project, which is a documentation site rather than the site being
  tested, and may be one source of drift.

- [ ] **Accessibility checks in the editor.** Run an accessibility audit per
  panel and report violations with the breakpoint width attached, which is the
  same thesis as the Report feature: a violation that only exists at one width
  is invisible to a tool that tests one width. axe-core is the fit. It is a
  plain JavaScript library, injects into a WKWebView the way the picker script
  already does, and returns structured violations with selectors, so results
  can land in the label strip and in `take_reports` alongside hand-written
  notes. Lighthouse is the wrong tool here: it drives Chrome over the DevTools
  Protocol, which a WKWebView does not speak, so it would need a real Chrome
  running alongside the panels rather than the panels themselves. Worth adding
  later as a separate "audit this URL in Chrome" action if the Chrome work
  below happens.

- [ ] **Open a panel in a real browser, and let the person choose which.** The
  panels are WKWebView and cannot be anything else: Tauri renders through WRY,
  which uses the platform's native engine, and on macOS that is WebKit with no
  engine option. So the Inspect button will always be WebKit's Web Inspector,
  and Chrome DevTools can never attach to a panel, because DevTools speaks the
  Chrome DevTools Protocol and a WKWebView speaks WebKit's own remote inspector
  protocol. Different protocol, not a setting.

  What is achievable is a sibling to Inspect: open this panel's URL in the
  browser of your choice, sized to that breakpoint, so you get that browser's
  own devtools. Chrome takes `--app=URL --window-size=W,H --window-position=X,Y`
  and an isolated `--user-data-dir`; Firefox takes `-width`/`-height`; Safari
  takes neither and would need resizing by hand. The browser is a setting,
  listing what is installed, defaulting to the system default. Note that
  `--window-size` is the window and not the viewport, so the titlebar delta has
  to be measured once and added, the same correction `canvas::layout` already
  makes for panels.

- [ ] **Suspend the panels nobody can see.** The row is often far wider than the
  window: six Bucknell viewports come to about 5966px, and a 1400px window shows
  two or three of them. Every one of the others is a live webview holding a full
  render of a page nobody is looking at. Unloading an off-screen panel and
  restoring it when it scrolls back would be the largest memory saving available
  and would change nothing about what anyone sees, which is what makes it the
  right one to do first.

  Two things to get right. A suspended panel has to come back at its declared
  width, not at whatever the restore happens to produce, so it goes through the
  same frame arithmetic `canvas::layout` already owns. And scroll position has
  to survive, or panning back across the row resets every page to the top, which
  would be worse than the memory.

  Do not confuse this with rendering one page and scaling it to six widths. That
  cannot work: different widths fire different media queries and run different
  JavaScript branches, so they are genuinely different renders, and scaling one
  to stand in for another is exactly the lie this app exists to expose.

  Baseline with nothing loaded, measured 12 September 2026: 59MB main process,
  41MB across seven WebKit content processes. Measure six loaded pages before
  building anything, because it may turn out there is no problem here.

- [ ] **Verify Lando and DDEV certificates.** Panels fail to load
  `https://bucknell-be-the-ray.lndo.site` while `curl -k` reaches it, which
  points at the local certificate authority not being trusted by WKWebView. Find
  out whether this is a machine setup problem or something the app should
  detect and say out loud, since a failed first load is also what puts a panel
  into the state where it cannot report anything.
