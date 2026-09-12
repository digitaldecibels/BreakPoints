# Break/Points tasks

## Open

- [ ] **Reloading the chrome sometimes kills the app.** Recorded as reproducing
  twice, each time straight after a `[vite] (client) page reload` line. It did
  not reproduce on 12 September 2026, across twelve reloads: one from editing
  `index.html` under Vite, three forced, five more issued while the panels were
  still loading from a `reload_all`, which is the race the teardown in `boot`
  would lose, and three back to back a second apart. The app stayed up every
  time with all seven panels intact and the claim restored from the snapshot.

  So it is intermittent at worst, and possibly already fixed by something
  since. Do not spend more time on it without a reproduction. If it happens
  again, the thing worth capturing is what the row was doing at the time,
  because none of the states tried here were enough.
