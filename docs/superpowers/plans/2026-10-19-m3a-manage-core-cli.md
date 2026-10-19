# M3a — Manage (core + CLI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. The user chose the lighter process: larger batches, emulator gates, and one whole-branch review at the end. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Full management of a namespace from the core and the CLI:
- queues, topics, subscriptions, rules and entity SAS policies, each created, read, updated and deleted with every property;
- status changes;
- duplicate and rename;
- bulk delete;
- XML export and import compatible with ServiceBusExplorer;
- a listener core: a live receive stream.

**Architecture:**
- `mgmt/` gains a typed model of every entity description, an ATOM writer, and PUT/DELETE on `ManagementClient`. An update is read-modify-write with `If-Match: *`.
- Every change goes through the existing safety gate as `Mutation::EntityChange`, with new `Effect` variants, as an `Operator` method.
- `mgmt/incumbent.rs` reads and writes the incumbent's `Entities` XML.
- `ops/listen.rs` runs N receivers until cancelled and streams `Received` events.

**Tech Stack:** Rust, quick-xml 0.36 (reader already in use; writer new), reqwest, mockito for unit tests, the local emulator for the gate.

**Spec:** `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`
- §4.3 Entity management
- §4.8 Listener
- §5 Safety
- §7 CLI (`entity create|update|delete|export|import`, `rule ls|add|rm`, `listen <entity>`)
- §11 (M3 row)
- §14 Decisions 3, 7, 9, 10, 12 and 13, which a listener must respect

The incumbent XML research is in `2026-10-19-m3a-manage-core-cli/m3a-incumbent-xml.md (in .superpowers/sdd/)`: the format from `ImportExportHelper.cs`, with line references.

## Global Constraints

- **Disk hygiene in every prompt that may build:**
  - never create extra target dirs;
  - never copy the repo or a crate to build or test it;
  - `CARGO_TARGET_DIR` only `~/.cache/cargo-shared-target`;
  - run `df -h /System/Volumes/Data` before heavy builds, and stop under 30 GB.
- **No real Azure.** The emulator (`testing/emulator/emulator.sh`) accepts ATOM PUT and DELETE for queues (probed: 201 and 200). Listing rules over management returns 500 there, so rules are unit-tested with mockito, and the gate notes it.
- **Every change is gated** (§5): confirm, audit before, run, audit after. On production the typed confirmation covers every entity the change touches, so a rename lists source and target, and a bulk delete lists every entity. Confirmation text is `Operation::summary`, naming the entity and the effect ("Delete queue orders and its 1,284 messages on prod-weu"). Nothing is reversible.
- **Secrets:** SAS keys are never logged, never written to activity entries, reports or stderr, and never shown unless asked.
  - `policy ls` shows key names and rights only. `policy keys <entity> <name>` prints keys, on request only.
  - XML export omits keys unless `--include-keys`, which warns on stderr that the file holds secrets. Import accepts keys.
- **ATOM bodies:** the elements of a `*Description` must follow the service's schema order (the XSD sequence). The writer emits a fixed order per kind, taken from the Azure REST reference. Durations in ATOM are ISO-8601 (`PT1M`, `P14D`, `P10675199DT2H48M5.4775807S`).
- **Incumbent XML:**
  - Root `<Entities xmlns="http://schemas.microsoft.com/servicebusexplorer" serviceBusNamespace="…">`.
  - TimeSpans in .NET "c" format (`00:01:00`, `14.00:00:00`, `10675199.02:48:05.4775807`). Booleans `True`/`False`. `ForwardTo` as the bare entity name.
  - Correlation user properties as `<Property>key: value</Property>`.
  - Import: namespace-qualified lookup; ignore unknown elements; accept ISO-8601 durations and booleans in any case; split a property on the FIRST colon and trim one leading space.
- **CLI:** clap derive in `main.rs`; handlers in `commands/`. New mutating commands join `mutates()` and the namespace hint list. `--format table|json`; JSON is stable.
- No attribution lines in commits.

## Review Focus

