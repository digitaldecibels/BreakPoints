# Break/Points feature requests

Things worth building that are not yet decided on. Each one says what it is, why
it would earn its place, what it would cost, and what has to be settled before
anybody starts.

This is not the task list. `tasks.md` holds work that has been agreed and is
waiting to be done. A request moves from here to there when the open questions
at the bottom of its section have answers.

## Send a note to a ticket tracker

### What it is

A note written in Break/Points becomes a ticket in whatever system the project
already uses. You click the thing that is wrong at 375px, describe it, and it
arrives in Jira or Asana with the width, the page, the element and a picture
already filled in, rather than being retyped by somebody an hour later.

### Why it would earn its place

The app already captures more about a problem than a person will ever type into
a ticket by hand. A note carries the breakpoint width, the panel it was written
in, what the page reported its own width to be, the URL, the page title, the
element, a CSS selector, the element's rectangle, a cropped picture, and the
time. A ticket filed by hand carries a sentence and, if you are lucky, a
screenshot of the whole window.

So the value is not "one less copy and paste". It is that the ticket is better
than the one a person would have written, because the app was standing there
when the problem was found and the person was not thinking about ticket
hygiene.

The second reason is the audience. Today a note goes to an agent session, or to
the clipboard. Both assume the person who will fix it is you, now. A client, a
project manager, or a developer who is not in this session has no route in at
all.

### What a note already carries

Everything in this list exists today and is available at the moment a note is
written. Nothing new has to be captured.

| What | Example | Where it would go in a ticket |
| --- | --- | --- |
| The description | "The header sits too close to the top edge" | Title, and the first line of the body |
| Breakpoint width | 375 | A custom field, and the title suffix |
| Reported width | 375 | The body, and flagged when it disagrees with the declared width |
| Panel name | "Small iPhone" | The body |
| Page URL | `https://example.com/about` | The body, as a link |
| Page title | "About us" | The body |
| Element | `header#nav` | The body |
| CSS selector | `#nav` | A custom field, so it can be searched |
| Element rectangle | x, y, w, h | The body, or dropped |
| Picture of the element | a cropped PNG | An attachment |
| Written at | a timestamp | The ticket's own created date |
| The reply, if a session answered | "already correct at this width" | A comment added after the fact |

