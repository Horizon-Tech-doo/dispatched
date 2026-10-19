# M2b — Desktop Operate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (the user chose native execution: larger batches, emulator gates, one whole-branch review at the end). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring every M2a operation into the desktop app: send and compose, receive and settle, delete, move, resubmit, purge, scheduled cancel, deferred settle, import, the dead-letter summary and session state. Each runs through the core gate, with confirmations in the GUI, job reports in the Jobs drawer and a persisted, shared Activity log.

**Architecture:**
- The desktop builds a core `Gate` and `Operator` per operation, exactly as the CLI does.
- Its `Policy` is `GuiPolicy`. It emits a `confirm` event carrying the core's own summary text and waits on a oneshot that the `confirm_answer` command fills. Confirmation text therefore always comes from `Operation::summary`, never from the frontend.
- The gate's `Audit` is the shared `activity.db`, wrapped so each entry is also emitted to the webview.
- Core `JobEvent`s are mapped onto a generalised desktop `JobEvent`, and the finished `JobReport` is kept per job for the report view and for saving.
- The frontend adds one `OperationSheet` frame (form → confirm → running → report) that every operation sheet uses.

**Tech Stack:**
- Rust: dispatched-core, Tauri 2, tokio, horizon-azure-kit `safety`.
- Frontend: React 18 + TS, TanStack Query, Vitest + jsdom, the existing `ui/` design system (`Sheet`, `Button`, grid, tree).

**Spec:** `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`. The relevant sections are §4.4–4.9, §5, §6, §11 (M2 row), §13 and §14 (M2a decisions, especially 3, 7, 22, 26, 33, 36, 39 and 42). The backlog is in `docs/live-verification-m2a.md` → "Follow-ups".

## Global Constraints

- Disk hygiene, for every build and in every subagent prompt:
  - never create extra target dirs;
  - never copy the repo or a crate to build or test it;
  - never set `CARGO_TARGET_DIR` except to `~/.cache/cargo-shared-target`;
  - run `df -h /System/Volumes/Data` before heavy builds, and stop if under 30 GB.
- No real Azure. All live checks run on the local emulator (`testing/emulator/emulator.sh`).
- No modal dialogs.
  - Confirmations are inline in the sheet that started the operation (DESIGN.md: right-edge `Sheet`, 420px, one primary button, no dimming).
  - `dismissible=false` while a confirmation or a run is in progress.
- Danger is carried by the label ("Delete 3 messages"), never by colour. Amber follows the Earned Amber Rule: dead-letter counts above 0, PROD, errors, invalid fields. `ProdBadge` appears in every production confirmation.
- Production confirmation (§5): type the entity name, once per entity the operation changes, including a move or resubmit target (Decision 3). The typed text must equal `ConfirmRequest.target`, because the kit's `require` checks it.
- Receive-and-delete is never remembered as a default. Purge is never one click (§5). The Delete key opens a confirmation and never deletes directly (§6).
- Confirmation text is the core's `ConfirmRequest.summary`, verbatim. The GUI adds no count of its own.
- Every activity entry from the desktop has `origin = gui`. No secret is put in any event, activity entry or report.
- Int64 values (sequence numbers, `long`/`ulong` properties) cross IPC as strings wherever they can exceed 2^53. This is parked Minor 13.
- Every UI task follows the impeccable and design-taste-frontend rules in `app/DESIGN.md`. The impeccable review pass runs once over all new screens in Task 10, which is the lighter process.

## Review Focus

1. **A confirmation that is never answered.** The sheet is closed, the job cancelled, the connection switched or the app quit. The job must end `cancelled` with nothing changed and no pending entry. A stuck oneshot must not hold the explorer. Test in Task 2: `unanswered_confirm_cancels_on_job_cancel` and `on_close`.
2. **Two confirmations in one operation**, as a production move (source, then target) or a multi-entity purge. Each must be shown in turn in the same sheet, and denying the second must change nothing. Test in Task 2: `production_move_asks_source_then_target`; Task 5: `confirm panel shows the second request after the first`.
3. **A CLI run changing an entity that is open in the app.** The view must refresh within the poll interval, and the Activity log must show the CLI entry. Test in Task 6: `cli entries invalidate the affected entity`.
4. **A long property or sequence number above 2^53** in resubmit edits or the report. It must round-trip unchanged. Test in Task 3: `long_property_round_trips_as_string`; Task 7: `edit sheet keeps 9007199254740993`.
5. **The connection drops or is switched while a job runs.** The job ends `stopped`, its report keeps the items so far, and the drawer says so. Test in Task 3: `close_running_keeps_partial_report`.