1. **An update that would drop a property the client does not model.** It must keep it. Read-modify-write works on the parsed tree: elements the model does not know are preserved in place. Test (Task 1): `unknown_elements_survive_an_update`.
2. **A rename or duplicate of a queue with messages.** A rename deletes the original, so with messages present it is refused before any confirmation ("{n} messages would be lost; move them first"). A duplicate copies settings, not messages, and says so. Tests (Task 4): `rename_refuses_a_queue_with_messages`, `duplicate_copies_settings_only`.
3. **Importing an incumbent file whose entities partly exist.** Existing ones are skipped items ("already exists") unless `--overwrite`. The rest are created. A subscription of an existing topic is still created (the incumbent's bug, fixed). Test (Task 5): `import_skips_existing_and_creates_the_rest`.
4. **A correlation filter with every field empty** (#794). Create, update and import refuse it, and refuse even when it has properties with empty names; the message says it matches every message, and says to use a SQL `1=1` filter or the default rule. Test (Task 2): `empty_correlation_filter_is_refused`.
5. **Ctrl-C on a listener in receive-and-delete mode with prefetch.** Every removed message is printed or listed in the report, including those the close sweeps, and none is lost silently (§14 Decision 13). Test (Task 6): `listener_close_reports_swept_messages`.

---

### Task 1: Entity description model and ATOM writer

**Files:**
- Create: `crates/dispatched-core/src/mgmt/model.rs`, `crates/dispatched-core/src/mgmt/atom_write.rs`
- Modify: `mgmt/xml.rs`. `Node` keeps attributes (`attrs: Vec<(String,String)>`), so `i:type` survives.
- Modify: `mgmt/mod.rs` (exports)
- Test: the same files; fixtures in `crates/dispatched-core/tests/fixtures/` (add `subscription_entry.xml`, `rule_entry.xml`, `queue_entry_full.xml`)

**Interfaces:**
- Produces:
  - `pub struct QueueSettings`: all `Option`s so that "not set" is distinct.
    - `lock_duration`, `max_size_in_megabytes`, `requires_duplicate_detection`, `requires_session`, `default_message_time_to_live`, `dead_lettering_on_message_expiration`, `duplicate_detection_history_time_window`, `max_delivery_count`, `enable_batched_operations`, `status: Option<EntityStatus>`, `forward_to`, `forward_dead_lettered_messages_to`, `auto_delete_on_idle`, `enable_partitioning`, `enable_express`, `max_message_size_in_kilobytes`, `user_metadata`
    - durations are `std::time::Duration`; "infinite" is `TimeSpan::MAX`, kept exactly.
  - `pub struct TopicSettings`: max size, duplicate detection plus window, TTL, batched, status, auto-delete, partitioning, express, support ordering, max message size, user metadata.
  - `pub struct SubscriptionSettings`: lock duration, requires session, TTL, dead-lettering on expiration, dead-lettering on filter evaluation exceptions, max delivery count, batched, status, forward to, forward DLQ to, auto-delete, user metadata.
  - `pub enum EntityStatus { Active, Disabled, SendDisabled, ReceiveDisabled }`
  - `pub struct RuleSpec { name: String, filter: RuleFilter, action: Option<String> }`, reusing the existing `RuleFilter`; `True` is added to it.
  - `pub struct AccessPolicy { key_name, rights: Vec<AccessRight>, primary_key: SecretString, secondary_key: SecretString }`
  - `pub enum AccessRight { Listen, Send, Manage }`
  - `pub struct Description<T> { settings: T, policies: Vec<AccessPolicy>, raw: Node }`. `raw` keeps the parsed description so an update preserves unknown elements.
  - `pub fn parse_queue(entry: &Node) -> Result<Description<QueueSettings>>`, and the same for topic, subscription and rule.
  - `pub fn queue_entry(d: &Description<QueueSettings>) -> String`, and likewise for the others. These write the full `<entry><content type="application/xml"><QueueDescription …>` with the schema order, ISO durations and `i:type` on filters and actions.
  - `pub fn iso_duration(d: Duration) -> String` and `parse_iso_duration(&str) -> Result<Duration>`, both round-tripping `P10675199DT2H48M5.4775807S`.
  - `pub fn validate_filter(f: &RuleFilter) -> Result<()>`: refuses an empty correlation filter (#794).

- [ ] **Step 1:** Write the failing tests:
  - `queue_entry_round_trips_every_setting`: parse `queue_entry_full.xml`, write it, parse again, and the result is equal.
  - `unknown_elements_survive_an_update`: a description with `<FutureFlag>true</FutureFlag>` keeps it in place after a settings change.
  - `iso_durations_round_trip_including_max`
  - `sql_and_correlation_filters_write_their_i_type`: the `i:type="SqlFilter"` and `CorrelationFilter` attributes, with correlation `Properties` as `KeyValueOfstringanyType` entries carrying `d3p1:type` / `i:type="d6p1:string"` the way the REST API expects.
  - `empty_correlation_filter_is_refused`
  - `keys_never_appear_in_debug_output`: `format!("{:?}", policy)` holds no key.
- [ ] **Step 2:** Run `cargo test -p dispatched-core --lib mgmt::`. Expected: FAIL.
- [ ] **Step 3:** Implement. The schema orders come from the Azure Service Bus REST reference (QueueDescription, TopicDescription, SubscriptionDescription, RuleDescription); copy each list into a `const ORDER: &[&str]`. The writer emits known elements in that order and keeps the raw elements it does not model at their original position relative to known neighbours.
- [ ] **Step 4:** Run `cargo test -p dispatched-core --lib`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: typed entity descriptions and an ATOM writer`.

### Task 2: ManagementClient writes

**Files:**
- Modify: `crates/dispatched-core/src/mgmt/mod.rs`
- Test: the mockito tests in the same file

**Interfaces:**
- Consumes: Task 1.
- Produces, on `ManagementClient`:
  - `get_queue_description(name) -> Result<Description<QueueSettings>>`, and the same for topic, subscription (`get_subscription(topic, name)`: the single-subscription GET parked in §15) and rule.
  - `create_queue(name, &Description<QueueSettings>) -> Result<()>`: PUT with no `If-Match`. A 409 is `Error::Conflict("queue {name} already exists")`.
  - `update_queue(name, &Description<QueueSettings>) -> Result<()>`: PUT with `If-Match: *`. A 404 is `NotFound`.
  - `delete_queue(name) -> Result<()>`: DELETE; a 404 is `NotFound`.
  - The same three for topics, subscriptions (`topic, name`) and rules (`topic, subscription, rule`).
  - `create_subscription_with_rule(topic, name, &Description<SubscriptionSettings>, Option<&RuleSpec>)`: the initial rule goes in the subscription's `DefaultRuleDescription`.
- All writes send `Content-Type: application/atom+xml;type=entry;charset=utf-8` and the same token the reads use. `map_http_error` maps 409 to `Conflict` and 412 to `Conflict("changed meanwhile")`. Create and update call `validate_filter` first.

- [ ] **Step 1:** Write the failing mockito tests:
  - `create_queue_puts_the_entry`: method, path, content type, no If-Match, body equal to `queue_entry`.
  - `update_sends_if_match_star`
  - `create_existing_is_conflict`
  - `delete_missing_is_not_found`
  - `get_subscription_reads_one`
  - `subscription_create_carries_the_initial_rule`
  - `rule_create_puts_rule_description`
- [ ] **Step 2:** Run `cargo test -p dispatched-core --lib mgmt::`. Expected: FAIL.
- [ ] **Step 3:** Implement. The private helpers are `put(path, body, if_match: bool)` and `delete(path)`; both percent-encode path segments as `get` does.
- [ ] **Step 4:** Run `cargo test -p dispatched-core --lib`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: create, update and delete over the management API`.

### Task 3: SAS policies

**Files:**
- Create: `crates/dispatched-core/src/mgmt/policies.rs`
- Test: the same file

**Interfaces:**
- Produces:
  - `pub fn new_key() -> SecretString`: 32 random bytes, base64.
  - `pub fn add_policy(d: &mut Description<_>, name, rights) -> Result<()>`: refuses a duplicate name, and refuses `Manage` without `Listen` and `Send` (the service requires both).
  - `pub fn remove_policy(d, name) -> Result<()>`
  - `pub fn regenerate(d, name, which: KeySlot) -> Result<()>`
  - `pub struct PolicyView { key_name, rights }`, with no keys.
  - Every change is applied through `update_queue` / `update_topic` (read-modify-write). Subscriptions have no policies.

- [ ] **Step 1:** Write the failing tests:
  - `add_then_remove_policy`
  - `manage_needs_send_and_listen`
  - `regenerate_changes_only_that_key`
  - `policy_view_has_no_keys` (the serialised JSON holds no key)
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p dispatched-core --lib`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: entity SAS policies`.

### Task 4: Gated manage operations

**Files:**
- Create: `crates/dispatched-core/src/ops/manage.rs`
- Modify: `crates/dispatched-core/src/safety.rs` (new `Effect` variants with `verb`, `summary` and `entities`), `ops/mod.rs` (`Operator` methods)
- Test: the same files. Effects are unit-tested in `safety.rs`; operations use mockito management plus a `MemoryAudit` gate.

**Interfaces:**
- Produces, on `Operator`. Each method is gated as `Mutation::EntityChange` and returns `JobReport` (one item per entity) or `()`:
  - `create_entity(spec: EntitySpec) -> Result<JobReport>`, where `EntitySpec` is `Queue(name, Description<QueueSettings>) | Topic(..) | Subscription { topic, name, d, rule: Option<RuleSpec> }`.
  - `update_entity(spec: EntitySpec) -> Result<JobReport>`
  - `delete_entities(targets: Vec<EntityRef>) -> Result<JobReport>`: bulk delete; one gate request per entity on production; the summary counts the messages each holds.
  - `set_status(target: EntityRef, status: EntityStatus) -> Result<JobReport>`
  - `duplicate(source: EntityRef, new_name: String) -> Result<JobReport>`: queue or subscription; settings, rules and policies are copied with new keys; messages are not copied, and the item detail says so.
  - `rename(source: EntityRef, new_name: String) -> Result<JobReport>`: queue or topic, done as create-copy then delete-original. It is refused before the gate when the entity holds messages (Review Focus 2). A topic rename copies its subscriptions and rules, which must all be empty too. The confirmation lists source and target.
  - `add_rule(sub: EntityRef, rule: RuleSpec)`, `remove_rule(sub, name)`
  - `add_policy`, `remove_policy`, `regenerate_key` (Task 3), each gated.
  - The `Effect` summaries:
    - "Create queue orders on {profile}"
    - "Delete topic events, its 3 subscriptions and their 1,204 messages on {profile}"
    - "Rename queue a to b (create b, then delete a) on {profile}"
    - "Set queue orders to SendDisabled on {profile}"
    - "Add rule eu to events/subscriptions/billing on {profile}"
    - "Remove policy app from orders on {profile}"
    - "Regenerate the primary key of policy app on orders on {profile}"

- [ ] **Step 1:** Write the failing tests:
  - `effects_name_every_entity_they_touch`: a rename lists 2; a bulk delete lists each.
  - `rename_refuses_a_queue_with_messages`: refused before the gate; the audit stays empty.
  - `duplicate_copies_settings_only`: the PUT body has the source settings and new keys; the detail says "messages are not copied".
  - `delete_topic_summary_counts_its_subscriptions`
  - `denied_change_does_nothing`: no PUT or DELETE reaches mockito; one `denied` entry.
  - `rename_rolls_back_when_the_delete_fails`: the copy stays; the item fails with "b was created; a could not be deleted ({error}); delete one of them".
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p dispatched-core --lib`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: gated entity, rule and policy changes`.

### Task 5: Incumbent XML export and import

**Files:**
- Create: `crates/dispatched-core/src/mgmt/incumbent.rs`
- Modify: `ops/manage.rs` (`Operator::import_entities`)
- Test: the same files, plus fixture `crates/dispatched-core/tests/fixtures/incumbent_entities.xml` (the reconstructed example from the research, with a `$Default` rule)

**Interfaces:**
- Produces:
  - `pub fn export_entities(ns_name: &str, entities: &[ExportEntity], include_keys: bool) -> String`. `ExportEntity` is a queue or a topic with its subscriptions and rules, collected by `Explorer::collect_for_export(selection) -> Result<Vec<ExportEntity>>`.
  - `pub fn parse_entities(xml: &str) -> Result<Vec<ImportEntity>>`
  - `Operator::import_entities(items: Vec<ImportEntity>, overwrite: bool, progress, cancel) -> Result<JobReport>`. It runs one gate for the whole import, whose summary lists every entity. Items: created, updated (with `overwrite`), or skipped as "already exists". The order is queues, topics, subscriptions, then rules.
  - `.NET "c"` TimeSpan format and parse: `dotnet_timespan(Duration) -> String`, `parse_dotnet_timespan(&str)`. The parser also accepts ISO-8601.

- [ ] **Step 1:** Write the failing tests:
  - `export_matches_the_incumbent_shape`: the root namespace and attribute, `True`/`False`, the "c" TimeSpans, bare `ForwardTo`, and `<Property>tenant: acme</Property>`.
  - `export_omits_keys_unless_asked`
  - `parse_reads_the_incumbent_example`: the 1 queue; the topic with 1 subscription; the SQL rule and the correlation rule with a property; and `$Default` maps to `True`.
  - `parse_property_splits_on_the_first_colon`: `url: http://x` becomes key `url`, value `http://x`.
  - `parse_ignores_unqualified_elements_and_unknown_ones`
  - `import_skips_existing_and_creates_the_rest`: mockito answers 409 for one queue; a subscription under an existing topic is still created.
  - `export_then_parse_round_trips`
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p dispatched-core --lib`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: incumbent-compatible entity XML export and import`.

### Task 6: Listener core

**Files:**
- Create: `crates/dispatched-core/src/ops/listen.rs`
- Modify: `ops/mod.rs` (`Operator::listen`), `safety.rs` (`Effect::Listen`)
- Test: the same file, against the fake broker

**Interfaces:**
- Produces:
  - `pub struct ListenRequest { entity: EntityPath, sub_queue: SubQueue, sessions: ListenSessions, mode: ListenMode, concurrency: u8 (1..=8), prefetch: u32 (0..=100) }`
  - `pub enum ListenMode { Watch, Complete, Delete }`:
    - `Watch`: peek-lock, then `release` each message; delivery count unchanged; read-only, not gated.
    - `Complete`: peek-lock auto-complete.
    - `Delete`: receive-and-delete.
  - `pub enum ListenSessions { None, Id(String), Each }`. `Each` takes next-available sessions in turn, the session-handler variant.
  - `Operator::listen(req, progress, cancel) -> Result<JobReport>`:
    - It runs until `cancel`, emitting `JobEvent::Received` per message and `Progress { done }`.
    - `Complete` and `Delete` are gated once at start (`Effect::Listen`, `CountEstimate::Unknown`): "Remove every message that arrives on orders until stopped, on {profile}". `Watch` is not gated.
    - Peek-lock keeps credit 1 per receiver (§14 Decision 10). `prefetch` applies to `Delete` only; peek-lock with prefetch is refused.
    - On cancel every receiver closes; the messages the close sweeps are emitted and counted (§14 Decision 13).
    - The report holds counts, not items: a listener can run for hours. `report.notes` gets "n messages received, m removed".

- [ ] **Step 1:** Write the failing tests (fake broker, paused clock):
  - `watch_releases_and_leaves_delivery_count`
  - `complete_removes_each_and_streams_it`
  - `delete_with_prefetch_never_exceeds_prefetch_credit`
  - `listener_close_reports_swept_messages`
  - `concurrency_runs_n_receivers`
  - `peek_lock_with_prefetch_is_refused`
  - `each_session_moves_to_the_next_session`
  - `cancel_ends_within_two_seconds`
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement. Reuse `MessagingBackend::open_receiver` and `LockReceiver::{receive, settle, release, close}`; there is one task per receiver, joined under the cancel token.
- [ ] **Step 4:** Run `cargo test -p dispatched-core --features test-support`. Expected: PASS.
- [ ] **Step 5:** Commit: `core: listener`.

### Task 7: CLI

**Files:**
- Create: `crates/dispatched-cli/src/commands/manage.rs`, `commands/listen.rs`
- Modify: `main.rs` (subcommands), `commands/mod.rs`
- Test: `crates/dispatched-cli/tests/` arg-parsing tests and `commands/manage.rs` unit tests

**Interfaces:**
- **Entity commands:**
  - `entity create <queue|topic|subscription> <name> [--lock-duration 1m] [--max-size 1024] [--ttl 14d] [--auto-delete-on-idle …] [--duplicate-detection 10m] [--max-delivery-count 10] [--forward-to X] [--forward-dlq-to X] [--sessions] [--partitioning] [--batched/--no-batched] [--dead-letter-on-expiration] [--metadata TEXT] [--rule-sql EXPR | --rule-correlation K=V…]`
    - A subscription name is `topic/subscriptions/name`.
    - Durations use the existing `parse_duration` syntax.
  - `entity update <entity> [same flags]`: only the given flags change.
  - `entity delete <entity>...`: bulk; production needs `--confirm-prod` for each.
  - `entity status <entity> active|disabled|send-disabled|receive-disabled`
  - `entity duplicate <entity> <new-name>`, `entity rename <entity> <new-name>`
  - `entity export [<entity>...] --out FILE [--include-keys] [--force]`: the incumbent XML; with no entity it exports everything.
  - `entity import FILE [--overwrite]`
- **Rule commands:** `rule add <subscription> <name> (--sql EXPR | --correlation FIELD=VALUE... [--property K=V...]) [--action SQL]`, `rule rm <subscription> <name>`. `rule ls` exists.
- **Policy commands:** `policy ls <entity>`, `policy add <entity> <name> --rights listen,send[,manage]`, `policy rm <entity> <name>`, `policy renew <entity> <name> primary|secondary`, `policy keys <entity> <name>`. `policy keys` prints keys and warns on stderr; it is a read and not logged.
- **Listen:** `listen <entity> [--dlq] [--session ID | --each-session] [--complete | --delete] [--concurrency N] [--prefetch N] [--format table|json]`.
  - It streams one line (table) or one JSON object per message until Ctrl-C, then prints the summary.
  - `--complete` and `--delete` are gated with the usual `--yes` and `--confirm-prod`.
- Every mutating command uses `operator_for` with a `Planned { kind: Mutation::EntityChange, verb }` and joins `mutates()`.

- [ ] **Step 1:** Write the failing tests:
  - argument parsing for each command;
  - `create_flags_map_to_settings`;
  - `update_changes_only_given_flags`;
  - `policy_ls_prints_no_keys`;
  - `export_refuses_existing_file_without_force`;
  - `listen_delete_needs_yes_without_a_terminal`.
- [ ] **Step 2:** Run `cargo test -p dispatched`. Expected: FAIL.
- [ ] **Step 3:** Implement. The README CLI reference gets the new commands.
- [ ] **Step 4:** Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check`. Expected: all green.
- [ ] **Step 5:** Commit: `cli: entity, rule, policy and listen commands`.

### Task 8: Emulator gate and docs

**Files:**
- Create: `crates/dispatched-core/tests/emulator_m3a.rs`
- Modify: `.github/workflows/ci.yml` (the emulator job runs it; the count check is updated)
- Create: `docs/live-verification-m3a.md`
- Modify: spec §16 "M3a decisions"

**Steps:**
- [ ] **Step 1:** Write the gate, behind `DISPATCHED_EMULATOR=1`, printing `skipped:` otherwise. It uses names with a run id (`m3a-{run}-…`) and deletes them at the end, on every path. It covers:
  1. create a queue with non-default settings, then read it back exactly;
  2. update two settings; the others are unchanged;
  3. status to SendDisabled, then back;
  4. duplicate;
  5. rename (empty);
  6. a rename with a message is refused;
  7. create a topic and a subscription with an initial SQL rule, read back if the emulator allows (otherwise note it);
  8. add and remove a policy, if the emulator allows;
  9. export the created entities to incumbent XML, delete them, import, and read back equal;
  10. a bulk delete of all;
  11. listener: send 5, listen with `Complete` and concurrency 2 until 5 are received, then cancel; the queue is empty; `Watch` leaves the delivery count at 0;
  12. the activity entries: started and finished for every gated step.
- [ ] **Step 2:** Run it with the emulator up. Expected: PASS, no `skipped:`. Run all earlier gates too.
- [ ] **Step 3:** Write the docs: the emulator results, the cloud checklist (Standard and Premium, Entra and SAS, partitioned entities, policies, rules, a real incumbent export file) and the decisions.
- [ ] **Step 4:** Commit: `docs: M3a live verification and decisions`.
- [ ] **Step 5:** Run the whole-branch review (most capable model), do one fix pass, then write the final report.

## Parked (not in M3a)
- The desktop screens for all of the above, the listener UI (rolling grid, rate chart, save or resubmit from it) and the forwarding-target picker over the tree: M3b.
- MCP tools: M4.
