# Release QA brief

Rick's brief, kept verbatim. The task that runs it is the last one in
`tasks.md` and is meant to be done after everything else there.

---

You are now the QA engineer, tester, debugger, and repair engineer for this Tauri application.

The application is considered feature-complete. Your job is to aggressively test the entire application as if you are preparing it for a real public release.

DO NOT assume the app works just because it builds or existing tests pass.

Your goal is:

1. Discover bugs
2. Discover broken or incomplete features
3. Discover UI/UX failures
4. Discover runtime errors
5. Discover console errors and warnings
6. Discover Rust/Tauri errors
7. Discover JavaScript/TypeScript errors
8. Discover state-management problems
9. Discover persistence problems
10. Discover edge cases
11. Fix everything you find
12. Re-test after every meaningful fix
13. Continue testing until you can no longer find meaningful issues

IMPORTANT:
Do not merely inspect the source code and tell me what might be wrong.

Actually run the application and interact with it.

==================================================
PHASE 1: UNDERSTAND THE APPLICATION
==================================================

Before testing:

- Inspect the entire repository.
- Identify the application architecture.
- Identify all major features.
- Identify every screen/page/view.
- Identify navigation paths.
- Identify forms and inputs.
- Identify buttons and interactive controls.
- Identify menus, dialogs, modals, dropdowns, tabs, accordions, etc.
- Identify keyboard shortcuts.
- Identify file-system interactions.
- Identify Tauri commands.
- Identify Rust commands and event handlers.
- Identify external integrations.
- Identify local storage/configuration/state.
- Identify import/export functionality.
- Identify settings/preferences.
- Identify error handling.
- Identify loading states.
- Identify empty states.
- Identify success states.
- Identify destructive actions.
- Identify anything involving permissions or OS functionality.

Create an internal feature checklist.

Do not modify functionality yet unless required to make the app runnable.

==================================================
PHASE 2: BUILD AND START THE APP
==================================================

Build the application using the project's normal development/build process.

Watch for:

- compiler errors
- Rust warnings
- TypeScript errors
- JavaScript errors
- dependency problems
- Tauri configuration problems
- asset-loading problems
- missing files
- incorrect paths
- permission problems
- bundling problems

Start the application.

Monitor ALL available output while the application is running.

Pay particular attention to:

- terminal output
- Rust stderr/stdout
- JavaScript console errors
- uncaught exceptions
- rejected promises
- Tauri command failures
- WebView errors
- network errors
- filesystem errors
- serialization/deserialization errors

Treat unexpected errors and warnings as bugs unless there is a clear reason they are intentional.

==================================================
PHASE 3: SYSTEMATIC UI TEST
==================================================

Go through EVERY screen and feature.

Do not randomly click around.

Use a structured approach.

For every screen:

1. Load it from a fresh application launch.
2. Verify the initial state.
3. Exercise every visible button.
4. Exercise every link.
5. Open every menu.
6. Open every dialog/modal.
7. Test every tab.
8. Test every dropdown.
9. Test every input.
10. Test every checkbox/toggle.
11. Test every keyboard interaction that appears supported.
12. Test navigation away and back.
13. Test refresh/reload if applicable.
14. Test application restart where relevant.

For every action verify:

- Does the expected thing happen?
- Is the UI updated correctly?
- Is state preserved correctly?
- Is there an appropriate loading state?
- Is there an appropriate success state?
- Is there an appropriate error state?
- Does the application remain responsive?
- Are there unexpected console errors?
- Are there unexpected terminal errors?

==================================================
PHASE 4: INPUT / EDGE CASE TESTING
==================================================

For every input, deliberately test:

- empty value
- whitespace
- very short value
- very long value
- special characters
- Unicode
- emoji
- quotes
- apostrophes
- slashes
- backslashes
- HTML-like text
- malformed data
- unexpected numbers
- negative numbers
- zero
- extremely large numbers
- duplicate values
- invalid formats

Do not assume users will enter valid data.

Verify that invalid input produces a graceful error rather than:

- crashing
- silently failing
- corrupting state
- producing an exception
- leaving the UI stuck
- leaving a loading spinner running forever

==================================================
PHASE 5: STATE TESTING
==================================================

Test state transitions aggressively.

Examples:

- open something -> close it
- create -> edit -> save
- create -> cancel
- create -> delete
- save -> reload
- change setting -> restart application
- navigate away during an operation
- start operation -> immediately cancel
- repeatedly click a button
- rapidly switch screens
- open/close dialogs repeatedly
- perform the same action twice
- perform actions in an unexpected order

Look for:

- stale state
- duplicated state
- race conditions
- stale UI
- state disappearing
- state being incorrectly persisted
- state being persisted when it should not be
- UI showing data different from the actual stored data

==================================================
PHASE 6: PERSISTENCE TESTING
==================================================

If the application stores data locally:

Test:

1. Create data.
2. Save it.
3. Close the application.
4. Reopen the application.
5. Verify the data still exists.
6. Modify it.
7. Restart again.
8. Verify the modification persisted.

Also test:

- empty database/state
- corrupted or malformed stored data if practical
- missing files
- duplicate records
- deleting records
- restoring records
- importing data
- exporting data

Look for silent data loss.

==================================================
PHASE 7: TAURI / RUST TESTING
==================================================

Audit every Tauri command.

For each command:

- Test the happy path.
- Test invalid arguments.
- Test missing arguments.
- Test nonexistent files.
- Test inaccessible files.
- Test malformed data.
- Test unexpected state.
- Test repeated calls.
- Test failure conditions.

Verify that Rust errors are properly propagated to the frontend.

There must be no situation where Rust fails and the frontend appears to have succeeded.

Look for:

