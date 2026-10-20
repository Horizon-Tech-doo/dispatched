# M3b Desktop Manage and Listener Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The desktop gets every M3a management operation (entities, status, duplicate, rename, bulk delete, rules, SAS policies, incumbent XML export and import) and the listener (rolling grid, rate chart, save, resubmit), through the same core gate as the CLI.

**Architecture:** The Tauri side adds read commands (`entity_settings`, `entity_policies`, checks) and `op_*` job commands built on `start_op` (`app/src-tauri/src/ops/operate.rs`), exactly as M2b did, so confirmations, items, reports and activity entries reuse `GuiPolicy`, `ProgressBridge` and `EmittingAudit`. One core change makes the desktop and CLI share their settings rules: `SettingsChange` moves the CLI's "only these settings change" logic into core. The webview adds sheets on the existing `OperationSheet` frame, a Policies section on Overview, rule editing on the Rules tab and a real Listener tab.

**Tech Stack:** Rust (dispatched-core, Tauri 2 app), React 19 + TypeScript, TanStack Query, Vitest, Playwright, the local Service Bus emulator.

**Spec:** `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` (§4.3, §4.8, §5, §6, §15 M2b decisions, §16 M3a decisions). Follow-ups: `docs/live-verification-m3a.md` "Follow-ups".

## Global Constraints

- Every mutating operation goes through core `Gate`/`Operator`; the webview never words a confirmation itself (§15 Decision 1). Production: the typed entity name, once per entity the operation names (both names for rename and duplicate, every entity of a bulk delete).
- Confirmations are inline in the sheet or panel that started the operation, never a modal (§6, §15 Decision 3).
- Side sheets, not modals, for entity create and edit (§6).
- SAS keys never cross IPC and are never shown: copying a key or a connection string is done by Rust writing the clipboard, as `copy_entity_connection_string` does. No key in any view type, event, log, activity entry or file except the `include_keys` export (§16 Decisions 14, 15).
- Files are read only from Rust-opened dialogs through one-time pick ids, and written only to the path a Rust-opened save dialog returned (§15 Decision 9).
- Sequence numbers cross IPC as decimal strings (§15 Decision 7).
- Visual direction (§6): no gradients, glows, cards, pills, emoji; one accent; colour carries meaning only; tabular numerals; system UI font plus monospace for names, IDs, bodies. Every UI task ends with the impeccable and design-taste-frontend skills; Task 12 is the screen-level impeccable review pass.
- Disk hygiene (user rule): build only in the real checkout's `target/` (never a new CARGO_TARGET_DIR, never a copy of the repo); `df -h /System/Volumes/Data` before heavy builds, stop under 30 GB.
- No attribution lines in commits.

## Review Focus

1. **An entity changed by someone else while its edit sheet was open.** Expected: the update re-reads the description and applies only the fields the user changed; other fields keep the newer values. Pinned by `update_applies_only_changed_fields_to_a_fresh_description` (Task 3).
2. **Durations at the edges:** `TimeSpan.MaxValue` ("never"), sub-second values, and values like 1d1h1m1s shown in the form and saved unchanged. Pinned by `duration_text_round_trips` (Task 1) and `untouched_duration_field_is_not_sent` (Task 7).
3. **A destructive listener running for hours.** Expected: it never drops a removed message it can no longer show. At the held cap it stops itself and says why, and the report holds every message. Pinned by `destructive_listener_stops_at_the_held_cap` (Task 4).
4. **Closing the Listener tab, switching profile or disconnecting while a listener runs.** Expected: the listener stops, its report stays in Jobs, and nothing keeps receiving out of sight. Pinned by `closing_the_tab_stops_its_listener` (Task 11) and the M2b stop-by-close path (Task 4 test `listener_stopped_by_close_keeps_messages`).
5. **Renaming or deleting the entity of an open tab.** Expected: a rename moves the tab to the new name, and a delete shows the "wasn't found" state. Neither leaves a tab that errors on every refresh. Pinned by `rename_moves_the_open_tab` and `delete_marks_the_tab_gone` (Task 6).

---

### Task 1: Core — shared settings changes, duration text, rule update

**Files:**
- Create: `crates/dispatched-core/src/mgmt/change.rs`
- Modify: `crates/dispatched-core/src/mgmt/mod.rs` (module + re-export), `crates/dispatched-core/src/messaging/props.rs` (add `duration_text`), `crates/dispatched-core/src/ops/manage.rs` (add `update_rule`), `crates/dispatched-core/src/safety*` where `Operation::add_rule` lives (add `Operation::update_rule`), `crates/dispatched-cli/src/commands/manage.rs` (SettingsArgs → SettingsChange)
- Test: unit tests in `change.rs`, `props.rs`, `ops/manage.rs` (FakeBroker/mockito as the existing manage tests do)

**Interfaces:**
- Produces:
  - `pub struct SettingsChange` (Default, Clone, Debug, `Deserialize` with `#[serde(default, deny_unknown_fields)]`). Every settable property of §4.3 is an `Option`: `lock_duration`, `max_size_in_megabytes: Option<u64>`, `requires_duplicate_detection: Option<bool>`, `requires_session`, `default_message_time_to_live`, `dead_lettering_on_message_expiration`, `dead_lettering_on_filter_evaluation_exceptions`, `duplicate_detection_history_time_window`, `max_delivery_count: Option<u32>`, `enable_batched_operations`, `status: Option<EntityStatus>`, `forward_to: Option<String>` (`""` clears), `forward_dead_lettered_messages_to: Option<String>` (`""` clears), `auto_delete_on_idle`, `enable_partitioning`, `enable_express`, `support_ordering`, `max_message_size_in_kilobytes: Option<u64>`, `user_metadata: Option<String>` (`""` clears). Durations are `Option<Duration>`, deserialized from text through `parse_duration`, with `max`/`never` meaning `TimeSpan::MAX`.
  - `impl SettingsChange { pub fn apply_queue(&self, s: &mut QueueSettings, create: bool) -> Result<()>; pub fn apply_topic(..TopicSettings..); pub fn apply_subscription(..SubscriptionSettings..); pub fn is_empty(&self) -> bool; pub fn issues(&self, kind: EntityKind, create: bool) -> Vec<FieldIssue> }`
  - `pub struct FieldIssue { pub field: &'static str, pub message: String }` (Serialize). `field` is the `SettingsChange` field name.
  - `pub fn duration_text(d: Duration) -> String` in `messaging/props.rs`.
  - `Operator::update_rule(&self, sub: EntityRef, rule: RuleSpec) -> Result<JobReport>`, with JobKind `UpdateEntities` and one item.

