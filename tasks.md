# Break/Points tasks

## Open

- [ ] **Reloading the chrome sometimes kills the app.** Recorded as reproducing
  twice, each time straight after a `[vite] (client) page reload` line. It did
  not reproduce on 12 September 2026: one HMR reload from editing `index.html`
  plus three forced `location.reload()` calls, and each time the app stayed up
  with all seven panels intact and the claim restored from the snapshot. So it
  is intermittent rather than reliable, and the teardown in `boot` is still the
  place to look. Do not spend long on it without a reproduction.

- [ ] **A screenshot over MCP comes back as a file path, not an image.**
  `screenshot_panel` returns `{"path": "..."}` wrapped as a text block, where
  the shape an MCP client expects for an image is a content block with base64
  data and a mime type. Left as a path deliberately for now: the client is
  always on the same machine as the app, so it can read the file, and a
  full-page capture as base64 would be several megabytes in a single tool
  response. Worth revisiting only if a client that cannot read local files
  needs it.
