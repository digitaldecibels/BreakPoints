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

- [ ] **Verify Lando and DDEV certificates.** Panels fail to load
  `https://bucknell-be-the-ray.lndo.site` while `curl -k` reaches it, which
  points at the local certificate authority not being trusted by WKWebView. Find
  out whether this is a machine setup problem or something the app should
  detect and say out loud, since a failed first load is also what puts a panel
  into the state where it cannot report anything.
