# Break/Points features

A Mac app that reads a project's real breakpoints out of its code and opens a
live browser at each one, side by side.

Most responsive testing is a browser window being dragged, which shows you
every width except the ones that matter. Break/Points reads the widths your
project actually declares, in its Tailwind config, its Drupal breakpoints file
or its own stylesheets, and opens a real page at each of them at once. Every
panel is a full browser at its exact declared width, so what you see is what
the site does at that width rather than an approximation of it.

The second half of the app is the agent bridge: a local server that lets Claude
Code see and drive the row, so the thing measuring your layout and the thing
fixing it are looking at the same pixels.

Everything runs on your machine. The bridge listens on loopback only.

## The canvas

- **A panel per breakpoint.** One real browser per width, in a row you pan
  sideways.
- **Exact widths.** A panel renders at its declared width to the pixel, and the
  app now checks that against what the page reports rather than assuming it.
- **Scale to fit.** Shrink the whole row so more of it fits the window, without
  changing the width any page renders at.
- **Zoom the row out.** A slider from actual size to ten times smaller, so ten
  widths sit in the space one did. Every page still renders at its real width.
- **One scale for all.** Optionally scale every panel by the same factor so
  their relative sizes stay comparable.
- **Where the panels sit.** One group of three: hanging from the top, centred in
  the space, or stretched to the window's height. The width is never touched.
- **Scroll sync.** Scrolling one panel takes the others to the same place, and
  skips any panel already there.
- **Follow links.** A link clicked in one panel is followed in all of them,
  with a safety valve that turns it off if a site redirects on width.
- **Pan by trackpad or strip.** A sideways swipe anywhere pans the row, and a
  strip along the bottom edge does the same.
- **Per panel controls.** Reload, screenshot, inspect, and open this width in a
  real browser, from each panel's own label.
- **Open in a browser.** Hand one panel's page to Chrome, Brave, Firefox or
  Safari at that panel's width, for that browser's own devtools.
- **Web Inspector.** WebKit's own inspector on any panel, at the width it is
  really rendering at.
- **One session across the row.** Every panel shares cookies and storage, so
  signing in once signs you in at every width.

## Reading your project

- **Drop a folder.** Point the app at a project and it works out the rest.
- **Tailwind.** Reads `theme.screens` from a v3 config or `--breakpoint-*` from
  a v4 theme, wherever in the project that theme lives.
- **Tailwind presets.** Follows a preset that lives inside the project, and says
  so plainly when it is a published package it cannot read.
- **Drupal.** Reads every `*.breakpoints.yml`, including the two sided media
  queries Drupal documents.
- **Stylesheets.** Finds width based media queries in CSS, SCSS, Sass, Less and
  PostCSS files, and inside Vue, Svelte and Astro components.
- **Variables and maps.** Resolves a breakpoint written once in one file and
  used in fifty others, including map lookups and the mixins Bootstrap style
  projects are consumed through.
- **Modern query syntax.** Understands `width >= 48rem` and the double ended
  `48rem <= width < 64rem`.
- **Usage counting.** Counts how often each declared breakpoint is actually used
  in your templates, so a default nobody uses ranks below a width you lean on.
- **Weight.** Ranks a width by how much CSS was written behind it, not only by
  how many files mention it.
- **Conflicts.** Reports where two sources disagree about the same breakpoint,
  rather than picking a side.
- **What it discarded.** Says what it looked at and did not count, so an empty
  result is a report rather than a dead end.
- **Dev server detection.** Finds the URL to test from Lando, DDEV, Docker
  Compose, Vite, a framework default or a Rails app, and picks the one that
  answers.
- **Viewport generation.** Turns breakpoints into panels with real device
  heights, and can add the pixel below each one for edge testing.
- **Desktop first support.** For a max-width project it also opens the width
  where the rule applies, not only where it stops.
- **A diagnostics log.** Every detector, every file skipped and every parse
  failure, with a reason.
- **Scan honesty.** Says when a walk stopped early instead of presenting a
  partial answer as a complete one.

## breakpoints.md

- **A file in your repo.** The chosen widths, written as a readable table that
  becomes the source of truth.
- **Prose preserved.** Anything you write around the table survives a rewrite.
- **Watched.** Editing it reloads the panels; editing a config offers a rescan.
- **Change detection.** Notices when the code's breakpoints have moved since the
  file was written, and names the file they came from.
- **Profiles.** Saved viewport sets for when there is no project, switchable
  from the toolbar.

## Checking a page

- **Layout audit.** Probes every panel for sideways overflow, overlapping
  elements, small type, upscaled images, tap targets and viewport hungry fixed
  elements, ranked by severity.
- **Line length.** Measures how many characters fit on a line of running text
  and flags the ends of the range.
- **Accessibility per width.** Runs axe-core in every panel and marks the rules
  broken at some widths and not others, which is the kind a single width tool
  cannot see.
- **Ask the page.** Reads the media queries out of a panel's live stylesheet and
  says which open widths the page really changes at, and which widths it changes
  at that no panel covers.
- **Console errors per panel.** Counts what each page logged at each width, on
  the label, with the latest error a click away.
- **Width verification.** Flags any panel whose page disagrees with the width on
  its label, which is the one claim the app cannot afford to get wrong.
- **Re-check on change.** Optionally re-runs the layout checks when the
  project's stylesheets change.

