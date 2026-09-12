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