- [ ] **Step 1: Write the failing tests**

In `change.rs`:
- `refuses_a_setting_the_kind_lacks`: `support_ordering: Some(true)` → `apply_queue(.., true)` is `Err(Invalid)` naming "support_ordering"; `lock_duration` on a topic fails the same way.
- `refuses_creation_only_settings_on_update`: `requires_session` and `enable_partitioning` with `create=false` → Err (§16 Decision 25). `requires_duplicate_detection` on update → Err.
- `duplicate_window_on_create_turns_detection_on`, and `duplicate_window_on_update_needs_detection`: the window on an entity with `requires_duplicate_detection == Some(false)` → Err.
- `empty_string_clears_forwarding_and_metadata`: `forward_to: Some("")` on settings with `forward_to: Some("x")` → `None`; the same for the DLQ target and metadata.
- `only_given_fields_change`: apply `max_delivery_count: Some(3)` to a fully populated `QueueSettings`, then assert equality with the original except that field.
- `deserializes_duration_text`: JSON `{"lock_duration":"45s","default_message_time_to_live":"never","auto_delete_on_idle":"P7D"}` → 45 s, `TimeSpan::MAX`, 7 d; `{"lock_duration":"0s"}` → error; unknown field → error.
- `issues_lists_each_bad_field_once`: two bad fields → two `FieldIssue`s, in field order.

In `props.rs`:
- `duration_text_round_trips`: for 1 s, 45 s, 90 s, 1 h 30 m, 1 d 1 h 1 m 1 s, 14 d, 1.5 s, 250 ms and `TimeSpan::MAX`, `parse_duration`/max(`duration_text(d)`) == d. Expected texts: `"45s"`, `"1m30s"`, `"1h30m"`, `"1d1h1m1s"`, `"14d"`, `"1s500ms"`, `"250ms"`, `"max"`.

In `ops/manage.rs`:
- `update_rule_is_gated_and_reads_back`: update rule `eu` of a subscription with a new SQL filter. The gate sees one `EntityChange` whose summary reads "Update rule eu of events/Subscriptions/billing on dev". The PUT carries `If-Match: *`. The report has one succeeded item.
- `update_rule_refuses_an_empty_correlation_filter_before_the_gate`: the gate sees nothing, and the error is `Invalid` with the #794 wording.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --locked -p dispatched-core --features test-support change:: duration_text update_rule 2>&1 | tail -20`
Expected: compile errors (`SettingsChange`, `duration_text`, `update_rule` not found).

- [ ] **Step 3: Implement**

- `SettingsChange::apply_*`: move the bodies of the CLI's `SettingsArgs::apply_*`, `check`, `duplicate_window`, `name_or_clear` and `metadata` into core, extended to the fields the CLI lacked (express, ordering, max message size, filter-evaluation dead-lettering). Field-to-kind table:
  - queue: everything except `support_ordering` and `dead_lettering_on_filter_evaluation_exceptions`
  - topic: `max_size_in_megabytes`, `requires_duplicate_detection`, duplicate window, TTL, batched, status, `auto_delete_on_idle`, partitioning, express, `support_ordering`, max message size, metadata
  - subscription: lock, sessions, TTL, both dead-lettering flags, `max_delivery_count`, batched, status, both forwards, `auto_delete_on_idle`, metadata
- `issues` runs the same checks without settings and collects every failure instead of stopping at the first.
- `duration_text`: `TimeSpan::MAX` → `"max"`. Otherwise concatenate the non-zero d/h/m/s/ms parts, largest first. A remainder below 1 ms is shown in ms, rounded up. `parse_duration` must accept `ms`; add it if it doesn't, with a test.
- `update_rule`: validate the filter (`validate_filter`) before the gate. Gated: `get_rule`, then replace filter and action on the parsed description so the raw tree keeps unchanged correlation value types (§16 Decision 5), then `update_rule`. Add `Operation::update_rule` next to `Operation::add_rule`, with the same summary shape.
- CLI: `SettingsArgs` gets `fn change(&self) -> anyhow::Result<SettingsChange>`, which maps the flags (`--ttl` → `default_message_time_to_live`, `--duplicate-detection` → window, `batched`/`no_batched` → `enable_batched_operations`). `apply_*` then call `change()?.apply_*`. The CLI's own `apply_*`, `check`, `duplicate_window`, `name_or_clear` and `metadata` are deleted, and the CLI's existing tests must pass unchanged.

- [ ] **Step 4: Run tests**

Run: `cargo test --locked --workspace --features dispatched-core/test-support 2>&1 | grep -E "^test result|FAILED|panicked" | head -20`
Expected: every `test result: ok`, the CLI manage tests included.

- [ ] **Step 5: Commit**

`git commit -m "feat(core): shared SettingsChange, duration_text, Operator::update_rule"`

---

### Task 2: App — manage reads, checks and key copies

**Files:**
- Create: `app/src-tauri/src/ops/manage_types.rs`, `app/src-tauri/src/ops/manage_read.rs`
- Modify: `app/src-tauri/src/ops/mod.rs`, `app/src-tauri/src/commands.rs`, `app/src-tauri/src/lib.rs` (register), `app/src-tauri/src/clipboard.rs` (reuse)
- Test: unit tests in both new files (mockito management server, as `ops/entity.rs` tests do)