The title is the only field that needs inventing, and the obvious rule is the
note's first sentence with the width appended: `Header sits too close to the top
edge (375px)`.

### Which trackers, and what each one actually needs

Researched against current documentation rather than from memory, in September
2026. Two corrections to the obvious assumptions came out of it and both change
the plan.

**Every one of these accepts a pasted token.** None of the eleven forces OAuth
on a desktop app. So "paste a token" is not a compromise to apologise for, it is
the mainstream path, and the adapter work is uniform.

**A desktop app can do OAuth anyway, except for Jira.** The loopback redirect is
the documented pattern for native apps: the app opens the system browser and
listens on 127.0.0.1 for the code coming back. GitHub also offers a device flow
needing no secret at all. Atlassian is the real exception, because its token
endpoint accepts no public clients, so for Jira a pasted token is the only route
that does not mean shipping a secret inside the app.

| Tracker | Credential | Header | Forced expiry |
| --- | --- | --- | --- |
| Jira Cloud | API token | `Basic base64(email:token)` | 365 days, no opt-out |
| GitHub | Fine-grained PAT | `Authorization: Bearer` | Optional |
| GitLab | PAT, or the new fine-grained one | `PRIVATE-TOKEN` | 365 days, mandatory |
| Linear | Personal API key | `Authorization` with **no** Bearer | None |
| Asana | Personal access token | `Authorization: Bearer` | None |
| Wrike | Permanent access token | `Authorization: bearer` | None |
| Azure DevOps | Org-scoped PAT | `Basic base64(":" + PAT)` | Configurable |
| Trello | API key plus user token | Query string | `never` available today |
| ClickUp | Personal token | `Authorization` with **no** Bearer | None |
| Monday.com | Personal API token | `Authorization` with **no** Bearer | Not documented |
| Shortcut | API token | `Shortcut-Token` | Not documented |

Four of these send a bare token with no `Bearer` prefix. That is the single most
likely implementation bug across the adapters, and it is worth a test each.

Two expire by force, so a token-expiry story is needed from the first release
rather than added after somebody's integration quietly stops working: catch the
401, say the token expired, and link to the page that issues a new one.

**Height is gone.** It shut down in September 2025 and its API no longer answers.
It was on the original list and comes off it.

### What the count of calls really is

A ticket with a screenshot and three custom fields:

| Calls | Trackers |
| --- | --- |
| 2 | Monday, ClickUp, Shortcut, and most of the rest |
| 3 | Linear, whose upload is a signed URL in two steps |
| 6 | Trello, which needs one PUT per custom field value |

The unpriced cost is not the create call. It is **setup-time discovery**: Jira
needs project ids, issue type ids and `customfield_NNNNN` ids, per site. Linear
needs a team id. Monday needs board-specific column ids. ClickUp needs field
UUIDs. Trello needs a list id. Every adapter therefore needs a configuration
screen that queries the tracker and lets you map its fields to a note's, and
that is probably larger than the create call it exists to serve.

### Two-way sync is not available to a desktop app. At all.

Every one of the eleven requires a publicly reachable HTTPS endpoint for
webhooks, and several block loopback explicitly. Linear requires a "non-
localhost URL". Azure DevOps says webhooks cannot target loopback or special
ranges. GitLab blocks private ranges as SSRF protection and on GitLab.com that
is not user-changeable. Shortcut's own testing documentation recommends a tunnel
service, which is the vendor saying directly that a local app cannot receive
these.

So polling is the only mechanism, and its quality varies more than anything else
here. GitHub is the best by a distance: `?since=` plus conditional requests that
return 304 without counting against the rate limit, so a quiet loop is free.
ClickUp, GitLab and Wrike all have a clean "changed since" filter. Monday is the
worst, interpreting relative date tokens against the calling user's own timezone
and locale rather than offering a timestamp.

### Prior art, and the gap is real

**No native desktop app does point-and-click element inspection and ticket
filing.** That was checked from several angles and held. The reason is
structural: reaching `document` needs a browser extension or an injected script,
and a desktop app is working with screen pixels. Polypane and Sizzy are native
multi-viewport browsers with element inspection and **no bug reporting at all**.
BugHerd, Marker.io, Userback and the rest are browser extensions or injected
snippets. The two halves exist separately and nobody has joined them.

Three findings worth carrying:

- **Only three tools verifiably capture the CSS selector**: Atarim, OverlayQA and
  Disbug. Element-precise capture is a thinner field than the marketing suggests,
  and it is the thing Break/Points already does.
- **Two-way sync is rare and oversold.** Marker.io markets it broadly and its own
  documentation says GitHub comments do not sync back and Linear is one-way.
- **The competitive threat is Vercel and Netlify, not the bug trackers.** Vercel
  Comments is on by default on every preview deployment on every plan, free, and
  files to Linear, Jira or GitHub. What that does not cover is production, and
  hosting with no equivalent, which is Pantheon and most Drupal and WordPress
  work.

**MCP arrived in this category during 2026.** Marker.io, Jam, Userback,
Crosscheck and Polypane all ship an MCP server, and Disbug dropped its tracker
integrations entirely to feed coding agents instead. Break/Points already has
one, which is worth knowing: the thing this app would be adding, several of the
incumbents are walking away from.

### Three things to settle before building

1. **Wrike's attachment upload is undocumented.** The current reference no longer
   says how to send file bytes at all, and the recipe in circulation is
   second-hand. One live call settles it, and it should be made before any of
   this hardens into a specification.
2. **GitHub has no supported screenshot upload API.** Everything else here is
   documented ground; that is not. Decide between the undocumented endpoint,
   a release asset, or shipping GitHub support without pictures.
3. **Linear's documentation says the upload PUT "must be executed on the
   server"** because of a content security policy. That almost certainly means
   browser JavaScript rather than a native app, but if it applies here the
   no-server premise breaks for Linear specifically.

### Recommendation

Two trackers, properly, before ten. **Jira and GitHub** cover the most
professional developers by a wide margin, they sit at opposite ends of the
difficulty range, and building both proves the plumbing generalises. Linear
third, because its `attachmentCreate` takes a metadata object and uses the URL as
an idempotency key, which means a note maps onto it more cleanly than anywhere
else.

### What this changes about the app, and it is not small

Break/Points is local. `features.md` says so in as many words: everything runs
on your machine, and the bridge listens on loopback only. That is currently
true, and it is a real part of what the app is.

Sending a note to Asana breaks that promise. Not accidentally, and not in a way
that should be hidden in a settings panel. The moment this ships, the app makes
outbound connections carrying a client's page content, a screenshot of their
unreleased design, and whatever was typed in the note.

That has three consequences, and all three are requirements rather than
nice-to-haves.

1. **Off by default, per project.** No tracker is configured until somebody
   configures one. The row's behaviour with no tracker set up is exactly what it
   is today.
2. **The privacy sentence has to change** in `features.md` and the README, to
   say that nothing leaves the machine unless a tracker is connected. A promise
   that is true except in one case is a promise that is false.
3. **Credentials do not go in `breakpoints.json`.** The bridge token lives there
   today and that is fine, because it guards a loopback port and is worthless to
   anybody who is not already on the machine. A tracker token is the opposite:
   it grants write access to a client's project management system, and a plain
   JSON file in the application support folder is the wrong place for it. It
   belongs in the macOS Keychain, which means a new dependency.

### What it would cost

The HTTP client is already in the build. `reqwest` with rustls is a dependency
today, used by the dev server detection, so calling an external API adds no new
crate for the networking itself.

The per-project settings have a home already: `ProjectRecord` in `model.rs`
holds the dev URL, the screenshot folder and the framework per project, and a
tracker mapping belongs beside them. The tracker, the project or board id, the
issue type and any field mapping are per project, because one person works on
several clients and they will not share a tracker.

The work splits into three parts and they are very different sizes.

- **The plumbing**: settings, credential storage, a "send to tracker" action on
  a note, the per-project mapping, and the error handling when a call fails.
  This is the same whichever tracker is chosen, and it is most of the work.
- **One adapter per tracker**: the actual call that creates the ticket. Small
  for each, but ten of them is not small, and every one is a thing that breaks
  when somebody else changes their API.
- **Attachments**: nearly always a second call after the ticket exists, with a
  different content type and often a different host. This is where each adapter
  stops being ten lines.

The honest read is that supporting ten trackers properly is a bigger job than
anything currently in the app, and that supporting two well is worth more than
supporting ten badly.