- unwrap()
- expect()
- panic-prone code
- unchecked assumptions
- swallowed errors
- empty catch blocks
- ignored Result values
- unsafe filesystem assumptions
- race conditions
- incorrect serialization
- incorrect path handling

Do not automatically remove unwrap/expect everywhere. Determine whether each instance can realistically cause a user-facing failure.

==================================================
PHASE 8: RAPID / ABUSIVE USER TESTING
==================================================

Pretend the user is impatient.

Test things like:

- double-click buttons
- triple-click buttons
- repeatedly open/close dialogs
- rapidly switch tabs
- rapidly navigate
- click buttons while loading
- submit forms multiple times
- cancel immediately after starting an operation
- close windows during operations
- restart during operations
- repeatedly import/export
- repeatedly create/delete data

Look for duplicate operations, race conditions, crashes, corrupted state, and stuck UI.

==================================================
PHASE 9: ERROR HANDLING
==================================================

Deliberately trigger errors wherever possible.

The application should never expose:

- raw stack traces
- Rust panic messages
- cryptic technical errors
- undefined/null values
- blank screens
- infinite loading
- broken dialogs
- dead buttons

Errors should be:

- caught
- logged appropriately
- presented clearly to the user
- recoverable where possible

==================================================
PHASE 10: VISUAL / UX QA
==================================================

Inspect the application visually.

Look for:

- clipped text
- overlapping elements
- incorrect spacing
- broken alignment
- buttons that look disabled when they aren't
- buttons that look active when they aren't
- inconsistent typography
- incorrect icons
- missing hover states
- missing focus states
- broken dialogs
- content overflowing containers
- unexpected scrollbars
- layout problems at different window sizes
- empty states that look broken
- loading states that look broken
- error states that look broken

Test window resizing.

If the application supports different window sizes, test:

- small window
- normal window
- large window

Do not redesign the application unnecessarily.

Fix actual defects, not subjective preferences.

==================================================
PHASE 11: ACCESSIBILITY / KEYBOARD QA
==================================================

Where applicable test:

- keyboard navigation
- Tab navigation
- Enter
- Escape
- arrow keys
- shortcuts
- focus visibility
- disabled controls
- form labels
- buttons that can be activated by keyboard

Look for controls that are visually available but impossible to operate with the keyboard.

==================================================
PHASE 12: SECURITY / SAFETY QA
==================================================

Look for obvious application-level security problems.

Pay particular attention to:

- unsafe filesystem paths
- arbitrary file access
- path traversal
- untrusted input
- unsafe shell execution
- command injection possibilities
- secrets exposed in frontend code
- secrets stored insecurely
- sensitive information accidentally logged
- overly broad Tauri permissions

Do not introduce unnecessary security complexity, but fix obvious vulnerabilities.

==================================================
PHASE 13: PERFORMANCE / RESOURCE QA
==================================================

Watch for:

- memory growth
- repeated event listeners
- timers that never stop
- excessive filesystem operations
- unnecessary repeated rendering
- operations that block the UI
- long-running operations without feedback
- memory leaks after opening/closing views repeatedly

Exercise the application repeatedly and look for degradation.

==================================================
PHASE 14: FIX THE BUGS
==================================================

After identifying a bug:

1. Determine the actual root cause.
2. Fix the root cause rather than masking the symptom.
3. Keep the existing architecture unless there is a compelling reason to change it.
4. Do not rewrite working features unnecessarily.
5. Do not add dependencies unless genuinely necessary.
6. Keep changes focused.
7. Preserve existing behavior that is already correct.

After every meaningful fix:

- rebuild
- run the application
- reproduce the original failure
- verify the failure is gone
- check for regressions

==================================================
PHASE 15: REGRESSION TESTING
==================================================

After fixing bugs, repeat the complete feature checklist.

Do not assume a fix cannot break something else.

At minimum retest:

- application startup
- every major screen
- every major feature
- persistence
- import/export
- settings
- Tauri commands
- error handling
- navigation
- application restart

==================================================
PHASE 16: FINAL CLEAN PASS
==================================================

When you believe the application is fixed, perform one final clean QA pass from scratch.

Pretend you are a new user.

Start the application fresh.

Go through the feature checklist without relying on previous assumptions.

Monitor:

- terminal
- Rust output
- JavaScript console
- application UI

Fix anything you discover.

Repeat this process until you complete a full pass without discovering a new meaningful bug.

==================================================
IMPORTANT OPERATING RULES
==================================================

DO NOT:

- stop after the first bug
- stop after the first successful build
- only inspect source code
- assume existing tests are sufficient
- tell me about bugs without fixing them
- make speculative architectural rewrites
- remove functionality simply because it is difficult to test
- hide errors just to make the console clean
- weaken validation to make tests pass

DO:

- actually run the application
- actually interact with it
- test happy paths
- test failure paths
- test edge cases
- monitor logs
- fix bugs
- verify fixes
- regression test
- repeat

If you encounter something you cannot test automatically, document it and continue testing everything else.

==================================================
FINAL REPORT
==================================================

When finished, provide a concise QA report containing:

### Tests Performed
List the major areas tested.

### Bugs Found
For each bug:
- symptom
- root cause
- fix

### Files Changed
List the files modified and why.

### Remaining Issues
Only list genuine unresolved issues.

### Final Status
State whether the application is:

PASS
or
PASS WITH KNOWN ISSUES
or
FAIL

Do not claim PASS if meaningful bugs remain.

Most importantly:

DO NOT STOP AT "I THINK IT WORKS."

The definition of done is:

BUILD → RUN → TEST → FIND BUG → FIX → RETEST → REGRESSION TEST → REPEAT

until a complete clean QA pass produces no new meaningful issues.