**Interfaces:**
- Consumes: `SettingsChange`, `FieldIssue`, `duration_text`, `validate_filter`, `policies::views`, `ManagementClient::get_*_description` / `get_subscription` / `get_rule`.
- Produces (Tauri commands; JSON field names are snake_case like the existing views):
  - `entity_settings(profile_id, kind: EntityKindName, entity: String) -> SettingsView`. `SettingsView { kind, entity, settings: Map<field, value>, fixed: Vec<&'static str>, status }`. `settings` uses the `SettingsChange` field names, with durations as `duration_text` and absent values as `null`. `fixed` lists the creation-only fields (`requires_session`, `enable_partitioning`, `requires_duplicate_detection`) that apply to the kind.
  - `entity_policies(profile_id, entity) -> Vec<PolicyView>`: name and rights only. A subscription → `invalid` "Subscriptions have no shared access policies."
  - `check_settings(kind, form: SettingsChange (as JSON), create: bool) -> Vec<FieldIssue>`: local, no connection. A form that does not parse returns one issue per field that failed, e.g. `{field:"lock_duration", message:"'0s' is not a positive duration"}`. Implement by deserializing field by field from a `serde_json::Map`.
  - `check_rule(rule: RuleForm) -> Vec<FieldIssue>`, where `RuleForm { name: String, filter: RuleFilterForm, action: Option<String> }` and `RuleFilterForm` is `{kind:"true"} | {kind:"sql", expression} | {kind:"correlation", correlation_id?, message_id?, to?, reply_to?, subject?, session_id?, reply_to_session_id?, content_type?, properties:[{name, value, type?}]}`, mirroring the read-side `RuleFilter` JSON. `impl TryFrom<RuleForm> for RuleSpec`. An empty name → issue on `name`. `validate_filter` failures → issue on `filter`.
  - `copy_policy_connection_string(profile_id, entity, policy, slot: "primary"|"secondary") -> ()` and `copy_policy_key(profile_id, entity, policy, slot) -> ()`: Rust reads the description and writes the clipboard. Nothing returns. A key read is a read, so it is not logged (§16 Decision 14). The clipboard write behaves exactly like `copy_entity_connection_string`'s.

- [ ] **Step 1: Write the failing tests**
- `settings_view_shows_durations_as_text`: a queue description with lock PT45S and TTL `TimeSpan.MaxValue` → `settings.lock_duration == "45s"`, `settings.default_message_time_to_live == "max"`, and `fixed` contains `requires_session`.
- `policy_views_have_no_keys`: serialize `entity_policies` output to JSON; the string contains neither `"primary"` nor the key text from the mockito body.
- `subscription_has_no_policies`: → `CommandError` kind `invalid`.
- `check_settings_reports_each_field`: `{"lock_duration":"0s","support_ordering":true}`, queue, create → two issues, `lock_duration` then `support_ordering`.
- `rule_form_to_spec`: `true` → `RuleFilter::True`. Correlation with a property of `type:"int"` gives a typed property. The empty correlation is refused with the #794 wording.
- `copy_policy_connection_string_never_returns_it`: through a fake clipboard (the existing clipboard trait/test double), the clipboard holds `Endpoint=sb://…;SharedAccessKeyName=app;SharedAccessKey=<key>;EntityPath=orders`, and the command's return value is `()`.

- [ ] **Step 2: Run them; expect compile failures.**
Run: `cargo test --locked -p dispatched-app --lib manage_ 2>&1 | tail -15`

- [ ] **Step 3: Implement the commands.** They follow `ops/entity.rs`'s pattern: active explorer for `profile_id`, else `not_connected`. Correlation property `type` is optional and defaults to `string`. The allowed types are those core's correlation writer knows.

- [ ] **Step 4: Run** `cargo test --locked -p dispatched-app --lib 2>&1 | grep -E "test result|FAILED"`. Expected: ok.

- [ ] **Step 5: Commit** `feat(app): entity settings, policy views, settings and rule checks, key copies`

---

### Task 3: App — manage operations as jobs

**Files:**
- Create: `app/src-tauri/src/ops/manage.rs`
- Modify: `app/src-tauri/src/commands.rs`, `app/src-tauri/src/lib.rs`, `app/src-tauri/src/jobs.rs` (import previews), `app/src-tauri/src/ops/operate.rs` (`start_op` visibility only if needed)
- Test: unit tests in `ops/manage.rs` (the M2b runner/unit pattern, recorded in the M2b ledger: FakeBroker is core test-only, so app tests exercise form→spec builders and the preview store; the gated paths are proven by the Task 5 emulator gate)

**Interfaces:**
- Consumes: Task 1 and Task 2 types, `start_op`, `Done::Report`, `operate_types::ItemView`.
- Produces (each returns `JobStarted`; the title is the job title shown in Jobs):
  - `op_entity_create(profile_id, kind, entity, settings: SettingsChange, rule: Option<RuleForm>)`, title "Create queue orders". `entity` for a subscription is `topic/subscriptions/name` (the tab id form). `EntityRef::parse` handles it; add case-insensitive `subscriptions` if it doesn't.
  - `op_entity_update(profile_id, kind, entity, changes: SettingsChange)`: refused `invalid` "Nothing to change." when `is_empty()`. Inside the job it reads the fresh description and applies `changes` with `create=false`. Title "Update queue orders".
  - `op_entity_delete(profile_id, entities: Vec<{kind, entity}>)`. Title "Delete 3 entities" or "Delete queue orders".
  - `op_entity_status(profile_id, kind, entity, status)`, `op_entity_duplicate(profile_id, kind, entity, new_name)`, `op_entity_rename(...)`.
  - `op_rule_add(profile_id, subscription, rule: RuleForm)`, `op_rule_update(profile_id, subscription, rule: RuleForm)`, `op_rule_remove(profile_id, subscription, name)`.
  - `op_policy_add(profile_id, entity, name, rights: Vec<AccessRight>)`, `op_policy_remove(profile_id, entity, name)`, `op_policy_renew(profile_id, entity, name, slot)`.
  - `pick_entities_export_path(namespace: String) -> PickedPath|null`: suggested name `"{namespace}-entities-{local stamp}.xml"`.
  - `entities_export(profile_id, entities: Vec<{kind, entity}> (empty = the whole namespace), include_keys: bool, pick) -> ExportedEntities { count: usize, file_name: String }`: core `collect_for_export` + `export_entities`, written through `FileExport::create_private`. It is a read: not gated, not logged.
  - `OpenPurpose` gains `"entities"` (filter `*.xml`, single file).
  - `entities_import_preview(profile_id, pick) -> ImportPreview { preview_id, entities: Vec<{kind, entity, rules: usize, policies: usize, keys: bool}> }`. It parses with `parse_entities`, and a refusal is `invalid` with core's text. Previews are held in `Jobs` like purge plans: they run once, on the same profile, within `PLAN_TTL`.
  - `op_entities_import(profile_id, preview_id, overwrite: bool)`, JobKind `ImportEntities`.

