# M2b live verification — 2026-10-19

M2b is the desktop half of M2: the operate screens, inline confirmation, jobs with reports, and the shared activity log.

- **The emulator column** comes from the desktop gate `emulator_m2b`. It runs the app's own operate path (the functions behind the Tauri commands, the GUI policy and the activity log) against the local emulator, on a profile marked production.
- **The screens** are covered by Vitest against the mock engine. There, every sheet is driven through confirmation, running and the report.
- **No cloud namespace was used.** Neither was the native window: the GUI walkthrough below is unchecked.

| Check | Emulator (SAS, production profile) | Cloud |
|---|---|---|
| Send 5 with an auto-increment MessageId and a `long` property of 9007199254740993 (above 2^53), read back exactly | ✅ gate step 1 | not run |
| Receive 2 and complete; the received messages stream to the webview (`received` events) | ✅ gate step 2 | not run |
| Delete by sequence number, plus one that is not there: 1 succeeded, 1 missing, each item reaches the webview once | ✅ gate step 3 | not run |
| Move to another queue: typed confirmations in order, source (step 1) then target (step 2) | ✅ gate step 4 | not run |
| Dead-letter one, then resubmit it from the dead-letter queue with an edited Subject, removing the original | ✅ gate step 5 | not run |
| Purge two entities: counted first, one typed confirmation per entity, both empty after; a plan runs once | ✅ gate steps 0 and 6 | not run |
| Session state: set, read back, clear | ✅ gate step 7 | not run |
| Dead-letter summary of an empty dead-letter queue | ✅ gate step 8 | not run |
| Activity: started and finished `gui` entries for all 13 gate runs (a purge gates once per entity), every one succeeded, every entry emitted to the webview | ✅ gate step 9 | not run |
| A next-available session accept releases the connection between 1 s slices | unit tests (`accept_in_slices_*`) | not run |
| Lock-and-match items reach the progress sink as they settle | unit test (`items_reach_progress_as_they_settle`) and gate step 3 | not run |

**Gate run (local):**
- Command: `DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-app --lib emulator_m2b -- --nocapture`
- Result: `emulator_m2b: all steps passed`, 1 passed, 11.5 s.
- The emulator was restarted first, to load the new `m2b-*` queues.
- CI runs this gate in the emulator job, after the four core gates.

**Local checks:**
- Rust: 906 workspace tests, plus clippy and fmt clean.
- Frontend: Vitest 72 files and 848 tests; `tsc` and ESLint clean.

## Decision

M2b desktop operate: **GO for the emulator.** Every operation runs through the app's own path with typed production confirmations and its activity entries. The GUI walkthrough and the cloud checks below are not run.

## Desktop walkthrough (not run; run on the emulator before the v0.x release)

Run `npm run tauri dev` in `app/`, with a connection-string profile for the emulator marked production:

- [ ] Every sheet opens from the toolbar, the tree menu and the palette. These are Send, Clone & resend, Import, Receive, Delete, Move, Resubmit, Cancel scheduled, Settle, Purge and Session state.
- [ ] Each confirmation shows the core's sentence and `PROD`, and needs the entity name typed. A move asks twice, in the same sheet.
- [ ] Closing a sheet while it asks cancels the job, and the Activity tab shows the change as denied.
- [ ] The Delete key on selected rows opens the Delete sheet and deletes nothing.
- [ ] A CLI command run in a terminal (for example `dispatched --profile emulator purge m2b-a --yes --confirm-prod m2b-a`) appears in the Activity tab within 3 s, tagged CLI, and the open view refreshes.
- [ ] Disconnecting during a long purge ends the job as stopped, keeping its report.
- [ ] Save report and Save current state write only to the file picked in the save dialog.

## Cloud checklist

The M2a cloud checklist (`docs/live-verification-m2a.md`) carries over unchanged. In addition:

- [ ] A partitioned entity: sequence numbers above 2^53 select, delete and report exactly in the desktop. The M1c grid holds sequence numbers as numbers (§15 Decision 7), so this is expected to fail until the grid moves to strings.
- [ ] Production confirmations on a real production profile.

## Follow-ups

- The grid's sequence numbers become strings, for partitioned entities (§15 Decision 7).
- Template collection files and single-subscription GET move to M3 (§15 Decision 13).