## Reporting a problem

- **Point and describe.** Click what is wrong in any panel and say what is
  wrong; the width goes with it automatically.
- **One step.** A command and shift click describes an element without arming
  anything first.
- **A picture.** Each note carries a cropped image of the element it points at.
- **A standing instruction.** Your own sentence goes in front of every note,
  captured when the note is written.
- **Pushed to a session.** A note arrives in the Claude Code session you chose
  the moment you send it.
- **Never lost.** The queue survives a restart, a note is kept until the session
  proves it arrived, and a note nobody is listening for goes to your clipboard.
- **A status bar along the bottom.** The last note, permanently, rather than a
  line that fades after five seconds.
- **A mark you can trust.** A ring turns while the note is on its way and closes
  with a tick only once a listening session has been handed it.
- **An answer back.** What the session said about your note, shown beside it,
  with a button that brings that terminal forward to read the rest.
- **Start a session.** One button opens your terminal at the project, runs the
  agent, and the session claims your notes by connecting.
- **The sitting so far.** Click the bar for the last ten notes, each with what
  became of it and whatever was said back.
- **A mark on the width.** The label of a width you reported from carries a dot
  until the session answers, so the answer is tied to the width it was about.
- **Warned before you type.** The report form says a note will go to your
  clipboard before you write it, and offers to start a session there and then.
- **What the session is doing.** The bar names the tool a session is running,
  rather than only pulsing a dot.

## Screenshots and comparison

- **Screenshot a panel or the row.** Cropped from the window, at real rendered
  size.
- **Full page.** Walks a long page a screenful at a time and stitches it,
  hiding anything sticky after the first tile.
- **Design references.** Bind a Figma node or a local image to a width and
  compare the live panel against it.
- **Before and after.** Photograph every width, make a change, and be told which
  widths moved.

## The agent bridge

A local HTTP API with an MCP endpoint on top, off by default, loopback only, and
protected by a token. Thirty two tools, all of which run the same code the
buttons do.

- `list_panels`: every open panel, its width, state and the width its page
  reports.
- `navigate`: point every panel at a URL.
- `reload`: reload every panel.
- `set_viewport`: change one panel's size without touching the others.
- `eval_js`: run JavaScript in one panel and get the value back.
- `eval_chrome`: run JavaScript inside the app's own interface rather than a
  panel.
- `get_dom`: the HTML of the first element matching a selector.
- `get_console`: recent console output and errors from one panel.
- `screenshot_panel`: a PNG of one panel.
- `screenshot_all`: one PNG per panel, left to right.
- `take_baseline`: photograph every panel and keep the pictures.
- `compare_to_baseline`: photograph them again and say which widths moved.
- `diff_panel`: compare a panel against its design reference.
- `attach_reference`: bind a Figma node or local image to a width.
- `get_references`: which reference belongs to which panel.
- `audit_all`: the layout probes for every panel, ranked.
- `audit_accessibility`: axe-core in every panel, grouped by rule and width.
- `verify_breakpoints`: the live page's own media queries against the open
  widths.
- `detect_project`: point the app at a folder and open what it recommends.
- `scan_breakpoints`: re-run detection and return the report.
- `get_project_info`: the open project, its framework and its dev URL.
- `get_breakpoint_sources`: where each width came from, with file and line.
- `get_scan_log`: the full diagnostics log of the last scan.
- `generate_project_config`: turn the last scan into viewports without writing
  anything.
- `write_project_file`: write `breakpoints.md` and reload from it.
- `list_profiles`: saved viewport sets.
- `load_profile`: switch to one.
- `claim_reports`: address problem reports to this session.
- `take_reports`: collect the notes waiting for this session.
- `await_reports`: wait for the next note rather than checking for one.
- `reply_to_report`: say something back about a note, shown in the app.
- `explain_report`: everything about one note at once, including the same
  element measured at every open width and the rules that match it.

There is also a menu in the toolbar listing the skills installed on your machine
and in the project, with each one's own description, so asking a session to run
one needs no memory of what it is called.

There is also a WebSocket, `/ws/reports`, which pushes each note to a session as
it is written and can carry console errors too, and a stdio shim for clients
that speak that instead.

## Settings and shortcuts

- **Viewports.** Reorder, rename, resize, add and remove the widths in the row.
- **Panel heights.** From real device ratios, one fixed height, or whatever you
  type.
- **Edge testing.** Also open one pixel below each breakpoint.
- **Screenshot folder.** Per project, shown in full rather than as a default.
- **Browser choice.** Which browser "open in browser" uses, listing what is
  actually installed.
- **Terminal choice.** Which terminal the session button opens, listing what is
  actually installed, and the command it runs there.
- **The report instruction.** The sentence sent with every note.
- **Shortcuts.** Command L focuses the address bar, Command R reloads the row,
  Command Shift R toggles reporting, Command Option I opens the app's own
  inspector, and the arrow keys pan.

## Deliberately not built

- **Bootstrap, Foundation and Bulma detectors.** A Bootstrap project is still
  read through its SCSS map and mixin calls.
- **Light mode.** Nothing in the palette blocks it.
- **Chrome DevTools on a panel.** Impossible, not unimplemented: a panel is a
  WKWebView and speaks a different protocol. Opening the page in a real browser
  is the answer instead.
