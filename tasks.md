# Break/Points tasks

## Open

- [ ] **Put the built app somewhere stable.** `npm run tauri build` produces
  `src-tauri/target/release/bundle/macos/BreakPoints.app`, and that is what
  `/run-breakpoints` launches today. It is inside `target/`, so `cargo clean`
  removes it. Copying it to `~/Applications` once makes the first line of the
  skill work and survives a clean. Attempted during the session that added it
  and declined at the permission prompt, both for `/Applications` and for
  `~/Applications`, so it is a decision rather than a missing step.

- [ ] **Accessibility results have no place in the window.** The audit is built
  and works: `audit_accessibility` over the bridge runs axe-core in every panel
  and returns the violations grouped by rule, each carrying the widths it was
  broken at, with the ones that only happen at some widths marked and ranked
  first. What is missing is a way to run it without an agent. A toolbar button,
  a count per panel in the label strip, and a way to see which rule broke at
  which width. Decide too whether a violation should join the notes queue that
  `take_reports` drains: one queue is tidy, but a page with forty violations
  would bury the handful of notes a person actually wrote.

- [ ] **Reloading the chrome sometimes kills the app.** Recorded as reproducing
  twice, each time straight after a `[vite] (client) page reload` line. It did
  not reproduce on 12 September 2026: one HMR reload from editing `index.html`
  plus three forced `location.reload()` calls, and each time the app stayed up
  with all seven panels intact and the claim restored from the snapshot. So it
  is intermittent rather than reliable, and the teardown in `boot` is still the
  place to look. Do not spend long on it without a reproduction.

- [ ] **Suspending off-screen panels is probably not worth building.** Measured
  on 12 September 2026, with seven Bucknell pages loaded from the built app:

  | | Nothing loaded | Seven pages loaded |
  | --- | ---: | ---: |
  | Main process | 59 MB | 64 MB |
  | WebKit content processes | 41 MB | 112 MB |
  | Total | 100 MB | 176 MB |

  So a loaded page costs about 11 MB over a blank one, and suspending the four
  or five nobody can see would save roughly 50 to 60 MB against a 176 MB
  footprint. That is a modest saving for a change that has to restore a panel
  at its exact declared width and keep its scroll position, both of which are
  places this app has been bitten before. Recommendation is to leave it, and
  the numbers above are why. Revisit if a project turns up with many more
  breakpoints or much heavier pages.

  Do not confuse this with rendering one page and scaling it to six widths.
  That cannot work: different widths fire different media queries and run
  different JavaScript branches, so they are genuinely different renders, and
  scaling one to stand in for another is exactly the lie this app exists to
  expose.

- [ ] **A screenshot over MCP comes back as a file path, not an image.**
  `screenshot_panel` returns `{"path": "..."}` wrapped as a text block, where
  the shape an MCP client expects for an image is a content block with base64
  data and a mime type. Left as a path deliberately for now: the client is
  always on the same machine as the app, so it can read the file, and a
  full-page capture as base64 would be several megabytes in a single tool
  response. Worth revisiting only if a client that cannot read local files
  needs it.