- [ ] **Step 1: Write the failing tests**
- `update_applies_only_changed_fields_to_a_fresh_description` (Review Focus 1): the description-building function the job uses, `fn updated(current: Description<QueueSettings>, changes: &SettingsChange) -> Result<Description<QueueSettings>>`, given a "fresh" description whose `max_size` differs from what the form was opened with plus changes `{max_delivery_count: 3}` → the max size stays fresh, max delivery is 3, and `raw` is untouched.
- `empty_update_is_refused_before_a_job`: → `invalid` "Nothing to change.", and no job registered (`state.jobs` empty).
- `preview_runs_once_on_its_profile`: take a preview, then take it again → `invalid` "Choose the file again."; another profile id → `invalid`.
- `import_preview_lists_rules_and_policies`: a fixture incumbent file (reuse the core parser test's fixture through `include_str!`) → the counts per entity, and `keys:false` for a policy without keys.
- `delete_title_names_one_or_counts_many`.

- [ ] **Step 2: Run** `cargo test --locked -p dispatched-app --lib ops::manage 2>&1 | tail -15`. Expected: compile errors.
- [ ] **Step 3: Implement.** Each `op_*` builds its core request, then `start_op(…, move |op, b, c| async move { report(op.<method>(…).await) })`. Import passes `&*b, &c` for progress and cancel.
- [ ] **Step 4: Run** the app lib tests. Expected: ok.
- [ ] **Step 5: Commit** `feat(app): manage operations as gated jobs, incumbent XML export and import`

---

### Task 4: App — listener jobs

**Files:**
- Create: `app/src-tauri/src/ops/listen.rs`
- Modify: `app/src-tauri/src/ops/operate.rs` (`ProgressBridge` optional holder), `commands.rs`, `lib.rs`, `jobs.rs`
- Test: unit tests in `ops/listen.rs` driving `ProgressBridge` with synthetic `CoreEvent::Received`

**Interfaces:**
- Consumes: `Operator::listen`, `ListenRequest`, `ListenOutcome`, `StoredReport { report, messages }`.
- Produces:
  - `op_listen(profile_id, form: ListenForm) -> JobStarted`, with `ListenForm { entity, sub_queue, mode: "watch"|"complete"|"delete", sessions: {type:"none"}|{type:"id", id}|{type:"each"}, concurrency: u8, prefetch: u32 }`. The job kind is `listen`. Titles: "Watch orders", "Listen to orders (receive and complete)", "Listen to orders (receive and delete)".
  - Held messages: for `complete` and `delete` the bridge keeps every `PeekedMessage` it sees, in arrival order. Caps: `HELD_MAX_MESSAGES = 10_000` and `HELD_MAX_BYTES = 256 * 1024 * 1024` (sum of `body_size`). At either cap the bridge records the note "Stopped at 10,000 held messages: save the report, then start again." (or "… at 256 MiB of held messages …") and cancels the job's token with a stop reason, so the job ends `stopped`, not `cancelled`. `watch` holds nothing.
  - Done: `Done::Report(outcome.report, held)`. A listener the user stopped is its normal end (§16 Decision 20). `finish()` must therefore not turn a user stop of a `listen` job into an error: the event has `error: null`, and the summary is the report's first note. A cap stop sends `error {kind:"stopped", message:"<the note>"}`. A stop by a closed connection keeps the M2b `stopped` mapping.
  - `job_message(id, sequence: String) -> MessageDetail | null`: the full message from a finished or running job's held messages (receive jobs included). The body is capped as `message_detail` caps it.

- [ ] **Step 1: Write the failing tests**
- `watch_holds_nothing`: 3 received events in watch mode → the bridge holds 0, and 3 `received` events are emitted.
- `destructive_listener_holds_every_message`: complete mode, 3 received → 3 held, in order.
- `destructive_listener_stops_at_the_held_cap` (Review Focus 3): with caps overridden for the test through a constructor parameter (`Held::with_caps(2, u64::MAX)`), 3 received → the token is cancelled after the 2nd, the note text is "Stopped at 2 held messages: save the report, then start again.", and the 3rd (already removed by then) is still held. It is never dropped: the cap stops the listener, it does not discard.
- `user_stop_of_a_listener_is_not_an_error`: `finish()` on a `listen` job whose token the user cancelled → the finished event has no `error`.
- `listener_stopped_by_close_keeps_messages`: the token is cancelled through `jobs.stop_all_by_close` → error kind `stopped`, and the held messages are in the stored report.
- `job_message_finds_a_held_message`: by sequence "9007199254740993" (above 2^53) → found.
- `listen_form_refusals`: prefetch with complete, concurrency 0 or 9, or a session id with concurrency > 1 → `invalid` before a job (core `listen::check`, surfaced unchanged).

- [ ] **Step 2: Run, expect failures.** `cargo test --locked -p dispatched-app --lib ops::listen`
- [ ] **Step 3: Implement.** `ProgressBridge::new` keeps its signature. Add `ProgressBridge::holding(id, events, held: Held, cancel)`. `Held` is a `Mutex` with the vec, the bytes and the caps. Put the cap stop reason into `Jobs` (`jobs.stop_with(id, kind, message)`), and make `stop_kind`/`stop_message` read it first.
- [ ] **Step 4: Run** the app lib tests. Expected: ok.
- [ ] **Step 5: Commit** `feat(app): listener jobs with held messages and job_message`

---

### Task 5: Desktop emulator gate `emulator_m3b`

**Files:**
- Create: `app/src-tauri/src/ops/emulator_m3b.rs` (`#[cfg(test)]`, gated on `DISPATCHED_EMULATOR=1` like `emulator_m2b`)
- Modify: `app/src-tauri/src/ops/mod.rs`, `.github/workflows/ci.yml` (the emulator job runs `emulator_m3b` after `emulator_m2b`)

**Interfaces:**
- Consumes: the Task 2–4 functions behind the commands, called as `emulator_m2b` calls `operate::*`, with the same test events sink and the production-marked emulator profile answering typed confirmations.

- [ ] **Step 1: Write the gate.** Entities are `m3b-{run}-…`, each deleted by a guard on every path (copy the `emulator_m3a` guard). Steps:
  1. Create a queue (lock 45 s, max delivery 7, TTL 30 min). `entity_settings` reads back `"45s"`, `7`, `"30m"`.
  2. Update `{max_delivery_count: 3}` → `entity_settings` shows 3, and lock is still `"45s"`.
  3. Status `SendDisabled`, then `Active`.
  4. Duplicate; then rename the empty copy. Typed confirmations for both names, in order.
  5. Topic plus a subscription created with an initial SQL rule; `op_rule_add` a correlation rule; `op_rule_update` it to SQL; `op_rule_remove`. `list_rules` after each.
  6. Export (`entities_export` to a pick created in the test from a temp-file path under the scratch dir the test owns, then deleted) → delete them through `op_entity_delete` (one job, typed confirmations per entity) → `entities_import_preview` → `op_entities_import` → read back equal.
  7. Listener: send 5 to the queue. `op_listen` watch → 5 `received` events; stop; no activity entry. Then `op_listen` complete with concurrency 2; stop after 5 → `job_message` finds each of the 5 sequences; peek shows 0.
  8. Activity: one started and one finished `gui` entry per gated step, each `Succeeded`.

  Emulator limits print `GATE-NOTE` as `emulator_m3a` does: policies dropped (the policy add/remove step runs and checks only the job and its activity entries), metadata dropped.
- [ ] **Step 2: Restart the emulator if needed, then run** `DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-app --lib emulator_m3b -- --nocapture 2>&1 | tail -20`. Expected: `emulator_m3b: all steps passed`.
- [ ] **Step 3: Run it a second time.** Expected: passes again, and only the `Config.json` entities remain afterwards (list via CLI `dispatched … ls`).
- [ ] **Step 4: Commit** `test(app): emulator_m3b desktop manage and listener gate; CI`

---

### Task 6: Engine and shell plumbing

**Files:**
- Modify: `app/src/engine/types.ts`, `app/src/engine/commands.ts`, `app/src/engine/queries.ts` (keys: `settings`, `policies`), `app/src/engine/mock/transport.ts` and `app/src/engine/mock/operate.ts` (handlers for every Task 2–4 command), `app/src/shell/operate/advice.ts` (`rightFor`: the entity kinds → `"Manage"`, `listen` → `"Listen"`), `app/src/shell/operate/words.ts` (titles for the new kinds), `app/src/shell/operate/invalidate.ts` and `JobRefresh.tsx` (after a manage job: the listing, subscriptions, rules, settings and policies of the entities it names), `app/src/shell/state.ts` (new reducer action `{type:"retarget", from, to}`)
- Test: `app/src/engine/mock/manage.test.ts`, `app/src/shell/operate/invalidate.test.ts`, `app/src/shell/tabs/tabs.test.tsx`

**Interfaces:**
- Produces (TS): `commands.entitySettings`, `entityPolicies`, `checkSettings`, `checkRule`, `copyPolicyConnectionString`, `copyPolicyKey`, `opEntityCreate`, `opEntityUpdate`, `opEntityDelete`, `opEntityStatus`, `opEntityDuplicate`, `opEntityRename`, `opRuleAdd`, `opRuleUpdate`, `opRuleRemove`, `opPolicyAdd`, `opPolicyRemove`, `opPolicyRenew`, `pickEntitiesExportPath`, `entitiesExport`, `entitiesImportPreview`, `opEntitiesImport`, `opListen`, `jobMessage`. Types: `SettingsChange` (all optional; durations `string`), `SettingsView`, `FieldIssue`, `RuleForm`, `PolicyView`, `ImportPreview`, `ListenForm`, `EntityRefForm {kind, entity}`.
- The mock engine keeps a mutable in-memory namespace, so a create shows up in the tree and a delete removes it. It asks confirmations through the same mock confirm flow as M2b operations, and its listener emits a received event every 200 ms until it is cancelled.

- [ ] **Step 1: Write the failing tests**
- `mock create then list shows the queue`; `mock delete removes it`; `mock listener emits until cancelled`.
- `rename_moves_the_open_tab` (Review Focus 5): with an open tab `orders` on sub-tab `rules`, a finished rename job of `orders` → `orders2` dispatches `retarget`. The tab id becomes `orders2`, the sub-tab is kept, and it stays active.
- `delete_marks_the_tab_gone`: after a delete job finishes, the tab's `useEntityInfo` resolves `gone` once the listing refetches (mock), and does not error-loop. Assert the query is not refetched more than once.
- `manage_job_invalidates_the_names_it_touched`: a finished `update-entities` job on `events/subscriptions/billing` invalidates `keys.subscriptions(p,"events")`, `keys.rules(p, id)`, `keys.settings(p, id)`.
- **Step 2:** `cd app && npx vitest run src/engine/mock/manage.test.ts src/shell/operate/invalidate.test.ts src/shell/tabs/tabs.test.tsx`. Expected: FAIL.
- **Step 3:** Implement.
- **Step 4:** Run the same command plus `npx tsc --noEmit`. Expected: PASS, no type errors.
- **Step 5:** Commit `feat(app): engine commands, mock namespace and job plumbing for manage and listen`

---

### Task 7: Entity create and edit sheets

**Files:**
- Create: `app/src/shell/manage/SettingsForm.tsx`, `app/src/shell/manage/EntitySheet.tsx`, `app/src/shell/manage/fields.ts` (the field table: kind, group, label, help, input type, creation-only), `app/src/shell/manage/DurationInput.tsx`
- Modify: `app/src/shell/operate/seed.ts` (seeds `{op:"entity-create", kind, topic?}` and `{op:"entity-edit", kind, entity}`), `OperationHost.tsx`, `app/src/shell/palette/commands.ts` (`manageCommands(tab, c)`: "New queue…", "New topic…", "New subscription on {topic}…", "Edit settings of {name}…"), `app/src/shell/tree/NamespaceTree.tsx` (context menu: New queue / New topic at the namespace node, New subscription on a topic, Edit settings on an entity; plus a "New…" icon button beside Explorer options), `app/src/shell/overview/OverviewView.tsx` ("Edit settings…" button in its header)
- Test: `app/src/shell/manage/EntitySheet.test.tsx`

**Interfaces:**
- Consumes: Task 6 commands; `EntityPicker` (forwarding targets, `exclude` = the entity itself); `OperationSheet` and `useOperation`; `RuleEditor` from Task 9. Task 9 builds it, so this task's subscription-create form imports it, and Task 7 lands after Task 9 in execution order (see the ordering note at the end).
- Layout: groups in this order: Name (create only); Messages (lock duration, max delivery count, TTL, dead-letter on expiration, dead-letter on filter exceptions); Duplicate detection (on/off at create, window); Forwarding (forward to, forward dead-lettered to, each with the picker and "None"); Size (max size MB, max message size KB, shown for Premium values only when set or on create, labelled "Premium only"); Lifecycle (auto-delete on idle, with "never"); Advanced (sessions, partitioning, batched, express, ordering); Metadata (textarea). A creation-only field shows its value read-only on edit, with "Set when the entity was created." Subscription create ends with "Initial rule" (default: the `$Default` rule that matches every message; or SQL or correlation through `RuleEditor`).
- Edit state: the sheet keeps the opened `SettingsView` values and sends `SettingsChange` holding only the fields whose value differs from the opened value. A cleared forward or metadata sends `""`. The primary button names the count: "Save 2 changes", disabled at 0.
- Validation: every change debounces 250 ms into `checkSettings(kind, change, create)` (and `checkRule`). Issues show under their fields through `FieldMessage`; the primary is disabled while any issue stands. A name must be non-empty with no `/` (local check).

- [ ] **Step 1: Write the failing tests**
- `create_queue_sends_only_set_fields`: type the name `orders`, set lock to `45s`, submit → `opEntityCreate` called with `("queue","orders",{lock_duration:"45s"}, null)`. The confirmation appears in the sheet; confirm; the report shows.
- `untouched_duration_field_is_not_sent` (Review Focus 2): the edit sheet opened on TTL `"max"` and lock `"1m30s"`; change only max delivery → `opEntityUpdate` changes `== {max_delivery_count: 3}`. The TTL field displays "max" and the lock "1m30s".
- `clearing_forward_sends_empty_string`.
- `creation_only_fields_are_read_only_on_edit`: the sessions control is disabled, and the text "Set when the entity was created." shows.
- `field_issue_shows_under_its_field_and_blocks_submit`: the mock `checkSettings` returns an issue for `lock_duration` → the message is under the Lock duration field, and the primary is disabled.
- `subscription_create_with_correlation_rule`: the payload's `rule.filter.kind == "correlation"`.
- `palette_and_tree_menu_open_the_sheet`.
- **Step 2:** `npx vitest run src/shell/manage/EntitySheet.test.tsx`. Expected: FAIL.
- **Step 3:** Implement. `DurationInput` is a text input with the placeholder "e.g. 45s, 5m, 1h30m, 14d" plus a "Never" checkbox where the field allows max (TTL, auto-delete). Help text sits under the label, not in tooltips.
- **Step 4:** Run the tests plus `npx tsc --noEmit && npx eslint src/shell/manage`. Expected: PASS.
- **Step 5:** Run the impeccable skill and the design-taste-frontend skill on `SettingsForm`/`EntitySheet` (the mock app at `npm run dev`, screenshot through Playwright in both themes) and fix what they find. Commit `feat(app): entity create and edit sheets`.

---

### Task 8: Status, duplicate, rename and delete sheets

**Files:**
- Create: `app/src/shell/manage/StatusSheet.tsx`, `CopySheet.tsx` (duplicate and rename, `mode` prop), `DeleteEntitiesSheet.tsx`
- Modify: `seed.ts` (`entity-status`, `entity-duplicate`, `entity-rename`, `entity-delete {entities: EntityRefForm[]}`), `OperationHost.tsx`, `palette/commands.ts` (`manageCommands`), `NamespaceTree.tsx` menu, `dashboard/DashboardView.tsx` ("Delete N selected…" next to Purge), `OverviewView.tsx` (the status shows with a "Change…" link)
- Test: `app/src/shell/manage/sheets.test.tsx`

**Interfaces:**
- StatusSheet: segmented choices per kind. Queue: Active, Disabled, SendDisabled, ReceiveDisabled. Topic: Active, Disabled, SendDisabled. Subscription: Active, Disabled, ReceiveDisabled. Each has a one-line meaning ("Send disabled: senders are refused; receivers go on."). The current status is preselected, and submit is disabled until it changes.
- CopySheet: duplicate offers queue and subscription (topic has no duplicate; the command does not show for topics). It says "Copies settings, rules and policies (with new keys), not messages." Rename offers queue and topic, and says "Creates {new} with the same settings, then deletes {old}. Refused while {old} holds messages." The new-name field is prefilled `{name}-copy`; a subscription copy stays under its topic.
- DeleteEntitiesSheet lists every entity with its glyph. For a topic: "and its subscriptions". The counts come from the core's confirmation sentence, not the sheet.
- After a successful rename the Task 6 `retarget` runs. After a delete the tab shows `gone`.

- [ ] **Step 1: Failing tests:**
  - `status_offers_kind_statuses`: a topic shows no ReceiveDisabled.
  - `rename_refusal_shows_core_text`: the mock refuses `conflict` "orders holds about 1 message; …" → a `Problem` with that detail, and the form is kept.
  - `bulk_delete_from_dashboard`: two selected → `opEntityDelete` with both; two typed confirmations in order on a production profile.
  - `duplicate_not_offered_for_topics`.
- **Step 2:** `npx vitest run src/shell/manage/sheets.test.tsx`. Expected: FAIL.
- **Step 3:** Implement.
- **Step 4:** Run tests, tsc and eslint. Expected: PASS.
- **Step 5:** impeccable plus taste pass, then commit `feat(app): status, duplicate, rename and bulk delete sheets`.

---

### Task 9: Rules editing and policies

**Files:**
- Create: `app/src/shell/manage/RuleEditor.tsx`, `app/src/shell/manage/RuleSheet.tsx` (add and edit), `app/src/shell/manage/PoliciesSection.tsx`, `app/src/shell/manage/PolicySheet.tsx` (add)
- Modify: `app/src/shell/rules/RulesView.tsx` (an "Add rule…" toolbar button, a per-rule menu with Edit…, Remove…, Copy filter; `ruleKind` handles `kind:"true"` → "Matches every message (default rule)"), `app/src/engine/types.ts` (`RuleFilter` gains `{kind:"true"}`), `OverviewView.tsx` (`PoliciesSection` for queues and topics), `seed.ts`, `OperationHost.tsx`, `palette/commands.ts`
- Test: `app/src/shell/manage/rules.test.tsx`, `app/src/shell/manage/policies.test.tsx`, `app/src/shell/rules/RulesView.test.tsx` (extend)

**Interfaces:**
- `RuleEditor({ value: RuleForm, onChange, issues })`. The filter kind is chosen with a segmented control: Default (matches every message), SQL, Correlation. SQL is a monospace textarea. Correlation has the 8 system fields (the labels of `RulesView` `FIELDS`) plus a properties table with name, value and type (string, int, long, double, boolean, datetime). The optional SQL action is a monospace textarea. A correlation filter with nothing set shows the #794 issue from `checkRule` at once: "This filter matches no message…".
- Remove rule: an `OperationSheet` with no form ("Remove rule eu from billing"). The `$Default` rule can be removed. The sheet then says "Without a rule this subscription receives no messages." when it is the last rule.
- `PoliciesSection`: a table of Name, Rights and actions. Actions are Copy connection string ▾ (primary/secondary), Copy key ▾, Regenerate ▾ (primary/secondary → `OperationSheet` "Regenerate the primary key of app? Clients using it stop working."), Remove…. "Add policy…" opens `PolicySheet`: a name plus Listen/Send/Manage checkboxes; Manage checks and locks Listen and Send. The limit of 12 is shown, and the button is disabled at 12. The status bar confirms copies ("Copied the connection string of app"). No key is ever rendered.

- [ ] **Step 1: Failing tests:**
  - `true_filter_renders_as_default`: `$Default` with `{kind:"true"}` → "Matches every message (default rule)", not "Unknown filter".
  - `add_correlation_rule_payload`: two properties, one `int`.
  - `empty_correlation_shows_794_and_blocks`.
  - `edit_rule_prefills_and_sends_update`.
  - `remove_last_rule_warns`.
  - `policies_never_render_keys`: the mock returns views; assert that no text matching `/[A-Za-z0-9+/]{43}=/` is in the document.
  - `manage_right_locks_listen_and_send`.
  - `regenerate_goes_through_confirmation`.
- **Step 2:** Run those test files. Expected: FAIL.
- **Step 3:** Implement.
- **Step 4:** Run tests, tsc and eslint. Expected: PASS.
- **Step 5:** impeccable plus taste pass, then commit `feat(app): rule editing and shared access policies`.

---

### Task 10: Incumbent XML export and import sheets

**Files:**
- Create: `app/src/shell/manage/ExportEntitiesSheet.tsx`, `app/src/shell/manage/ImportEntitiesSheet.tsx`
- Modify: `seed.ts` (`entities-export {entities: EntityRefForm[]}`, `entities-import`), `OperationHost.tsx`, `palette/commands.ts` ("Export entities (ServiceBusExplorer XML)…", "Import entities from XML…"), the `NamespaceTree.tsx` namespace node menu, `DashboardView.tsx` ("Export N selected…")
- Test: `app/src/shell/manage/xml.test.tsx`

**Interfaces:**
- Export is a plain sheet, not an `OperationSheet`, since it is a read. Scope: "Whole namespace" or "N selected" (from the seed). The "Include keys" checkbox carries the warning "The file will hold the keys of every policy. Anyone with the file can send and receive." Then the Rust save dialog, then `entitiesExport`. The done line reads "Exported 7 entities to orders-entities-….xml" with a Reveal button (reuse `jobReveal`-style reveal: add `reveal_path(pick)` only if no existing helper fits; prefer the status bar line).
- Import is an `OperationSheet`. Choose file → `entitiesImportPreview` → a table of entities (kind, name, rules, policies, "keys in file") → the "Replace existing entities" checkbox (overwrite) with the note "Existing entities get the file's settings; policies without keys in the file keep their keys." → "Import N entities". The confirmation and report follow the M2b flow.

- [ ] **Step 1: Failing tests:**
  - `export_whole_namespace_without_keys` (payload `entities: []`, `include_keys:false`).
  - `include_keys_warns`.
  - `import_preview_then_import_with_overwrite`.
  - `import_parse_refusal_shows_core_text`.
- **Step 2:** Run. Expected: FAIL. **Step 3:** Implement. **Step 4:** Run tests, tsc and eslint. Expected: PASS.
- **Step 5:** impeccable plus taste pass, then commit `feat(app): incumbent XML export and import sheets`.

---

### Task 11: Listener tab

**Files:**
- Create: `app/src/shell/listener/ListenerView.tsx`, `ListenerForm.tsx`, `RateChart.tsx`, `listenerStore.ts`
- Modify: `app/src/shell/tabs/EntityView.tsx` (render `ListenerView` instead of the empty state), `app/src/shell/drawer/jobEvents.ts` (route `received` events of `listen` jobs to `listenerStore`), `app/src/shell/tabs/Workspace.tsx` or the tab-close reducer path (closing a tab cancels its listener), `connection.tsx` (a disconnect or profile switch ends listeners: the engine already stops jobs on close; the store marks them stopped)
- Test: `app/src/shell/listener/listener.test.tsx`, `app/src/shell/listener/listenerStore.test.ts`, `RateChart.test.tsx`

**Interfaces:**
- `listenerStore`, per job id: `rows` (a ring of the last `ROLLING_ROWS = 1000` `ReceivedView`s, newest first), `total`, `bytes`, and `buckets` (per second: messages and bytes, the last `RATE_WINDOW = 60` seconds). `useListener(jobId)`. One listener per tab: the tab remembers `listenerJobId` in the store keyed by `profileId|entity`.
- Form (shown while no listener runs): Mode, segmented: Watch (default; "Shows new messages and leaves them; nothing is locked"), Receive and complete, Receive and delete. Then Sub-queue (active, dead-letter, transfer DLQ); Sessions (only for session entities: one session id, or each session in turn); Receivers (1–8, default 1; Watch forces 1); Prefetch (delete mode only, 0–100). Start. Receive and delete is never remembered as the default (§5): the mode resets to Watch on every visit.
- Running: the confirmation (complete and delete modes) is inline in the panel through `ConfirmPanel` and the `useOperation` answer. Then a header line "Watching orders · 1,284 received · 12.4 msg/s · 3.1 KB/s" with a Stop button, then `RateChart`, then the grid (the existing `DataGrid` and `columns.tsx` subset: sequence, enqueued, message id, subject, size, preview) with `DetailPane` on select.
- `RateChart({ buckets })` is an inline SVG with two aligned rows, messages/s and bytes/s: 60 bars of 1 s each, no gradients, `fg` bars on `line`, a y-max label in tabular numerals, and an `aria-label` summarising the current and peak rates.
- Detail for a selected row: watch mode uses `messageDetail` (peek, by sequence, string); destructive modes use `jobMessage(jobId, seq)`.
- Row actions:
  - Watch: "Resubmit…" and "Move…" open the M2b sheets with `{entity, view, sequences: selected}`; "Delete…" the same.
  - Destructive modes: "Send again…" opens the Composer with `clone` = the held message detail.
  - Any mode: "Save report…" (after Stop: the M2b report save; it writes the held messages for destructive modes).
- Stopped: the header shows how it ended (the job's summary or stop note), and the grid stays until "New listener".

- [ ] **Step 1: Failing tests:**
  - `ring_keeps_last_1000_newest_first`.
  - `rate_buckets_roll_over_60_seconds` (fake timers).
  - `watch_starts_without_confirmation`.
  - `delete_mode_confirms_inline_on_production` (typed name).
  - `stop_cancels_the_job_and_keeps_rows`.
  - `closing_the_tab_stops_its_listener` (Review Focus 4): close the tab → `jobCancel(jobId)` called.
  - `delete_mode_is_not_remembered`: leave and come back → Watch.
  - `send_again_clones_the_held_message`.
  - `rate_chart_has_summary_label`.
- **Step 2:** Run. Expected: FAIL. **Step 3:** Implement. **Step 4:** Run tests, tsc and eslint. Expected: PASS.
- **Step 5:** impeccable plus taste pass on the running state with the mock's 5 msg/s stream, in both themes. Commit `feat(app): listener tab with rolling grid and rate chart`.

---

### Task 12: Screen review, end-to-end, docs

**Files:**
- Create: `app/e2e/manage.spec.ts`, `docs/live-verification-m3b.md`
- Modify: the spec (add `## 17. M3b decisions (2026-10-20)` with every ruling and decision taken here), `README.md` (desktop manage and listener section), `docs/live-verification-m3a.md` (Follow-ups: mark the M3b items done)

- [ ] **Step 1: Playwright e2e against the mock:** create a queue → it appears in the tree → edit max delivery → rename → the tab follows → delete → gone. Listener watch: rows appear, then stop.
  Run: `cd app && npx playwright test e2e/manage.spec.ts`. Expected: pass. Then run the whole e2e suite.
- [ ] **Step 2: impeccable review pass** over every new screen, as one session: the entity sheet (create and edit, every kind), status, copy, delete, rules, policies, XML, and the listener. Use the `impeccable` skill's critique/audit and fix every P1/P2. P3s go into the doc as deferred.
- [ ] **Step 3: Full checks:**
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --features dispatched-core/test-support -- -D warnings`
  - `cargo test --locked --workspace --features dispatched-core/test-support`
  - `cd app && npx tsc --noEmit && npx eslint . && npx vitest run && npx playwright test`
  - the emulator gates `emulator_m3a` and `emulator_m3b`

  Expected: all clean. Write the counts into `docs/live-verification-m3b.md` (format of the M2b note: an emulator table, an unchecked desktop walkthrough, a cloud checklist).
- [ ] **Step 4: Commit** `docs: M3b decisions, verification note, README`

**Execution order:** 1, 2, 3, 4, 5, 6, 9, 7, 8, 10, 11, 12. Task 7 imports `RuleEditor` from Task 9.