---

### Task 1: Core carry-overs (Decision 42)

**Files:**
- Modify: `crates/dispatched-core/src/explorer.rs` (`open_receiver_once`, about l.400)
- Modify: `crates/dispatched-core/src/ops/locate.rs` (emit found items as they settle)
- Test: the same files' `#[cfg(test)]` modules (fake broker in `messaging/fake.rs`)

**Interfaces:**
- Produces:
  - Unchanged signatures.
  - `JobEvent::Item` is now emitted per item during delete, move and resubmit, as each item settles, not only at the end.

- [ ] **Step 1:** Write the failing tests.
  - `explorer::tests::peek_proceeds_while_next_available_waits`: the fake broker delays next-available accepts by 3 s; a peek started 100 ms later finishes in under 1 s.
  - `locate::tests::items_reach_progress_as_they_settle`: delete of 3 sequences with a `RecordingProgress`; the first `Item` event arrives before the last message is located (assert event order: `Item(seq a)` precedes the fake's `delivered(seq c)` marker).
- [ ] **Step 2:** Run `cargo test -p dispatched-core --features test-support peek_proceeds_while items_reach_progress`. Expected: FAIL.
- [ ] **Step 3:** In `open_receiver_once`, the `NextAvailable` path waits in slices of at most 1 s. Each slice takes the connection guard, calls `try_accept_next(spec, slice)` and drops the guard. The loop ends on a session, on `wait` elapsed (`Ok(None)`), or on an error. In `locate.rs`, call `progress.event(&JobEvent::Item(..))` where each item's outcome is final, and remove the end-of-run replay; no item may be emitted twice.
- [ ] **Step 4:** Run the tests above, then the full `cargo test -p dispatched-core --features test-support`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: next-available accepts release the connection between slices; lock-and-match emits items live`.

### Task 2: Desktop gate — GuiPolicy, persisted audit, operator factory

**Files:**
- Create: `app/src-tauri/src/safety.rs`
- Modify:
  - `app/src-tauri/src/state.rs` (`Active` gains `production: bool`, `profile_name: String`; `AppState` gains `activity: Arc<ActivityLog>`, `confirms: Arc<Confirms>`)
  - `app/src-tauri/src/events.rs` (`Events::confirm(ConfirmEvent)`, event name `CONFIRM = "confirm"`; `Events::activity_entry(ActivityRecord)`)
  - `app/src-tauri/src/commands.rs`, `lib.rs`
- Test: `app/src-tauri/src/safety.rs` tests

**Interfaces:**
- Consumes:
  - `dispatched_core::safety::{Gate, Actor, ProfileRef, Audit}`, `dispatched_core::activity::{ActivityLog, ActivityEntry}`, `dispatched_core::paths::open_activity_log`;
  - kit `Policy`, `ConfirmRequest`, `Decision`, `Origin::Gui`.
- Produces:
  - `pub struct Confirms`, with:
    - `fn ask(&self, job_id: &str, req: &ConfirmRequest) -> (String /*confirm id*/, oneshot::Receiver<Decision>)`
    - `fn answer(&self, confirm_id: &str, decision: Decision) -> bool`
    - `fn cancel_job(&self, job_id: &str)`, which resolves that job's pending asks as `Deny("cancelled")`
    - `fn cancel_all(&self)`
  - `pub struct GuiPolicy { job_id: String, confirms: Arc<Confirms>, events: Arc<dyn Events> }`, which implements `Policy`.
    - It emits `ConfirmEvent { id, job_id, verb, target, summary, item_count: Option<String>, production, step: u32, steps: Option<u32> }`.
    - It returns the answer, or `Deny("cancelled")` when the receiver is dropped.
  - `pub struct EmittingAudit { log: Arc<ActivityLog>, events: Arc<dyn Events> }`, which implements core `Audit`. It records, then emits `ActivityRecord` (the core entry, serialised with int64 as strings).
  - `pub fn operator_for(state: &AppState, profile_id: &str, job_id: &str, events: Arc<dyn Events>) -> Result<Operator, CommandError>`. It builds `Gate::new(GuiPolicy, EmittingAudit, Actor { origin: Gui, profile: ProfileRef { name, production } })`.
  - Command `confirm_answer(id: String, decision: ConfirmAnswer) -> bool`.
    - `ConfirmAnswer` is serde-tagged: `{ "type": "allow" } | { "type": "typed", "text": String } | { "type": "deny" }`.
    - `Allow` is mapped to `ConfirmedTyped(target)` only when the request was not production. On production, only `typed` is accepted, and the kit then checks the text.
  - `AppState::close` also calls `confirms.cancel_all()`. `Jobs::cancel(id)` also calls `confirms.cancel_job(id)`.
- The activity log is opened at startup with `paths::open_activity_log()`. The open failure is shown like `startup_error` and does not crash; mutating commands then fail with kind `activity_unavailable`, because §5 requires an audit entry before any mutation.

- [ ] **Step 1:** Write the failing tests:
  - `allow_on_non_production_proceeds`
  - `typed_wrong_name_is_denied_on_production`, using the kit's `require` through `Gate::run` with a fake op; it is logged `denied`
  - `production_move_asks_source_then_target`: two `ConfirmEvent`s with `step` 1 and 2; denying step 2 leaves the fake broker unchanged
  - `unanswered_confirm_cancels_on_job_cancel`
  - `unanswered_confirm_cancels_on_close`: the job ends cancelled, and the activity has `denied` or `cancelled` and no `pending`
  - `audit_entry_is_emitted_and_persisted`: `ActivityLog::open_in_memory()` plus a `Recorder`
- [ ] **Step 2:** Run `cargo test -p dispatched-app safety`. Expected: FAIL.
- [ ] **Step 3:** Implement the types above. `step` and `steps` come from `Operation::confirm_requests(&actor).len()`: the policy is called in order, so count the calls per job.
- [ ] **Step 4:** Run `cargo test -p dispatched-app`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: GUI policy with inline confirmation, persisted activity, operator factory`.

### Task 3: Operate commands and generalised jobs

**Files:**
- Create: `app/src-tauri/src/ops/operate.rs` (runner plus one fn per operation); `app/src-tauri/src/ops/operate_types.rs` (IPC request and view types)
- Modify:
  - `app/src-tauri/src/events.rs`: desktop `JobEvent` gains `kind: String` on `Started`, and new variants `Item { id, item: ItemView }`, `Note { id, text }`, `Received { id, message: MessageSummaryView }`. `Finished` gains `report: Option<ReportSummary>`. The export fields stay.
  - `app/src-tauri/src/jobs.rs`: `Entry` keeps `report: Option<JobReport>`.
  - `commands.rs`, `ops/mod.rs`
- Test: `app/src-tauri/src/ops/operate.rs` tests with the core fake broker (`dispatched-core` `test-support` feature as a dev-dependency) and `Recorder`

**Interfaces:**
- Consumes: Task 2 `operator_for`, `Confirms`; core `Operator` methods `send`, `import`, `cancel_scheduled`, `receive`, `settle_deferred`, `delete`, `move_messages`, `resubmit`, `plan_purge`, `purge`, `set_session_state`; `Explorer::{dlq_summary, session_state, resolve_kind}`; `dispatched_core::read_messages`.
- Produces these commands. Each `op_*` returns `JobStarted { id }` at once; confirm, progress and finish arrive as events.
  - `op_send(profile_id, req: SendForm)`. `SendForm` has:
    - `target`, `template: OutgoingView`, `body: BodyForm`, `ids: MessageIdForm`, `count: u64`, `schedule: Option<String /*RFC3339*/>`, `batch: bool`;
    - `body` is `BodyForm::Text(String) | Files(Vec<String /*pick ids*/>) | Template(String /*pick id*/)`.
  - `op_import(profile_id, target, pick_id, opts: ImportOptions)`
  - `op_cancel_scheduled(profile_id, entity, sequences: Vec<String>)`
  - `op_receive(profile_id, req: ReceiveForm)`. Received messages stream as `Received` events, and the full messages are kept in the report (Decision 17).
  - `op_settle_deferred(profile_id, entity, sub_queue, sequences, session, how: SettlementForm)`
  - `op_delete(profile_id, spec: SelectForm)`, `op_move(profile_id, spec, target, drop_uncopyable: bool)`, `op_resubmit(profile_id, spec: ResubmitForm)`
    - `SelectForm` is `{ entity, view, selection: Sequences(Vec<String>) | Matching { filter: FilterSpec, limit: u64 } }` with the core default `LocateLimits`.
  - `purge_plan(profile_id, entities: Vec<EntityRef>, scope) -> Vec<PurgePlanView>`. It has no gate. It counts every entity first; a failed count returns an error with nothing changed (Decision 26). Plans are stored in `Jobs` under a plan id.
  - `op_purge(profile_id, plan_id)`
  - `dlq_summary(profile_id, entity, sub_queue) -> DlqSummaryView`
  - `session_state_get(profile_id, entity, session) -> Option<SessionStateView { base64, text: Option<String>, len }>`
  - `op_session_state_set(profile_id, entity, session, state: Option<String /*base64*/>, allow_large: bool)`
  - `resolve_kind(profile_id, entity) -> Option<EntityKind>`
  - `pick_open_paths(window, purpose: "body" | "template" | "import", multiple: bool) -> Vec<PickedPath>`. Rust opens the native dialog and the webview gets pick ids only (R8 pattern).
  - `job_report(id) -> Option<ReportView>`. It is paged: `offset`, `limit`, and items with int64 as strings.
  - `pick_report_path(window, id, format)` and `job_report_save(id, pick_id)`, which write through `JobReport::write`.
- The runner is `fn spawn_op(state, events, kind: JobKind, title: String, work: impl FnOnce(Operator, ProgressBridge, CancellationToken) -> Fut)`. It:
  - emits `Started`;
  - maps core `JobEvent` → desktop events through `ProgressBridge`, which implements core `Progress`;
  - stores the report, then emits `Finished` with `ReportSummary { outcome, succeeded, failed, skipped, missing, not_reached, notes, summary_line }`;
  - maps a connection close to `stopped` and keeps the partial report.
- `AtLeast(0)` deferred estimates are rendered by the view type as `"an unknown number of deferred messages"` (Decision 42).

- [ ] **Step 1:** Write the failing tests (fake broker, auto-answer policy through `Confirms`):
  - `send_three_reports_three_sequences`
  - `receive_delete_streams_received_events`
  - `delete_by_sequence_reports_missing`
  - `move_production_needs_two_typed_answers`
  - `purge_plan_failed_count_changes_nothing`
  - `purge_runs_stored_plan_once`: a second `op_purge` with the same id fails `not_found`
  - `session_state_set_then_get`
  - `long_property_round_trips_as_string`: a resubmit edit sets `long` `9007199254740993`, and the fake broker receives exactly that value
  - `close_running_keeps_partial_report`
  - `report_save_writes_json`
- [ ] **Step 2:** Run `cargo test -p dispatched-app operate`. Expected: FAIL.
- [ ] **Step 3:** Implement. Register every command in `lib.rs`.
- [ ] **Step 4:** Run `cargo test -p dispatched-app`, then `cargo clippy -p dispatched-app --all-targets -- -D warnings`. Expected: PASS and clean.
- [ ] **Step 5:** Commit: `app: operate commands, generalised job events and reports`.

### Task 4: Frontend engine — types, commands, events, mock

**Files:**
- Modify:
  - `app/src/engine/types.ts`: mirror Task 2 and 3 types; int64 fields are `string`.
  - `app/src/engine/commands.ts`: one wrapper per command.
  - `app/src/engine/events.ts`: `onConfirm`, extended `onJob`.
  - `app/src/engine/mock/transport.ts`, `mock/fixtures.ts`
- Modify: `app/src/shell/drawer/activity.ts`. `jobStore` holds the generalised `Job` (kind, items count, report summary, confirm state); the export retry stays.
- Test: `app/src/engine/mock/operate.test.ts`, `app/src/shell/drawer/activity.test.ts`

**Interfaces:**
- Produces:
  - TS functions `opSend`, `opImport`, `opCancelScheduled`, `opReceive`, `opSettleDeferred`, `opDelete`, `opMove`, `opResubmit`, `purgePlan`, `opPurge`, `dlqSummary`, `sessionStateGet`, `opSessionStateSet`, `resolveKind`, `pickOpenPaths`, `jobReport`, `pickReportPath`, `jobReportSave`, `confirmAnswer`, `activityList`, `activityExport`.
  - Mock scenarios: `operate-ok`, `operate-prod` (two confirmations), `operate-partial` (one failed item), `operate-stopped`.

- [ ] **Step 1:** Write the failing tests:
  - mock `opDelete` under `operate-prod` emits two `confirm` events and finishes only after both answers;
  - `jobStore` folds `started → item×3 → finished` into one job with `succeeded: 3`;
  - a `finished` event with `outcome: "partly_failed"` keeps the failed count.
- [ ] **Step 2:** Run `cd app && npx vitest run src/engine src/shell/drawer`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the same command, then `npx tsc --noEmit`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: engine bindings and mock for operate`.

### Task 5: OperationSheet, ConfirmPanel, report view, Jobs drawer

**Files:**
- Create:
  - `app/src/shell/operate/OperationSheet.tsx`, the frame. Stages are `form | confirming | running | done`. The footer has one primary button per stage: the verb label, then "Confirm"/"Delete in {entity}", then "Cancel job", then "Close".
  - `app/src/shell/operate/ConfirmPanel.tsx`: the core summary text, the item count, `ProdBadge`, and a typed-name field on production (the button is enabled only when the text equals `target`), plus "step n of m".
  - `app/src/shell/operate/ReportView.tsx`: the summary line, notes, and a virtualised item list (`ui/grid`) with outcome, sequence, reason and new sequence; filters All / Failed / Not done; "Save report…".
  - `app/src/shell/operate/useOperation.ts`, a hook that starts a job, tracks its events and answers confirms.
- Modify:
  - `app/src/shell/context.tsx`: `SheetState` gains `{ kind: "operation"; op: OperationKind; entity?: EntityRef; seed?: … }` and `{ kind: "report"; jobId }`.
  - `app/src/shell/sheets/SheetHost.tsx`
  - `app/src/shell/drawer/Drawer.tsx`: job rows for every kind, with the outcome caption from `ReportSummary` and the actions "View report" / "Cancel".
- Test: `app/src/shell/operate/operate.test.tsx`, `drawer/Drawer.test.tsx`

- [ ] **Step 1:** Write the failing tests:
  - `confirm panel shows the second request after the first`
  - `production confirm stays disabled until the typed name matches`
  - `closing the sheet while confirming answers deny and the job ends cancelled`
  - `the sheet is not dismissible while running`
  - `report view filters to failed items`
  - `drawer shows "2 of 3 deleted, 1 failed" and opens the report`
- [ ] **Step 2:** Run `npx vitest run src/shell/operate src/shell/drawer`. Expected: FAIL.
- [ ] **Step 3:** Implement to DESIGN.md: 420px sheet, the danger in the label, amber only on PROD and errors, and `Problem` for errors.
- [ ] **Step 4:** Run the same, then `npx tsc --noEmit && npx eslint src`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: operation sheet with inline confirmation, report view, jobs drawer`.

### Task 6: Activity log on activity.db

**Files:**
- Modify (Rust):
  - `app/src-tauri/src/ops/` adds `activity.rs`, with:
    - `activity_list(query: ActivityQueryForm { limit, origin, profile, outcome, after_id: Option<String> }) -> Vec<ActivityRecord>`
    - `pick_activity_export_path` and `activity_export(query, pick_id, format)`
    - `activity_since(after_id) -> Vec<ActivityRecord>`, which the poller uses
  - The in-memory read entries (`tracked`) stay as session-only rows. They are not persisted, because reads are not logged (Decision 2).
- Modify (TS): `app/src/shell/drawer/ActivityLog.tsx`.
  - It merges persisted records with the session rows by time.
  - Each row shows an origin tag (GUI/CLI/MCP), profile, entity, action, counts and outcome.
  - Filters: origin, outcome, current profile only. "Export…".
- Create: `app/src/shell/drawer/externalChanges.ts`.
  - It polls `activity_since` every 3 s while the window is focused.
  - For each finished record with origin ≠ gui on the active profile, it invalidates the TanStack queries of that entity (messages views, counts, sessions, dashboard).
- Test: Rust `ops::activity` tests; TS `ActivityLog.test.tsx`, `externalChanges.test.ts`

- [ ] **Step 1:** Write the failing tests:
  - `activity_list_filters_by_origin`
  - `activity_since_returns_only_newer`
  - TS: `merges persisted and session rows by time`
  - TS: `cli entries invalidate the affected entity`
  - TS: `gui entries do not trigger refetch`
- [ ] **Step 2:** Run the failing tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p dispatched-app` and `npx vitest run`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: activity log on the shared activity.db, CLI changes refresh views`.

### Task 7: Composer, clone & resend, import

**Files:**
- Create:
  - `app/src/shell/operate/ComposerSheet.tsx`:
    - body as text, file(s) round-robin or template;
    - the system properties of §4.7, with ContentType presets (`application/json`, `application/xml`, `text/plain`, `application/octet-stream`);
    - typed custom properties (`string, int, long, double, bool, datetime, timespan, guid`);
    - MessageId: fixed, auto-increment or random;
    - TTL, schedule, count (N copies), batch;
    - an inline validation per field, from the same rules as the CLI parser.
  - `app/src/shell/operate/PropertyEditor.tsx`
  - `app/src/shell/operate/ImportSheet.tsx`
- Modify: `shell/messages/MessagesView.tsx` and the detail pane. "Clone & resend" opens the composer seeded from the message: every user-settable property, typed properties kept, an `other` property shown as not copyable (Decision 33).
- Test: `ComposerSheet.test.tsx`, `PropertyEditor.test.tsx`

- [ ] **Step 1:** Write the failing tests:
  - `auto-increment previews order-0009 → order-0010`
  - `int out of range is an invalid field`
  - `clone seeds typed properties and marks "other" as not copyable`
  - `edit sheet keeps 9007199254740993` (long as a string end to end)
  - `schedule sends RFC3339 in UTC`
  - `files body needs at least one pick`
- [ ] **Step 2:** Run the failing tests. Expected: FAIL.
- [ ] **Step 3:** Implement on `OperationSheet`. "Send" in the Messages toolbar and the palette, and "Clone & resend" in the detail pane.
- [ ] **Step 4:** Run `npx vitest run` and `npx tsc --noEmit`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: composer, clone & resend, import`.

### Task 8: Message operations in the views

**Files:**
- Modify: `app/src/shell/messages/Toolbar.tsx` and `MessagesView.tsx`. The toolbar becomes: Peek more · Receive ▾ · Resubmit · Move · Delete · Export ▾ · Filter (§6). Per view:
  - Scheduled: "Cancel scheduled".
  - Deferred: Receive ▾ is "Settle ▾" (complete, abandon, dead-letter, delete).
  - Dead-letter and transfer DLQ: a "Summary" toggle showing the `dlq_summary` groups. Clicking a group sets the view filter to its reason.
- Create:
  - `ReceiveSheet.tsx`: count up to 5,000, wait, mode (peek-lock + settlement, or receive-and-delete), and session. Receive-and-delete defaults off every time.
  - `DeleteSheet.tsx`, `MoveSheet.tsx`: the target picker is the tree in pick mode, queues and topics only, with the source excluded.
  - `ResubmitSheet.tsx`: target, edits via `PropertyEditor`, new MessageId, remove original, keep dead-letter properties, drop uncopyable.
  - `CancelScheduledSheet.tsx`, `SettleDeferredSheet.tsx`
  - `shell/messages/DlqSummary.tsx`
- Selection source: the selected rows → `Sequences`, otherwise "all matching the current filter" → `Matching { limit }`. The sheet shows "matched n in the first m examined" from the core notes (Decision 22).
- The Delete key on a grid selection opens `DeleteSheet`. After a job finishes, the views of every affected entity are invalidated.
- Test: `Toolbar.test.tsx`, `ReceiveSheet.test.tsx`, `MoveSheet.test.tsx`, `DlqSummary.test.tsx`

- [ ] **Step 1:** Write the failing tests:
  - `delete key opens the delete sheet and deletes nothing`
  - `receive-and-delete is off when the sheet opens, even after a previous delete run`
  - `move target picker excludes the source and subscriptions`
  - `scheduled view shows Cancel scheduled, not Receive`
  - `dlq summary group click filters the view`
  - `finished job invalidates source and target views`
- [ ] **Step 2:** Run the failing tests. Expected: FAIL.
- [ ] **Step 3:** Implement on `OperationSheet`.
- [ ] **Step 4:** Run `npx vitest run` and `npx tsc --noEmit`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: receive, delete, move, resubmit, scheduled cancel, deferred settle, DLQ summary`.

### Task 9: Purge, session state, resolved kinds, palette

**Files:**
- Create: `app/src/shell/operate/PurgeSheet.tsx`.
  - Entities come from the tree context menu (one), the Dashboard (multi-select rows: add row checkboxes) or the palette.
  - It shows the scope (Active / Dead-letter / Transfer DLQ / All) and calls `purge_plan`.
  - It then shows a per-entity plan: per sub-queue counts, "about"/"at least", deferred, and "scheduled messages are not purged".
  - The confirmations follow, one per entity (§14 Decision 26). The report notes "N removed, more remain; run purge again".
- Create: `app/src/shell/sessions/SessionStateSheet.tsx`, opened from the Sessions view row.
  - It views state as text or hex, and edits it as text or base64.
  - Before Clear or Set it offers "Copy current state" and "Save current state…" (parked Minor 14, backup).
  - Over 256 KiB it needs "Allow large state".
  - A `conflict` error is shown with the core wording.
- Modify: `shell/tree/*` and `shell/palette/commands.ts`.
  - Pinned and `EntityPath` entities call `resolve_kind` once per connection and show the resolved glyph, or "unknown" (R23 desktop half, Decision 39).
  - The palette gets a command for every operation, scoped to the focused entity: Send, Receive, Delete selected, Move selected, Resubmit selected, Purge…, Cancel scheduled, Session state, Import…, Show activity.
  - The tree context menu has the same actions.
- Test: `PurgeSheet.test.tsx`, `SessionStateSheet.test.tsx`, `tree/resolvedKind.test.tsx`, `palette/commands.test.ts`

- [ ] **Step 1:** Write the failing tests:
  - `purge of two entities shows two plans and asks twice on production`
  - `purge plan with an unknown count is refused before confirm`
  - `AtLeast(0) deferred reads "an unknown number of deferred messages"`
  - `session state clear offers copy first`
  - `pinned entity shows topic glyph after resolve`
  - `palette lists every operate command for a queue, and no Rules commands`
- [ ] **Step 2:** Run the failing tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npx vitest run`, `npx tsc --noEmit` and `npx eslint src`. Expected: PASS.
- [ ] **Step 5:** Commit: `app: purge, session state, resolved kinds, palette operate commands`.

### Task 10: Emulator gate, impeccable pass, docs

**Files:**
- Create: `app/src-tauri/tests/emulator_m2b.rs`, behind `DISPATCHED_EMULATOR=1`, printing `skipped:` otherwise like the other gates. It runs the desktop ops layer (`ops::operate`) against the emulator with a `Recorder` and auto-answering `Confirms`:
  1. send 5 to a queue;
  2. receive 2 with complete;
  3. delete 1 by sequence;
  4. move 1 to a second queue;
  5. resubmit 1 from the DLQ with an edit;
  6. purge both queues (two confirmations);
  7. set, get and clear session state;
  8. check that `activity.db` (temp `DISPATCHED_CONFIG_DIR`) holds a started and a finished `gui` entry for every step.
- Modify: `.github/workflows/ci.yml`. The emulator job also runs `cargo test --locked -p dispatched-app --test emulator_m2b -- --nocapture`, and the pass check expects it.
- Create: `docs/live-verification-m2b.md`, with the emulator results plus the manual desktop walkthrough on the emulator: every sheet, a production profile pointed at the emulator for the typed confirmations, and a CLI run while the app is open. The M2a cloud checklist is carried forward unchanged.
- Modify: spec §15, "M2b decisions", recording this plan's rulings: GuiPolicy via events, activity polling at 3 s, int64 as strings, purge plan ids, session state backup. Modify `README.md` with the desktop operate section.
- Run the impeccable and design-taste-frontend review over every new sheet and view in the dev app (`npm run dev` with the mock transport), and fix what it finds.

- [ ] **Step 1:** Write the emulator test and run it with the emulator up. Expected: PASS, with no `skipped:`.
- [ ] **Step 2:** Run all four existing gates and the new one, then the full `cargo test --locked --workspace`, clippy, fmt, `npx vitest run`, `tsc` and `eslint`. Expected: all green.
- [ ] **Step 3:** Do the impeccable pass and its fixes. Commit them: `app: impeccable pass on operate screens`.
- [ ] **Step 4:** Write the docs. Commit: `docs: M2b live verification, decisions, README`.
- [ ] **Step 5:** Run the whole-branch review (most capable model) with `superpowers:requesting-code-review` over `m2a..m2b`. Do one fix round, then a scoped re-review.

## Parked (not in M2b)

- Template collection files (Decision 34): they need real incumbent sample files. → M3, with XML import/export.
- Atomic batch send (Decision 28). → v1.x.
- Single-subscription GET in management. → M3 (entity CRUD needs it anyway).
- The listener (§4.8). → M3 (§11).
