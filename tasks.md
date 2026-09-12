# Break/Points tasks

## Open

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
