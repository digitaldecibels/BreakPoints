# Break/Points tasks

## Run of 12 September 2026

Two hours and twenty two minutes, 64 commits, every task on this list done.
The test suite went from 170 to 214. Nothing was pushed.

**Done, in order.** The two you picked first, then top to bottom: four silent
failures, four width correctness fixes, nine performance fixes, seven small
fixes, three robustness items, sixteen scanner fixes, six features, and the
release QA pass.

**What the run found that was not on the list.**

- The window never opened where it was left. Screens cannot be enumerated
  before a window exists, so every remembered geometry was rejected and the
  window opened at the default size every single launch. Fixed, and the check
  now runs after the window is built.
- Every screenshot on this machine comes back blank. A capture of a single flat
  colour is now refused rather than written, and the message names the likely
  cause. See the known issue below.
- Collecting a note took it out of the queue and hoped. A reply that never
  arrived lost the note. Notes are now held until the caller asks again.
- Two premises on the list were stale and are recorded as such rather than
  fixed: the chrome reload crash did not reproduce in twelve attempts, and the
  scan hash mismatch does not happen.

**Known issues, both needing you rather than more code.**

1. **Screen recording permission.** Every capture is blank, which blocks
   screenshots, the baseline comparison and the picture attached to a note.
   macOS withdraws this permission whenever the binary changes and the app has
   been rebuilt many times today. Grant it in System Settings, Privacy and
   Security, Screen Recording, then restart Break/Points.
2. **The chrome reload crash.** Seen twice before this run, not reproducible in
   twelve attempts here. Written up in `CLAUDE.md` as a trap rather than a
   task, with what was tried.

**Not tested, and why.** Anything that needs eyes on the window: how it looks,
focus rings, hover states, and dragging the window to resize it. Anything that
needs the Web Inspector, which takes focus. The picture half of every capture
feature, for the permission above.

## Open

Six picked from the agent integration lists, 12 September 2026.

- [ ] **Drag a box instead of picking an element.** Some problems are the space
      between things rather than a thing. A dragged rectangle becomes the note,
      carrying the region and whatever is inside it.
- [ ] **Let a session ask a question.** A question with buttons in the status
      bar, answered without leaving the row. Holds the session's request open
      until it is answered, the way await_reports does.
- [ ] **Notes as a subscribable MCP resource.** Any MCP client subscribes and
      gets notes natively, with no WebSocket URL, no token in a query string and
      no long poll. Needs the MCP endpoint to support server-sent events, which
      it may not today.
- [ ] **The source file that drew the element travels with a note.** Resolve a
      reported element back to the template or component that produced it,
      through source maps and framework markers, so a session opens a file
      instead of searching for one. Hardest part is Twig and SDC, where there is
      no source map and the answer has to come from template comments or a debug
      mode.
- [ ] **The app writes the MCP config into the project.** Today registering the
      bridge is a command you copy out of Settings. Write `.mcp.json` in the
      project folder instead, so any session started there has the tools. Needs
      a decision about the token, which would then be in a file in the repo:
      probably gitignored, or read from the app's config at call time.
- [ ] **Preview a CSS change in the row without writing a file.** A tool that
      applies a stylesheet to every panel at once, so a proposed fix is seen at
      all widths before anything is edited. Cleared by a reload, and a matching
      tool to clear it on demand.

Add items as they come up. Anything finished comes off this list rather than
being marked done, and anything settled is written up in `CLAUDE.md` instead.
The completed run is summarised above; the work itself is in the git history,
one commit per task.

## Turned down rather than missed

- Signing and notarising the app, which it needs before it can run on another
  Mac.
- Comparing a signed-in and a signed-out render side by side, which one shared
  session makes impossible today.
- Returning screenshots as image data over MCP rather than as a file path. The
  reasoning is in `CLAUDE.md`.
