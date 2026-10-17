# M2a Operate (core + CLI): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `dispatched-core` and the `dispatched` CLI every M2 "Operate" capability: the safety gate (operation kinds, confirmation with the exact effect, production checks, audit before and after), a persisted SQLite activity log, job results per message, the message plane (send, batch send, schedule, cancel scheduled, peek-lock and receive-and-delete receive, complete/abandon/defer/dead-letter, renew lock, deferred receive, session receivers and session state), the composer core (all system properties, typed custom properties, auto-increment MessageId, a real TTL duration, BrokeredMessageTemplate files), and DLQ and repair jobs (resubmit, move, delete, purge, DLQ summary, import and replay, clone and resend), plus the M1 carry-overs.

**Architecture:**
- **Core decides, surfaces translate.** Mutating broker calls exist only behind `ops::Operator`, whose every public mutating method builds an `Operation`, passes it through `safety::Gate` (policy confirmation, audit entry before, audit entry after) and only then touches the broker. The raw message plane (`messaging::backend::MessagingBackend`) is `pub(crate)`, so no surface can bypass the gate.
- **One message plane, two implementations.** `MessagingBackend` is implemented by `explorer::ExplorerPlane` (the cached AMQP connection, locked per call, never retried for mutations) and by `messaging::fake::FakeBroker` (in-memory, `#[cfg(test)]`), so every op is unit-tested without a network. The emulator gate `emulator_m2a` exercises every mutating op against the real emulator.
- **Jobs are plain async functions** that take a `&dyn Progress` sink and a `CancellationToken`, record one `ItemResult` per message, never stop at the first failure, and return a `JobReport` that can be written to JSON/NDJSON and retried. The CLI runs them in the foreground; M2b's desktop app spawns them.
- **Desktop wiring is M2b.** M2a changes the app crate only where core types change under it (it must keep compiling); it adds no Tauri commands, no Jobs drawer and no UI. The MCP server is M4 (spec §11), and nothing here blocks it: `Origin::Mcp` and the `Policy` trait are the only hooks it needs.

**Tech Stack:** Rust 2024 (MSRV 1.88) with `fe2o3-amqp` 0.18 (+ `-cbs`, `-management`, `-types`, `-ws`), `rusqlite` 0.40 (bundled), `quick-xml` 0.36, `serde`/`serde_json`, `time` 0.3, `uuid` 1, `tokio` 1, `clap` 4.6, `horizon-azure-kit` 0.2 (`safety::{Policy, ConfirmRequest, Decision, Origin, require, CancellationToken}`). No new crates.

**Spec:** `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` (binding: §2–§3, §4.4 receive/delete/import, §4.5, §4.6, §4.7, §4.9 Jobs and Activity log (core part), §5, §7 CLI, §8, §9, §11 M2 row, §13). Follow-ups bound into this plan: `docs/live-verification-m1c.md` "Follow-ups", `.superpowers/sdd/m1c-rulings.txt` (R21: the log records what the user did, not polling; R23: entity kind resolution moves to M2). Format precedent: `docs/superpowers/plans/2026-10-16-m1c-browse.md`.

## Global Constraints

- **Repo and branch:** `/Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched`, branch `m2a` (stacked on `m1c`). Never switch branches, never commit to `main`, `m1c` or `m1b`. The kit repo (`../horizon-azure-kit`) is not changed by this plan.
- **The code moves on:** before each task, read the files it modifies as they are now. Where a snippet's surroundings differ, keep the plan's behaviour, names and tests, adapt the snippet, and report each adaptation in the task report.
- **Disk hygiene (hard rule, from the user's CLAUDE.md):** build only in the repo's own `target/`. Never set `CARGO_TARGET_DIR`, never copy the repo or a crate to build or test it, never create extra target dirs. Before any heavy build (`cargo build`, `cargo test`, `cargo clippy`, `cargo check -p dispatched-app`, `docker compose up`) run `df -h /System/Volumes/Data` and **stop and tell the user if under 30 GB free**. Delete what a session created before reporting done (scratch files, Docker images it pulled: `docker image rm`, never `docker system prune`). Every subagent prompt that may build carries this rule verbatim.
- **Running processes:** never stop the user's running `npm run tauri:dev` or the Docker emulator. Task 22 changes `testing/emulator/Config.json`, which the emulator reads only at start: the implementer asks the user before running `testing/emulator/emulator.sh down && testing/emulator/emulator.sh up`, and until then reports the gate as "not run".
- **No real Azure in tests, ever.** Unit and CLI tests use fakes (`FakeBroker`, `MemoryAudit`, in-memory SQLite, scripted policies). The only network test is the emulator gate (`DISPATCHED_EMULATOR=1`, `testing/emulator`, the well-known key `SAS_KEY_VALUE`). Nobody signs in to Azure or connects to a real namespace while executing this plan.
- **Safety model (spec §5), enforced in core:**
  - Classification: `OpKind::Read` or `OpKind::Mutating(Mutation)`, `Mutation ∈ {Send, Settle, Delete, Purge, EntityChange, SessionState}`.
  - `Policy::confirm` (through `horizon_azure_kit::safety::require`) runs before any mutating broker call. A denial touches nothing and is logged.
  - Confirmation text names the exact effect; the spec's example is reproduced byte for byte: `Remove 1,284 messages from orders/$DeadLetterQueue on prod-weu`.
  - Production profiles: CLI needs `--yes` **and** `--confirm-prod <entity>` (the entity as the user names it, compared on `EntityPath::base()` case-insensitively). No interactive way around it.
  - Receive-and-delete is never a default: it needs its own flag (`--delete`) and is confirmed like a purge. Purge is never one step: it always confirms.
  - Every mutating op writes an activity entry **before** it starts (`phase = started`) and **another when it finishes** (`phase = finished`, with counts and outcome), tagged `cli`, `gui` or `mcp`.
- **Mutations are never retried.** A send, settle or delete that fails on a broken connection drops the cached connection (as reads do) but is reported, not repeated. Only opening a receiver or sender link (no broker side effect) may retry once.
- **Locks are never taken blindly:** peek-lock links use manual credit and never hold more credit than the messages still wanted; receive-and-delete links never hold more credit than the messages still allowed, so they can never remove more than requested (#590). Messages delivered after a stop are drained and reported, never dropped silently.
- **Secrets:** connection strings, keys, tokens and SAS signatures never appear in activity entries, job reports, confirmation texts, errors, test snapshots or CLI output. Activity entries carry profile names, entity paths, counts, outcomes and error text only.
- **Copy:** sentence case, plain words, no em dashes, no exclamation marks, no emoji. Errors say what happened and what to do. Numbers in human text use `,` thousands separators (`output::thousands` / `safety::count_text`).
- **Toolchain:** Rust 2024, MSRV 1.88. Run `cargo fmt --all` before every Rust commit; `cargo clippy --workspace --all-targets -- -D warnings` must pass at the end of each task (CI runs it). `#![deny(missing_docs)]` is not on in core, but every new `pub` item gets a one-line doc comment (house style).
- **Stable JSON:** every new type crossing to the CLI's `--format json` (and later IPC) is `Serialize` with snake_case fields and kebab-case enum tags, as declared in the task that adds it. Changing a field name later is a breaking change.
- **Commits:** one or more per task, message style `Area: what changed` (e.g. `Core: activity log in SQLite`). **No attribution lines.** Stage only the task's files (`git add <paths>`, never `-A`). Never push, tag or open a PR.

**Decisions taken while planning (spec ambiguities resolved; Task 23 records them in the spec as §14):**
1. **Safety types live in `dispatched-core`, not the kit.** The kit's `safety` has `Policy`, `ConfirmRequest`, `Decision`, `Origin`, `require` and `CancellationToken` but no operation kinds, jobs or audit log. Spec §2.1 puts "Job / JobEvent, Audit trait + SQLite audit log" in the kit, but the kit's rule is "only code both products need" and Arkived is not on the kit yet, so M2a builds `safety.rs`, `jobs.rs` and `activity.rs` in core and maps onto the kit's `ConfirmRequest` (`verb`, `target = EntityPath::base()`, `summary = exact effect text`, `item_count`, `reversible`, `production`, `origin`). Extraction into the kit happens when Arkived adopts them.
2. **Activity log file:** a separate SQLite database `activity.db` next to `dispatched.db` (`paths::open_activity_log`), its own `user_version` schema, WAL mode, busy timeout 5 s, so the CLI and the desktop app can write concurrently. Pruned on open to the newest 50,000 rows and to 90 days. In M2a the CLI records mutating ops only (reads are not logged, per R21's "what the user did" rule applied to a one-shot CLI); the GUI keeps its in-memory log until M2b moves it onto this file.
3. **Purge is bounded by what was confirmed.** The confirmation counts the messages first (management `CountDetails` when available: exact; otherwise a peek count: `about N`, or `at least 10,000` when capped). Purge then removes at most that many per sub-queue with receive-and-delete (credit never above the remaining allowance) and reports `N removed, more remain; run purge again` when messages are left. Deferred messages are purged by sequence number (receive-by-sequence-number, then complete). Scheduled messages are never purged; the confirmation says so and points to `scheduled cancel --all`. `all` = active + deferred + dead-letter + transfer dead-letter.
4. **Move, delete and remove-original use lock-and-match.** The wanted sequence numbers are first classified by peek (`collect_sequences`): missing ones are reported `missing` at once; deferred ones are taken by `receive-by-sequence-number`; scheduled ones (delete only) are cancelled; active ones are found with a peek-lock link at credit 1, acting on a match immediately and holding non-matching messages locked until the end, when they are **released** (AMQP `released`, not `modified{delivery-failed}`, so their delivery count is not raised). Limits: 5 s without a delivery ends the search, at most 10,000 messages examined and 2,000 held; anything not reached is reported `not-reached` with the reason (`timeout`, `budget`, `held-limit`). On session entities the search runs inside the message's session.
5. **Move is at-least-once (spec §4.5):** lock the source, send the copy, then complete the source. If the send fails, the lock is released and the item fails. If the complete fails after the send, the item is `failed` with reason `sent to {target}, but the original could not be removed ({error}); it may be delivered twice`. Resubmit with "remove original" is the same code path with edits allowed; resubmit without it is clone and resend (peek the source, send copies).
6. **Clone and resend copy rules:** body (Data sections and AMQP string values byte-exact; other AMQP-value or sequence bodies are re-encoded from their JSON form and the item's detail says `body re-encoded from JSON`), all user-settable system properties (MessageId unless a new one is asked for, SessionId, CorrelationId, Subject, To, ReplyTo, ReplyToSessionId, ContentType, PartitionKey, TimeToLive) and custom properties. Broker-owned values (sequence number, enqueued time, lock, delivery count, `x-opt-*` except partition key, scheduled time) are not copied. When the source is a dead-letter queue, the custom properties `DeadLetterReason` and `DeadLetterErrorDescription` are dropped unless `keep_dead_letter_properties` is set.
7. **TTL is a duration.** `PeekedMessage.time_to_live` becomes `Option<std::time::Duration>`, read from the AMQP header `ttl` or else from `absolute-expiry-time − creation-time` (so TTLs above the u32 millisecond cap of the header survive), serialized as `time_to_live_ms` (u64) so the export schema `dispatched.message/v1` and the app's TypeScript stay as they are. On send: header `ttl = min(ms, u32::MAX)` plus `creation-time = now` and `absolute-expiry-time = now + ttl` (saturating), as the Azure SDKs do.
8. **Batch send is pipelined, not atomic.** `send --batch` sends all messages over one sender link with up to 64 unsettled at a time and reports each one's outcome. The atomic AMQP batch format (`0x80013700`) is not used in v1; recorded as a deviation.
9. **Schedule goes through `$management`** (`com.microsoft:schedule-message`) so the broker returns the new sequence numbers that `scheduled cancel` takes.
10. **BrokeredMessageTemplate:** M2a reads a single incumbent template file in JSON (`messageId`, `sessionId`, `correlationId`, `contentType`, `label`, `partitionKey`, `to`, `replyTo`, `replyToSessionId`, `timeToLive`, `scheduledEnqueueTimeUtc`, `forcePersistence` (ignored), `body`, `properties: [{key, type, value}]`) or XML (root `brokeredMessage` in namespace `http://schemas.microsoft.com/servicebusexplorer`, same element names, `body` as CDATA, `properties/property/{key,type,value}`). Property `type` accepts the .NET names `String`, `Int32`, `Int64`, `Double`, `Boolean`, `DateTime`, `TimeSpan`, `Guid` (any case) and Dispatched's short names; `timeToLive` accepts .NET `TimeSpan` text (`[d.]hh:mm:ss[.fffffff]`). The incumbent's multi-template collection files and any type names beyond these are ruled to M2b: the incumbent's `MessagePropertyInfo.Type` is marked TODO in its source, so they need real sample files from a user, not guesses.
11. **Typed custom properties:** `string`, `int` (i32), `long` (i64), `double`, `bool`, `datetime` (AMQP timestamp), `timespan` (described `com.microsoft:timespan`, i64 ticks of 100 ns, as the .NET SDK writes it) and `guid` (AMQP uuid). The CLI syntax is `KEY=VALUE[:TYPE]` with `string` as default; a literal `:` in a string value is written `KEY=VALUE:string`. Decoding learns `timespan` (shown as `PropValue::TimeSpan`).
12. **Auto-increment MessageId:** the trailing digit run of the base id is incremented per message, keeping zero padding (`order-0009` → `order-0010`); a base without trailing digits gets `-1`, `-2`, …; `--message-id` without increment sends the same id N times (duplicate detection may drop them, which the CLI notes when N > 1).
13. **Lock tokens** of link deliveries are the 16-byte delivery tag read as a .NET GUID (`uuid::Uuid::from_bytes_le`); lock tokens from `receive-by-sequence-number` are AMQP uuids taken as they are. Management requests made on behalf of a receiver (renew lock, update disposition, session state) carry `associated-link-name` = the receiver link's name.
14. **Session state needs a session lock.** Reading or changing state accepts that session with a session receiver at credit 0 (no message is locked), then closes it. If another receiver holds the session, the op fails `conflict` ("session X is locked by another receiver; stop that receiver or wait for its lock to expire") and changes nothing.
15. **Entity kind resolution (R23):** `Explorer::resolve_kind(&EntityPath)` returns `Some(kind)` from the ATOM entry when the identity can read it; on `forbidden` it probes `peek-message` (count 1) on the entity's `$management`: success means a queue, a service refusal with any status other than 401/403/404/429/503 means a topic, anything else is `None` ("kind unknown"). Subscription paths are subscriptions without a call. The CLI's limited `ls` shows the resolved kind.
16. **Case-insensitive user property names in filters (M1a carry-over):** `user.X` matches the property named exactly `X` first; otherwise the first property whose name equals `X` ignoring ASCII case (map order). System property names were already case-insensitive.
17. **CLI exit codes for jobs:** `0` when every item succeeded or was skipped, `2` when the job ran but at least one item failed or was not reached, `1` for errors before the job started (bad input, denied confirmation, connection failure).
18. **MCP stays in M4.** No `dispatched-mcp` crate exists; spec §11 puts "full CLI and MCP surfaces" in M4. M2a only guarantees that an MCP policy can be plugged into `Gate` and that `Origin::Mcp` is logged as `mcp`.

## Review Focus

1. **A receive-and-delete or purge that takes more than the user agreed to:** a slow broker that delivers after the wait expired, credit granted ahead of need, a purge racing new arrivals. Expected: never more messages removed than the confirmed count; late deliveries are drained and reported in the result, not lost. Tests: Task 9 `receive_and_delete_never_grants_more_credit_than_remaining`, Task 12 `receive_and_delete_reports_messages_drained_after_the_wait`, Task 15 `purge_never_removes_more_than_confirmed`.
2. **Locks expiring during a long move, delete or resubmit-with-remove** (lock duration 30 s, thousands of messages to search). Expected: each match is acted on at once; a lock lost before settling fails that item with "the lock expired; run again for the failed sequence numbers"; a move whose complete fails after its send says the message may be delivered twice; the job continues. Tests: Task 13 `lost_lock_fails_only_that_item`, Task 14 `move_reports_possible_duplicate_when_complete_fails_after_send`.
3. **A production profile in a script without the production flag** (or with the wrong entity in it, or a non-TTY without `--yes`). Expected: refused before any broker call with a message naming `--confirm-prod <entity>`, nothing touched, an activity entry with outcome `denied`, exit code 1. Tests: Task 4 `denied_operation_touches_nothing_and_is_logged`, Task 18 `production_needs_yes_and_matching_confirm_prod`, `non_tty_without_yes_is_refused`.
4. **Session-enabled entities in purge, delete and move**, including sessions held by a running consumer and the emulator's empty session listing. Expected: purge visits each session (next-available until none, plus the listed ones), a locked session is skipped with `skipped: session locked by another receiver`, messages in the DLQ of a session entity need no session. Tests: Task 15 `purge_drains_every_session_and_skips_locked_ones`, Task 13 `active_match_runs_inside_the_message_session`.
5. **Ctrl-C or cancel in the middle of a job.** Expected: the job stops between messages, held locks are released, the report says `cancelled` with what was done so far, the activity log gets the finished entry with outcome `cancelled`, and the CLI can write the report. Tests: Task 13 `cancel_releases_held_locks_and_reports_not_reached`, Task 4 `cancelled_work_is_audited_as_cancelled`, Task 18 `ctrl_c_cancels_the_job_token`.

---

## File Structure

```
crates/dispatched-core/src/
├── lib.rs                     + pub mod activity, jobs, ops, safety, import; re-exports
├── error.rs                   + LockLost, Conflict, Denied; PolicyDenied → Denied
├── diagnostics.rs             + error kinds lock_lost, conflict, denied
├── paths.rs                   + ACTIVITY_DB, open_activity_log()
├── safety.rs                  NEW Mutation, OpKind, CountEstimate, ProfileRef, Actor, Operation, effect text, Gate
├── activity.rs                NEW ActivityLog (SQLite), ActivityEntry, Audit trait, MemoryAudit, queries, export
├── jobs.rs                    NEW JobKind, ItemOutcome, ItemResult, JobReport, JobEvent, Progress, report files
├── import.rs                  NEW read the M1c export format (JSON/NDJSON) into outgoing messages
├── explorer.rs                + ExplorerPlane (MessagingBackend over the cached connection), resolve_kind, dlq_summary
├── filter/expr.rs             user property names: exact, then case-insensitive
├── messaging/
│   ├── mod.rs                 + new modules and re-exports
│   ├── message.rs             time_to_live: Option<Duration>; PropValue::TimeSpan; decode timespan
│   ├── summary.rs             MessageSummary.time_to_live follows
│   ├── props.rs               NEW PropType, TypedValue, KEY=VALUE[:TYPE], durations
│   ├── outgoing.rs            NEW OutgoingMessage, OutgoingBody, MessageIdSource, Edits, CloneOptions, to_amqp
│   ├── template.rs            NEW BrokeredMessageTemplate JSON/XML reader
│   ├── management.rs          + call_for_link (associated-link-name); lock-lost/session-lock-lost mapping
│   ├── lockops.rs             NEW request/response for schedule, cancel, renew-lock, receive-by-seq, update-disposition, session state, renew-session-lock
│   ├── sender.rs              NEW AmqpSender (pipelined sends)
│   ├── receiver.rs            NEW ReceiveMode, ReceiverSpec, SessionChoice, LockToken, ReceivedMessage, Settlement, AmqpReceiver
│   ├── backend.rs             NEW pub(crate) MessagingBackend, MessageSender, LockReceiver traits
│   ├── fake.rs                NEW #[cfg(test)] FakeBroker
│   └── dlq.rs                 NEW DlqSummary, ReasonGroup, summarize
└── ops/
    ├── mod.rs                 NEW Operator (the only public way to mutate), MessageSelection, specs
    ├── send.rs                NEW send, schedule, send N copies, batch, cancel scheduled
    ├── receive.rs             NEW receive (peek-lock + settle, receive-and-delete), deferred settle
    ├── session_state.rs       NEW get / set / clear session state
    ├── locate.rs              NEW lock-and-match engine
    ├── remove.rs              NEW delete by sequence number
    ├── resubmit.rs            NEW resubmit, move, clone and resend
    ├── purge.rs               NEW purge active / DLQ / all, sessions, many entities
    └── import.rs              NEW replay imported messages
crates/dispatched-core/tests/emulator_m2a.rs   NEW emulator gate
crates/dispatched-core/Cargo.toml              + [[test]] emulator_m2a
crates/dispatched-cli/src/
├── main.rs                    + Send, Receive, Resubmit, Move, Delete, Purge, Dlq, Scheduled, Deferred, Import, Activity; session state; global --yes/--confirm-prod/--report
├── resolve.rs                 resolve() → Resolved { namespace, profile: ProfileRef }
├── safety.rs                  NEW CliPolicy, prompt, flags
├── jobs.rs                    NEW progress printer, report output, exit codes, ctrl-c
└── commands/
    ├── send.rs                NEW send / scheduled cancel / import
    ├── receive.rs             NEW receive / deferred
    ├── repair.rs              NEW resubmit / move / delete / purge / dlq summary
    ├── state.rs               NEW session state get|set|clear
    ├── activity.rs            NEW activity ls|export
    └── browse.rs              ls fallback shows resolved kinds
app/src-tauri/src/…            compile fixes only (TTL type)
testing/emulator/Config.json   + m2a-ops, m2a-target, m2a-sessions, m2a-purge-a, m2a-purge-b, topic m2a-topic/m2a-sub
.github/workflows/ci.yml       emulator job runs four gate tests
docs/                          spec §14, live-verification-m2a.md, README CLI section
```

### Task order and dependencies
1 → 2 → 3 → 4 (safety, activity, jobs, gate) · 5 → 6 → 7 (message model, composer, template/import; 5 is independent of 1–4) · 8 → 9 → 10 (AMQP plane; after 5–6) · 11 (traits, fake, ExplorerPlane; after 8–10) · 12 (Operator and simple ops; after 4 and 11) · 13 → 14 → 15 → 16 (jobs; after 12) · 17 carry-overs (any time after 5) · 18 → 19 → 20 → 21 CLI (after the core tasks each one uses) · 22 emulator gate (after 16) · 23 docs. Tasks 5–7 may run in parallel with 1–4 when their file sets are disjoint.

---

### Task 1: Core: new error kinds and the operation model

**Files:**
- Create: `crates/dispatched-core/src/safety.rs` (types and texts only; `Gate` arrives in Task 4)
- Modify: `crates/dispatched-core/src/error.rs`, `crates/dispatched-core/src/diagnostics.rs` (`error_kind`, `ERROR_KINDS` → 15 entries), `crates/dispatched-core/src/lib.rs`

**Interfaces:**
- Consumes: `horizon_azure_kit::safety::{ConfirmRequest, Origin}`, `crate::namespace::{EntityPath, SubQueue}`.
- Produces:
  - `Error::LockLost(String)` ("the lock on … expired or was lost; …"), `Error::Conflict(String)`, `Error::Denied(String)`; kinds `"lock_lost"`, `"conflict"`, `"denied"`; `From<KitError>` maps `PolicyDenied(m)` → `Error::Denied(m)`.
  - `pub enum Mutation { Send, Settle, Delete, Purge, EntityChange, SessionState }` (serde kebab-case), `pub enum OpKind { Read, Mutating(Mutation) }`.
  - `pub enum CountEstimate { Exact(u64), About(u64), AtLeast(u64), UpTo(u64), Unknown }` with `fn bound(&self) -> Option<u64>` (the largest number the op may touch: `Exact/About/UpTo(n)` → n, `AtLeast(n)` → n, `Unknown` → None) and `fn text(&self, noun: &str) -> String`.
  - `pub fn count_text(n: u64) -> String` (`1284` → `1,284`).
  - `pub struct ProfileRef { pub name: String, pub production: bool }`, `pub struct Actor { pub origin: Origin, pub profile: ProfileRef }`.
  - `pub struct Target { pub entity: EntityPath, pub sub_queue: Option<SubQueue> }` (`None` = the whole entity, for purge `all`) with `Target::new(EntityPath, SubQueue)`, `Target::all(EntityPath)`, `fn address_text(&self) -> String`; `pub enum SettleVerb { Complete, Abandon, Defer, DeadLetter }`.
  - `pub struct Operation { pub mutation: Mutation, pub target: Target, pub count: CountEstimate, pub reversible: bool, effect: Effect /* private: one variant per exact sentence below */ }` with the constructors listed in Step 3, `fn verb(&self) -> &'static str`, `fn summary(&self, profile: &ProfileRef) -> String`, `fn confirm_request(&self, actor: &Actor) -> ConfirmRequest`.

- [ ] **Step 1: Write the failing tests** (in `safety.rs` and `error.rs`)

```rust
fn prod() -> ProfileRef { ProfileRef { name: "prod-weu".into(), production: true } }
fn dlq(entity: &str) -> Target { Target::new(EntityPath::queue(entity), SubQueue::DeadLetter) }

#[test]
fn purge_text_matches_the_spec_example() {
    let op = Operation::purge(dlq("orders"), CountEstimate::Exact(1284));
    assert_eq!(op.summary(&prod()), "Remove 1,284 messages from orders/$DeadLetterQueue on prod-weu");
    assert_eq!(op.verb(), "purge");
    assert_eq!(op.mutation, Mutation::Purge);
}

#[test]
fn every_effect_names_count_entity_and_profile() {
    let p = prod();
    let q = || Target::new(EntityPath::queue("orders"), SubQueue::Active);
    let cases = [
        (Operation::send(q(), 3, None), "Send 3 messages to orders on prod-weu"),
        (Operation::send(q(), 1, Some(datetime!(2026-10-18 09:00 UTC))), "Schedule 1 message to orders for 2026-10-18T09:00:00Z on prod-weu"),
        (Operation::cancel_scheduled(q(), 4), "Cancel 4 scheduled messages in orders on prod-weu"),
        (Operation::receive_and_delete(q(), 10), "Remove up to 10 messages from orders on prod-weu (receive and delete)"),
        (Operation::receive_and_settle(q(), 10, SettleVerb::Complete), "Receive up to 10 messages from orders and complete them (removes them) on prod-weu"),
        (Operation::receive_and_settle(q(), 2, SettleVerb::DeadLetter), "Receive up to 2 messages from orders and dead-letter them on prod-weu"),
        (Operation::delete(dlq("orders"), &[4, 9, 12]), "Delete 3 messages (sequence numbers 4, 9, 12) from orders/$DeadLetterQueue on prod-weu"),
        (Operation::move_to(dlq("orders"), "orders", 3), "Move 3 messages from orders/$DeadLetterQueue to orders on prod-weu (sent first, then removed: a message may arrive twice)"),
        (Operation::resubmit(dlq("orders"), "orders", 3, false), "Resubmit 3 messages from orders/$DeadLetterQueue to orders on prod-weu"),
        (Operation::resubmit(dlq("orders"), "orders", 3, true), "Resubmit 3 messages from orders/$DeadLetterQueue to orders on prod-weu, removing the originals"),
        (Operation::set_session_state(q(), "RMA-1", true), "Replace the state of session RMA-1 in orders on prod-weu"),
        (Operation::set_session_state(q(), "RMA-1", false), "Clear the state of session RMA-1 in orders on prod-weu"),
    ];
    for (op, text) in cases { assert_eq!(op.summary(&p), text); }
}

#[test]
fn estimated_counts_say_so() {
    let t = || dlq("orders");
    assert_eq!(Operation::purge(t(), CountEstimate::About(40)).summary(&prod()), "Remove about 40 messages from orders/$DeadLetterQueue on prod-weu");
    assert_eq!(Operation::purge(t(), CountEstimate::AtLeast(10_000)).summary(&prod()),
        "Remove up to 10,000 messages from orders/$DeadLetterQueue on prod-weu (at least 10,000 are there; run purge again for the rest)");
    assert_eq!(Operation::delete(t(), &(1..=30).collect::<Vec<_>>()).summary(&prod()),
        "Delete 30 messages (sequence numbers 1, 2, 3, 4, 5, 6, 7, 8, 9, 10 and 20 more) from orders/$DeadLetterQueue on prod-weu");
}

#[test]
fn purge_all_mentions_scheduled_messages_are_kept() {
    let op = Operation::purge(Target::all(EntityPath::queue("orders")), CountEstimate::Exact(7));
    assert_eq!(op.summary(&prod()), "Remove 7 messages from orders (active, deferred and dead-lettered) on prod-weu. Scheduled messages are kept; cancel them with `scheduled cancel --all`");
}

#[test]
fn confirm_request_targets_the_entity_base_and_carries_production() {
    let actor = Actor { origin: Origin::Cli, profile: prod() };
    let op = Operation::purge(Target::new(EntityPath::subscription("events", "all"), SubQueue::DeadLetter), CountEstimate::Exact(2));
    let r = op.confirm_request(&actor);
    assert_eq!(r.target, "events/Subscriptions/all");
    assert_eq!(r.verb, "purge");
    assert_eq!(r.item_count, Some(2));
    assert!(r.production && !r.reversible && r.origin == Origin::Cli);
    assert!(r.summary.contains("events/Subscriptions/all/$DeadLetterQueue"));
}

#[test]
fn only_sends_are_reversible_in_confirmation_terms() {
    // A sent message can be received again; nothing else can be undone.
    let q = Target::new(EntityPath::queue("q"), SubQueue::Active);
    assert!(!Operation::send(q.clone(), 1, None).reversible);
    assert!(!Operation::delete(q, &[1]).reversible);
}
// error.rs
#[test]
fn policy_denial_is_denied_not_invalid() {
    let e: Error = KitError::PolicyDenied("no".into()).into();
    assert!(matches!(e, Error::Denied(ref m) if m == "no"));
    assert_eq!(crate::diagnostics::error_kind(&e), "denied");
}
```

(`address_text` is `orders/$DeadLetterQueue`, or the base for `None`. `SettleVerb` texts are `complete`, `abandon`, `defer`, `dead-letter`. `reversible` is `false` for every op: the field exists for the kit, and nothing a broker op does can be undone by Dispatched.)

- [ ] **Step 2: Run to verify they fail**

Run: `df -h /System/Volumes/Data && cargo test -p dispatched-core safety:: error::`
Expected: FAIL (module `safety` not found, `Error::Denied` missing).

- [ ] **Step 3: Implement**

`Operation` constructors: `send(Target, n: u64, schedule: Option<OffsetDateTime>)`, `cancel_scheduled(Target, n)`, `receive_and_delete(Target, n)`, `receive_and_settle(Target, n, SettleVerb)`, `settle_deferred(Target, n, SettleVerb)` (text `"{Verb} {n} deferred messages in {address} on {profile}"`, verb capitalised: Complete/Abandon/Defer/Dead-letter), `delete(Target, &[i64])`, `move_to(Target, target_name: &str, n)`, `resubmit(Target, target_name, n, remove_original: bool)`, `purge(Target, CountEstimate)`, `import(Target, n)` (text `"Send 12 imported messages to orders on prod-weu"`), `set_session_state(Target, session: &str, replace: bool)`. Mutations: send/schedule/import → `Send`; cancel scheduled, delete → `Delete`; receive-and-settle, settle-deferred → `Settle` (receive-and-delete → `Delete`); move/resubmit → `Send` when not removing, `Delete` when removing; purge → `Purge`; session state → `SessionState`. Plural rule: `1 message`, otherwise `messages`. Sequence lists show the first 10 then `and N more`. Timestamps RFC 3339 in UTC with `Z`. Add the three error variants with messages that say what to do (`LockLost`: "…; the message is available again, run the operation again", `Conflict` as given by callers, `Denied`: "denied: {m}"). `Error::LockLost` is **not** in `explorer::retry_after` (a healthy service answered).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p dispatched-core safety:: error:: diagnostics::`
Expected: PASS (the diagnostics test that compares `ERROR_KINDS` with `error_kind` also passes with 15 kinds).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/safety.rs crates/dispatched-core/src/error.rs crates/dispatched-core/src/diagnostics.rs crates/dispatched-core/src/lib.rs
git commit -m "Core: operation kinds, exact-effect confirmation texts, lock-lost/conflict/denied errors"
```

---

### Task 2: Core: the activity log in SQLite

**Files:**
- Create: `crates/dispatched-core/src/activity.rs`
- Modify: `crates/dispatched-core/src/paths.rs` (`ACTIVITY_DB = "activity.db"`, `open_activity_log()`), `crates/dispatched-core/src/lib.rs`, `crates/dispatched-core/src/profiles/store.rs` (`create_private_dir` becomes `pub(crate)`)

**Interfaces:**
- Consumes: Task 1 (`Mutation`, `OpKind`), `horizon_azure_kit::safety::Origin`.
- Produces:
  - `pub enum Phase { Started, Finished }`, `pub enum Outcome { Pending, Succeeded, PartlyFailed, Failed, Cancelled, Denied }` (kebab-case).
  - `pub struct ActivityEntry { pub id: i64 /* 0 before insert */, pub op_id: String, pub at: OffsetDateTime, pub origin: Origin, pub profile: String, pub entity: Option<String>, pub action: String /* verb */, pub kind: OpKind, pub phase: Phase, pub outcome: Outcome, pub count: Option<u64>, pub succeeded: Option<u64>, pub failed: Option<u64>, pub skipped: Option<u64>, pub summary: String, pub error: Option<String> }` (Serialize + Deserialize; `at` RFC 3339).
  - `pub trait Audit: Send + Sync { fn record(&self, entry: &ActivityEntry) -> Result<i64>; }`
  - `pub struct ActivityLog` with `open(path: &Path) -> Result<Self>`, `open_in_memory() -> Result<Self>`, `list(&self, q: &ActivityQuery) -> Result<Vec<ActivityEntry>>` (newest first), `export(&self, q: &ActivityQuery, path: &Path, format: ExportFormat) -> Result<u64>`, `impl Audit`.
  - `pub struct ActivityQuery { pub limit: u32 /* default 100, max 10,000 */, pub origin: Option<Origin>, pub profile: Option<String>, pub after_id: Option<i64>, pub op_id: Option<String> }` (Default).
  - `pub struct MemoryAudit` (always compiled; `entries() -> Vec<ActivityEntry>`, `fail_next()` makes the next `record` fail) for tests in core and in surfaces.
  - `paths::open_activity_log() -> Result<ActivityLog>`.

- [ ] **Step 1: Write the failing tests** (in `activity.rs`)

```rust
fn entry(op: &str, phase: Phase, outcome: Outcome) -> ActivityEntry { /* origin Cli, profile "dev", entity Some("orders"), action "purge", kind Mutating(Purge), summary "Remove 3 messages from orders on dev" */ }

#[test]
fn started_and_finished_entries_round_trip_newest_first() {
    let log = ActivityLog::open_in_memory().unwrap();
    let a = log.record(&entry("op-1", Phase::Started, Outcome::Pending)).unwrap();
    let mut fin = entry("op-1", Phase::Finished, Outcome::PartlyFailed);
    (fin.succeeded, fin.failed, fin.skipped) = (Some(2), Some(1), Some(0));
    let b = log.record(&fin).unwrap();
    assert!(b > a);
    let all = log.list(&ActivityQuery::default()).unwrap();
    assert_eq!(all.iter().map(|e| e.id).collect::<Vec<_>>(), [b, a]);
    assert_eq!(all[0].outcome, Outcome::PartlyFailed);
    assert_eq!(all[0].failed, Some(1));
    assert_eq!(all[1].phase, Phase::Started);
}

#[test]
fn queries_filter_by_origin_profile_op_and_after_id() {
    let log = ActivityLog::open_in_memory().unwrap();
    let mut gui = entry("op-g", Phase::Started, Outcome::Pending);
    gui.origin = Origin::Gui;
    let first = log.record(&entry("op-c", Phase::Started, Outcome::Pending)).unwrap();
    log.record(&gui).unwrap();
    assert_eq!(log.list(&ActivityQuery { origin: Some(Origin::Gui), ..Default::default() }).unwrap().len(), 1);
    assert_eq!(log.list(&ActivityQuery { op_id: Some("op-c".into()), ..Default::default() }).unwrap().len(), 1);
    assert_eq!(log.list(&ActivityQuery { after_id: Some(first), ..Default::default() }).unwrap().len(), 1);
    assert_eq!(log.list(&ActivityQuery { profile: Some("other".into()), ..Default::default() }).unwrap().len(), 0);
}

#[test]
fn two_handles_on_one_file_both_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity.db");
    let (a, b) = (ActivityLog::open(&path).unwrap(), ActivityLog::open(&path).unwrap());
    a.record(&entry("x", Phase::Started, Outcome::Pending)).unwrap();
    b.record(&entry("y", Phase::Started, Outcome::Pending)).unwrap();
    assert_eq!(a.list(&ActivityQuery::default()).unwrap().len(), 2);
}

#[test]
fn open_prunes_to_the_row_and_age_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity.db");
    {
        let log = ActivityLog::open_with_limits(&path, Limits { max_rows: 3, max_age: time::Duration::days(90) }).unwrap();
        let mut old = entry("old", Phase::Started, Outcome::Pending);
        old.at = OffsetDateTime::now_utc() - time::Duration::days(91);
        log.record(&old).unwrap();
        for i in 0..4 { log.record(&entry(&format!("n{i}"), Phase::Started, Outcome::Pending)).unwrap(); }
    }
    let log = ActivityLog::open_with_limits(&path, Limits { max_rows: 3, max_age: time::Duration::days(90) }).unwrap();
    let ops: Vec<String> = log.list(&ActivityQuery::default()).unwrap().into_iter().map(|e| e.op_id).collect();
    assert_eq!(ops, ["n3", "n2", "n1"]);
}

#[test]
fn export_writes_json_and_ndjson() {
    let log = ActivityLog::open_in_memory().unwrap();
    log.record(&entry("op", Phase::Started, Outcome::Pending)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let n = log.export(&ActivityQuery::default(), &dir.path().join("a.ndjson"), ExportFormat::Ndjson).unwrap();
    assert_eq!(n, 1);
    let line = std::fs::read_to_string(dir.path().join("a.ndjson")).unwrap();
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["origin"], "cli");
    assert_eq!(v["kind"], serde_json::json!({"mutating": "purge"}));
}

#[test]
fn a_newer_schema_is_refused_with_advice() {
    // user_version 99 written by a newer Dispatched: refuse, do not wipe.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity.db");
    rusqlite::Connection::open(&path).unwrap().execute_batch("PRAGMA user_version = 99").unwrap();
    let e = ActivityLog::open(&path).unwrap_err().to_string();
    assert!(e.contains("newer"), "{e}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p dispatched-core activity::`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement**

Schema v1 (one table): `activity(id INTEGER PRIMARY KEY AUTOINCREMENT, op_id TEXT NOT NULL, at TEXT NOT NULL /* RFC 3339 UTC */, origin TEXT NOT NULL, profile TEXT NOT NULL, entity TEXT, action TEXT NOT NULL, kind TEXT NOT NULL /* JSON of OpKind */, phase TEXT NOT NULL, outcome TEXT NOT NULL, count INTEGER, succeeded INTEGER, failed INTEGER, skipped INTEGER, summary TEXT NOT NULL, error TEXT)` with indexes on `(op_id)` and `(at)`. Open: create the parent dir private (reuse `profiles::store`'s `create_private_dir`, made `pub(crate)`), `journal_mode = WAL`, `busy_timeout = 5 s`, migrate under an immediate transaction (same pattern as `ProfileStore::migrate`), then prune (`DELETE … WHERE at < now − max_age`, then keep the newest `max_rows` by id). Defaults `Limits { max_rows: 50_000, max_age: 90 days }`. `OpKind` serializes as `"read"` or `{"mutating": "<mutation>"}`. Text columns go through `output`-style control-character stripping on write (`summary`, `error`, `entity`, `profile`): replace control characters with a space, so a hostile entity name cannot corrupt a terminal later.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p dispatched-core activity:: paths::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/activity.rs crates/dispatched-core/src/paths.rs crates/dispatched-core/src/lib.rs crates/dispatched-core/src/profiles/store.rs
git commit -m "Core: activity log in SQLite (activity.db), queries, export, pruning"
```

---

### Task 3: Core: job results, progress and report files

**Files:**
- Create: `crates/dispatched-core/src/jobs.rs`
- Modify: `crates/dispatched-core/src/lib.rs`

**Interfaces:**
- Consumes: `crate::export::ExportFormat`, `crate::export::FileExport` (atomic writes), Task 2 `Outcome`.
- Produces:
  - `pub enum JobKind { Send, Import, CancelScheduled, Receive, SettleDeferred, Delete, Move, Resubmit, Purge }` (kebab-case).
  - `pub enum NotReached { Timeout, Budget, HeldLimit, Cancelled }` (kebab-case).
  - `pub enum ItemOutcome { Succeeded { detail: Option<String> }, Failed { kind: &'static str /* error_kind */, reason: String }, Skipped { reason: String }, Missing, NotReached { why: NotReached } }` (serde tag `"status"`, kebab-case).
  - `pub struct ItemResult { pub sequence_number: Option<i64>, pub message_id: Option<String>, pub session_id: Option<String>, pub entity: String, pub new_sequence_number: Option<i64>, pub outcome: ItemOutcome }`.
  - `pub struct JobReport { pub kind: JobKind, pub entity: String, pub target: Option<String>, pub started_at, pub finished_at, pub cancelled: bool, pub items: Vec<ItemResult>, pub notes: Vec<String> }` with `succeeded() -> u64`, `failed() -> u64` (Failed + NotReached), `skipped() -> u64` (Skipped + Missing), `outcome() -> Outcome` (Cancelled if cancelled; Succeeded if failed()==0; Failed if succeeded()==0 and failed()>0; else PartlyFailed), `failed_sequences() -> Vec<i64>`, `summary_line() -> String`, `write(&self, path: &Path, format: ExportFormat, overwrite: bool) -> Result<()>`.
  - JSON shape adds the spec §8 summary: `{"kind", "entity", "target", "started_at", "finished_at", "cancelled", "succeeded", "failed": [{"seq", "reason"}], "skipped", "items": [...], "notes": [...]}`.
  - `pub enum JobEvent { Started { kind: JobKind, total: Option<u64> }, Item(ItemResult), Progress { done: u64, total: Option<u64> }, Note(String) }`; `pub trait Progress: Send + Sync { fn event(&self, e: &JobEvent); }`; `pub struct NoProgress;`; `pub struct RecordingProgress` (test helper, `events()`).
  - `pub(crate) struct JobRun` — the builder ops use: `new(kind, entity, target, progress, total)`, `item(ItemResult)` (pushes and emits `Item` + `Progress`), `note(String)`, `finish(cancelled: bool) -> JobReport`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn report_counts_and_outcome() {
    let mut r = report(&[ok(1), failed(2, "lock_lost", "the lock expired"), skipped(3), missing(4), not_reached(5, NotReached::Timeout)]);
    assert_eq!((r.succeeded(), r.failed(), r.skipped()), (1, 2, 2));
    assert_eq!(r.outcome(), Outcome::PartlyFailed);
    assert_eq!(r.failed_sequences(), [2, 5]);
    r.cancelled = true;
    assert_eq!(r.outcome(), Outcome::Cancelled);
    assert_eq!(report(&[]).outcome(), Outcome::Succeeded, "nothing to do is success");
    assert_eq!(report(&[failed(1, "timeout", "x")]).outcome(), Outcome::Failed);
}

#[test]
fn json_has_the_spec_summary_shape() {
    let r = report(&[ok(1), failed(2, "lock_lost", "the lock expired")]);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["succeeded"], 1);
    assert_eq!(v["failed"], serde_json::json!([{"seq": 2, "reason": "the lock expired"}]));
    assert_eq!(v["skipped"], 0);
    assert_eq!(v["items"][1]["outcome"]["status"], "failed");
    assert_eq!(v["items"][1]["outcome"]["kind"], "lock_lost");
}

#[test]
fn summary_line_reads_like_a_sentence() {
    assert_eq!(report_kind(JobKind::Move, &[ok(1), ok(2), failed(3, "lock_lost", "the lock expired")]).summary_line(),
        "Moved 2 of 3 messages from orders/$DeadLetterQueue to orders; 1 failed (sequence 3: the lock expired)");
    assert_eq!(report_kind(JobKind::Purge, &[ok(1)]).summary_line(), "Removed 1 message from orders/$DeadLetterQueue");
}

#[test]
fn report_writes_json_and_ndjson_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let r = report(&[ok(1), ok(2)]);
    r.write(&dir.path().join("r.json"), ExportFormat::Json, false).unwrap();
    assert!(r.write(&dir.path().join("r.json"), ExportFormat::Json, false).is_err(), "no silent overwrite");
    r.write(&dir.path().join("r.ndjson"), ExportFormat::Ndjson, false).unwrap();
    // NDJSON: one line per item, then one summary line with "type": "summary".
    let text = std::fs::read_to_string(dir.path().join("r.ndjson")).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(text.lines().last().unwrap().contains("\"type\":\"summary\""));
}

#[test]
fn job_run_emits_item_and_progress_events() {
    let p = RecordingProgress::default();
    let mut run = JobRun::new(JobKind::Delete, "orders".into(), None, &p, Some(2));
    run.item(ok(1));
    let r = run.finish(false);
    assert_eq!(r.items.len(), 1);
    assert!(matches!(p.events()[0], JobEvent::Started { total: Some(2), .. }));
    assert!(matches!(p.events().last().unwrap(), JobEvent::Progress { done: 1, total: Some(2) }));
}
```

(`ok`, `failed`, `skipped`, `missing`, `not_reached`, `report`, `report_kind` are test helpers in the module; `report*` use entity `orders/$DeadLetterQueue` and target `orders`.)

Summary verbs per kind: Send "Sent", Import "Sent", CancelScheduled "Cancelled", Receive "Received", SettleDeferred "Settled", Delete "Deleted", Move "Moved", Resubmit "Resubmitted", Purge "Removed". Failure list shows the first 5 `sequence N: reason`, then `and N more`.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core jobs::` → FAIL.
- [ ] **Step 3: Implement** the types above; `write` reuses `FileExport`'s temp-file-and-rename (add `FileExport::write_raw(&mut self, bytes)` if needed rather than duplicating the atomic logic).
- [ ] **Step 4: Run** — `cargo test -p dispatched-core jobs:: export::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/jobs.rs crates/dispatched-core/src/export.rs crates/dispatched-core/src/lib.rs
git commit -m "Core: job reports with per-message results, progress events, report files"
```

---

### Task 4: Core: the gate (confirm, audit before, audit after)

**Files:**
- Modify: `crates/dispatched-core/src/safety.rs`

**Interfaces:**
- Consumes: Tasks 1–3; `horizon_azure_kit::safety::{Policy, require}`.
- Produces:
  - `pub struct Gate { policy: Arc<dyn Policy>, audit: Arc<dyn Audit>, actor: Actor }` with `pub fn new(policy, audit, actor) -> Self`, `pub fn actor(&self) -> &Actor`.
  - `pub trait Audited { fn audit_outcome(&self) -> AuditFacts; }` with `pub struct AuditFacts { pub outcome: Outcome, pub succeeded: Option<u64>, pub failed: Option<u64>, pub skipped: Option<u64>, pub note: Option<String> }`; implemented for `JobReport`, `()` (Succeeded) and, in later tasks, `ReceiveOutcome`.
  - `pub async fn Gate::run<T: Audited, F, Fut>(&self, op: &Operation, work: F) -> Result<T> where F: FnOnce() -> Fut, Fut: Future<Output = Result<T>>`:
    1. `require(policy, &op.confirm_request(actor))`; on denial: record one `Finished` entry with `Outcome::Denied` and the reason in `error`, return `Error::Denied`. `work` is never called.
    2. record `Started` (`Outcome::Pending`, `count = op.count.bound()`); **if this write fails, return `Error::Io("cannot write the activity log ({e}); nothing was changed")` without calling `work`** (the spec's "before it starts" is a precondition).
    3. run `work`; record `Finished` with the facts (`Ok`) or `Outcome::Failed`/`Cancelled` with the error text (`Err`; `Error::Cancelled` → `Cancelled`). A failed finish write is logged with `tracing::warn!` and does not change the result.
    4. Both entries share a fresh `op_id` (`op-{uuid simple}`).

- [ ] **Step 1: Write the failing tests**

```rust
struct Scripted(Decision);
#[async_trait] impl Policy for Scripted { async fn confirm(&self, _: &ConfirmRequest) -> Decision { self.0.clone() } }

fn gate(decision: Decision, production: bool) -> (Gate, Arc<MemoryAudit>) { /* actor Cli, profile "prod-weu" */ }
fn purge_op() -> Operation { Operation::purge(Target::new(EntityPath::queue("orders"), SubQueue::DeadLetter), CountEstimate::Exact(3)) }

#[tokio::test]
async fn allowed_work_is_audited_before_and_after() {
    let (g, audit) = gate(Decision::Allow, false);
    let calls = AtomicUsize::new(0);
    g.run(&purge_op(), || async { calls.fetch_add(1, SeqCst); assert_eq!(audit.entries().len(), 1, "started is written first"); Ok(()) }).await.unwrap();
    let e = audit.entries();
    assert_eq!(calls.load(SeqCst), 1);
    assert_eq!((e[0].phase, e[0].outcome), (Phase::Started, Outcome::Pending));
    assert_eq!((e[1].phase, e[1].outcome), (Phase::Finished, Outcome::Succeeded));
    assert_eq!(e[0].op_id, e[1].op_id);
    assert_eq!(e[0].summary, "Remove 3 messages from orders/$DeadLetterQueue on prod-weu");
    assert_eq!(e[0].count, Some(3));
}

#[tokio::test]
async fn denied_operation_touches_nothing_and_is_logged() {
    let (g, audit) = gate(Decision::Deny("not now".into()), false);
    let e = g.run(&purge_op(), || async { panic!("work must not run") ; #[allow(unreachable_code)] Ok(()) }).await.unwrap_err();
    assert!(matches!(e, Error::Denied(_)));
    let log = audit.entries();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].phase, log[0].outcome), (Phase::Finished, Outcome::Denied));
    assert!(log[0].error.as_deref().unwrap().contains("not now"));
}

#[tokio::test]
async fn production_needs_the_typed_entity() {
    let (g, _) = gate(Decision::Allow, true);
    assert!(matches!(g.run(&purge_op(), || async { Ok(()) }).await, Err(Error::Denied(_))));
    let (g, _) = gate(Decision::ConfirmedTyped("orders".into()), true);
    g.run(&purge_op(), || async { Ok(()) }).await.unwrap();
}

#[tokio::test]
async fn failing_audit_start_stops_the_operation() {
    let (g, audit) = gate(Decision::Allow, false);
    audit.fail_next();
    let e = g.run(&purge_op(), || async { panic!("must not run"); #[allow(unreachable_code)] Ok(()) }).await.unwrap_err();
    assert!(e.to_string().contains("nothing was changed"));
}

#[tokio::test]
async fn cancelled_work_is_audited_as_cancelled() {
    let (g, audit) = gate(Decision::Allow, false);
    let _ = g.run::<(), _, _>(&purge_op(), || async { Err(Error::Cancelled) }).await;
    assert_eq!(audit.entries()[1].outcome, Outcome::Cancelled);
}

#[tokio::test]
async fn job_report_facts_reach_the_finished_entry() {
    let (g, audit) = gate(Decision::Allow, false);
    g.run(&purge_op(), || async { Ok(report_with(2, 1, 0)) }).await.unwrap();
    let fin = &audit.entries()[1];
    assert_eq!((fin.outcome, fin.succeeded, fin.failed, fin.skipped), (Outcome::PartlyFailed, Some(2), Some(1), Some(0)));
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core safety::` → FAIL (`Gate` missing).
- [ ] **Step 3: Implement** `Gate` and `Audited` as specified.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core safety:: jobs:: activity::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/safety.rs
git commit -m "Core: safety gate confirms, then audits before and after every mutating op"
```

---

### Task 5: Core: TTL as a duration and typed property values

**Files:**
- Create: `crates/dispatched-core/src/messaging/props.rs`
- Modify: `crates/dispatched-core/src/messaging/message.rs`, `crates/dispatched-core/src/messaging/summary.rs`, `crates/dispatched-core/src/filter/expr.rs` (match arm for `PropValue::TimeSpan`; `sys.TimeToLive` if present uses the duration), `crates/dispatched-core/src/messaging/mod.rs`, `crates/dispatched-core/src/lib.rs`, every compile error in `crates/dispatched-cli` and `app/src-tauri` (field rename only)

**Interfaces:**
- Produces:
  - `PeekedMessage.time_to_live: Option<std::time::Duration>` and `MessageSummary.time_to_live` — serialized as `"time_to_live_ms"` (u64 milliseconds) via `#[serde(rename = "time_to_live_ms", with = "ttl_ms")]` (a small serde module in `message.rs`, also usable for `Deserialize` in Task 7).
  - Decoding: header `ttl` if present; else `absolute_expiry_time − creation_time` when both are present and the difference is positive; else `None`. `expires_at` keeps its M1c rule but uses the duration.
  - `PropValue::TimeSpan(time::Duration)` serialized `{"type":"timespan","value":"PT1H30M"}` (ISO 8601 duration text); decoded from `SimpleValue::Described` with descriptor name `com.microsoft:timespan` and a `Long` value of 100 ns ticks. Other described values stay `Other`.
  - `pub enum PropType { String, Int, Long, Double, Bool, DateTime, TimeSpan, Guid }` with `parse(&str)` accepting short names (`string|int|long|double|bool|datetime|timespan|guid`) and .NET names (`String|Int32|Int64|Double|Boolean|DateTime|TimeSpan|Guid`), case-insensitive.
  - `pub enum TypedValue { String(String), Int(i32), Long(i64), Double(f64), Bool(bool), DateTime(OffsetDateTime), TimeSpan(time::Duration), Guid(uuid::Uuid) }` with `parse(ty: PropType, text: &str) -> Result<Self>`, `to_simple(&self) -> SimpleValue`, `from_prop(&PropValue) -> Option<TypedValue>` (for cloning: `Int` fits i32 → `Int` else `Long`; `UInt` ≤ i64::MAX → `Long`; `Uuid` → `Guid`; `Timestamp` → `DateTime`; `Binary`/`Other`/`Null` → None, reported by the caller).
  - `pub fn parse_prop_arg(arg: &str) -> Result<(String, TypedValue)>` for `KEY=VALUE[:TYPE]` (split on the first `=`, then on the last `:` only when the suffix parses as a `PropType`).
  - `pub fn parse_duration(text: &str) -> Result<std::time::Duration>`: `90s`, `15m`, `2h`, `7d`, `1h30m`, ISO 8601 `PT1H30M`/`P7D`, .NET `[d.]hh:mm:ss[.fffffff]`. Zero and negative are refused.
  - `pub fn parse_when(text: &str, now: OffsetDateTime) -> Result<OffsetDateTime>`: RFC 3339, or `+DURATION` relative to `now`.

- [ ] **Step 1: Write the failing tests**

```rust
// message.rs
#[test]
fn ttl_comes_from_the_header_or_from_expiry_minus_creation() {
    let created = Timestamp::from_milliseconds(1_000);
    let m = amqp_with(|m| { m.header = Some(Header { ttl: Some(60_000), ..Default::default() }); });
    assert_eq!(decode_message(m).time_to_live, Some(Duration::from_secs(60)));
    // 100 days does not fit the u32 header: it survives through absolute-expiry-time.
    let hundred_days_ms = 100 * 86_400_000i64;
    let m = amqp_with(|m| { m.properties = Some(Properties { creation_time: Some(created.clone()),
        absolute_expiry_time: Some(Timestamp::from_milliseconds(1_000 + hundred_days_ms)), ..Default::default() }); });
    assert_eq!(decode_message(m).time_to_live, Some(Duration::from_millis(hundred_days_ms as u64)));
    assert_eq!(decode_message(amqp_with(|_| {})).time_to_live, None);
}

#[test]
fn ttl_serializes_as_milliseconds_under_the_old_name() {
    let mut m = test_message(1);
    m.time_to_live = Some(Duration::from_millis(100 * 86_400_000));
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v["time_to_live_ms"], 8_640_000_000u64);
}

#[test]
fn timespan_properties_decode_from_dotnet_ticks() {
    let described = SimpleValue::Described(Box::new(Described { descriptor: Descriptor::Name(Symbol::from("com.microsoft:timespan")), value: Value::Long(36_000_000_000) }));
    let m = amqp_with(|m| m.application_properties = Some(ApplicationProperties(OrderedMap::from_iter([("wait".to_string(), described)]))));
    let d = decode_message(m);
    assert_eq!(d.application_properties["wait"], PropValue::TimeSpan(time::Duration::hours(1)));
    assert_eq!(serde_json::to_value(&d.application_properties["wait"]).unwrap(), serde_json::json!({"type": "timespan", "value": "PT1H"}));
}
// props.rs
#[test]
fn prop_args_parse_with_types_and_defaults() {
    assert_eq!(parse_prop_arg("kind=x").unwrap(), ("kind".into(), TypedValue::String("x".into())));
    assert_eq!(parse_prop_arg("n=42:int").unwrap().1, TypedValue::Int(42));
    assert_eq!(parse_prop_arg("n=42:Int64").unwrap().1, TypedValue::Long(42));
    assert_eq!(parse_prop_arg("url=http://x:string").unwrap().1, TypedValue::String("http://x".into()));
    assert_eq!(parse_prop_arg("at=10:30").unwrap().1, TypedValue::String("10:30".into()), "`30` is not a type");
    assert_eq!(parse_prop_arg("w=01:30:00:timespan").unwrap().1, TypedValue::TimeSpan(time::Duration::minutes(90)));
    assert_eq!(parse_prop_arg("ok=TRUE:bool").unwrap().1, TypedValue::Bool(true));
    assert!(matches!(parse_prop_arg("g=nope:guid"), Err(Error::Invalid(m)) if m.contains("guid")));
    assert!(parse_prop_arg("=1").is_err() && parse_prop_arg("novalue").is_err());
    assert!(matches!(parse_prop_arg("n=3000000000:int"), Err(Error::Invalid(m)) if m.contains("long")), "suggests long");
}

#[test]
fn typed_values_encode_as_the_dotnet_sdk_does() {
    let SimpleValue::Described(d) = TypedValue::TimeSpan(time::Duration::seconds(1)).to_simple() else { panic!() };
    assert_eq!(d.descriptor, Descriptor::Name(Symbol::from("com.microsoft:timespan")));
    assert_eq!(d.value, Value::Long(10_000_000));
    assert!(matches!(TypedValue::Guid(uuid::Uuid::nil()).to_simple(), SimpleValue::Uuid(_)));
    assert!(matches!(TypedValue::DateTime(datetime!(2026-01-01 0:00 UTC)).to_simple(), SimpleValue::Timestamp(_)));
}

#[test]
fn durations_and_times_parse_in_every_documented_form() {
    for (t, s) in [("90s", 90), ("15m", 900), ("2h", 7200), ("7d", 604_800), ("1h30m", 5400), ("PT1H30M", 5400), ("P7D", 604_800), ("1.02:03:04", 93_784), ("00:00:01.5000000", 1)] {
        assert_eq!(parse_duration(t).unwrap().as_secs(), s, "{t}");
    }
    assert!(parse_duration("0s").is_err() && parse_duration("-1m").is_err() && parse_duration("soon").is_err());
    let now = datetime!(2026-10-17 12:00 UTC);
    assert_eq!(parse_when("+10m", now).unwrap(), datetime!(2026-10-17 12:10 UTC));
    assert_eq!(parse_when("2026-10-18T09:00:00Z", now).unwrap(), datetime!(2026-10-18 9:00 UTC));
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::message messaging::props` → FAIL.
- [ ] **Step 3: Implement** as specified; then fix compile errors in the CLI (`commands/browse.rs` table/JSON if they read `time_to_live_ms`) and the app crate (`app/src-tauri/src/ops/browse.rs` or wherever it is read) by using `time_to_live` — no TypeScript change (the JSON name is unchanged).
- [ ] **Step 4: Run**

Run: `df -h /System/Volumes/Data && cargo test -p dispatched-core && cargo test -p dispatched && cargo check -p dispatched-app`
Expected: PASS; the app crate checks.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src crates/dispatched-cli/src app/src-tauri/src
git commit -m "Core: TTL is a duration (header or expiry minus creation), typed property values, timespan decoding"
```

---

### Task 6: Core: the outgoing message (composer core and clone rules)

**Files:**
- Create: `crates/dispatched-core/src/messaging/outgoing.rs`
- Modify: `crates/dispatched-core/src/messaging/mod.rs`, `crates/dispatched-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 5 (`TypedValue`, `PeekedMessage.time_to_live`), `MessageBody`.
- Produces:
  - `pub enum OutgoingBody { Bytes(Vec<u8>), AmqpString(String), AmqpJson(serde_json::Value) /* re-encoded */, Empty }`.
  - `pub struct OutgoingMessage { pub body: OutgoingBody, pub message_id: Option<String>, pub session_id: Option<String>, pub correlation_id: Option<String>, pub subject: Option<String>, pub to: Option<String>, pub reply_to: Option<String>, pub reply_to_session_id: Option<String>, pub content_type: Option<String>, pub partition_key: Option<String>, pub time_to_live: Option<std::time::Duration>, pub scheduled_enqueue_time: Option<OffsetDateTime>, pub properties: BTreeMap<String, TypedValue> }` (Default, Clone, Debug, PartialEq).
  - `impl OutgoingMessage { pub fn text(body: &str) -> Self; pub fn validate(&self) -> Result<()>; pub(crate) fn to_amqp(&self, now: OffsetDateTime) -> Result<Message<Body<Value>>>; pub fn from_peeked(m: &PeekedMessage, opts: &CloneOptions) -> Cloned }`.
  - `pub struct CloneOptions { pub new_message_id: bool, pub keep_dead_letter_properties: bool, pub from_dead_letter: bool }`; `pub struct Cloned { pub message: OutgoingMessage, pub notes: Vec<String> /* "body re-encoded from JSON", "property X (binary) was not copied" */ }`.
  - `pub struct Edits { pub body: Option<OutgoingBody>, pub set: BTreeMap<String, TypedValue>, pub remove: Vec<String>, pub message_id: Option<String>, pub session_id: Option<Option<String>>, pub correlation_id: Option<Option<String>>, pub subject: Option<Option<String>>, pub content_type: Option<Option<String>>, pub to: Option<Option<String>>, pub reply_to: Option<Option<String>>, pub time_to_live: Option<Option<Duration>> }` with `fn apply(&self, m: &mut OutgoingMessage)` (`Some(None)` clears a field).
  - `pub enum MessageIdSource { Keep, Fixed(String), Random, Increment { base: String } }` with `fn id_for(&self, index: u64) -> Option<String>` (`Keep` → None meaning "leave as is").
  - `pub enum BodySource { Text(String), Files(Vec<PathBuf>) }` with `fn body_for(&self, index: u64) -> Result<OutgoingBody>` (files round-robin, read on use, `Bytes`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn amqp_encoding_sets_every_system_property() {
    let now = datetime!(2026-10-17 12:00 UTC);
    let m = OutgoingMessage { message_id: Some("m1".into()), session_id: Some("s".into()), correlation_id: Some("c".into()),
        subject: Some("sub".into()), to: Some("to".into()), reply_to: Some("rt".into()), reply_to_session_id: Some("rs".into()),
        content_type: Some("application/json".into()), partition_key: Some("s".into()),
        time_to_live: Some(Duration::from_secs(60)), scheduled_enqueue_time: Some(now + time::Duration::hours(1)),
        properties: BTreeMap::from([("n".into(), TypedValue::Int(1))]), ..OutgoingMessage::text("{}") };
    let a = m.to_amqp(now).unwrap();
    let p = a.properties.as_ref().unwrap();
    assert_eq!(p.message_id, Some(MessageId::String("m1".into())));
    assert_eq!(p.group_id.as_deref(), Some("s"));
    assert_eq!(p.reply_to_group_id.as_deref(), Some("rs"));
    assert_eq!(p.content_type.as_ref().map(|s| s.0.as_str()), Some("application/json"));
    assert_eq!(a.header.as_ref().unwrap().ttl, Some(60_000));
    assert_eq!(p.absolute_expiry_time.as_ref().unwrap().milliseconds() - p.creation_time.as_ref().unwrap().milliseconds(), 60_000);
    let ann = a.message_annotations.as_ref().unwrap();
    assert!(ann.get(&"x-opt-scheduled-enqueue-time" as &dyn AnnotationKey).is_some());
    assert!(ann.get(&"x-opt-partition-key" as &dyn AnnotationKey).is_some());
    assert!(matches!(a.body, Body::Data(_)));
}

#[test]
fn a_ttl_longer_than_the_header_caps_the_header_but_keeps_the_expiry() {
    let now = datetime!(2026-10-17 12:00 UTC);
    let m = OutgoingMessage { time_to_live: Some(Duration::from_secs(100 * 86_400)), ..OutgoingMessage::text("x") };
    let a = m.to_amqp(now).unwrap();
    assert_eq!(a.header.unwrap().ttl, Some(u32::MAX));
    let p = a.properties.unwrap();
    assert_eq!(p.absolute_expiry_time.unwrap().milliseconds() - p.creation_time.unwrap().milliseconds(), 100 * 86_400_000);
}

#[test]
fn validation_names_the_broker_limits() {
    let long = "x".repeat(129);
    for (m, needle) in [
        (OutgoingMessage { message_id: Some(long.clone()), ..OutgoingMessage::text("x") }, "MessageId"),
        (OutgoingMessage { session_id: Some(long.clone()), ..OutgoingMessage::text("x") }, "SessionId"),
        (OutgoingMessage { partition_key: Some(long), ..OutgoingMessage::text("x") }, "PartitionKey"),
        (OutgoingMessage { session_id: Some("a".into()), partition_key: Some("b".into()), ..OutgoingMessage::text("x") }, "PartitionKey must equal SessionId"),
    ] {
        assert!(matches!(m.validate(), Err(Error::Invalid(e)) if e.contains(needle)), "{needle}");
    }
}

#[test]
fn clone_keeps_user_properties_and_drops_broker_values() {
    let mut src = test_message(42);
    src.body = MessageBody::Binary(b"{\"a\":1}".to_vec());
    src.message_id = Some("orig".into());
    src.session_id = Some("s".into());
    src.time_to_live = Some(Duration::from_secs(30));
    src.scheduled_enqueue_time = Some(datetime!(2030-01-01 0:00 UTC));
    src.application_properties.insert("DeadLetterReason".into(), PropValue::String("x".into()));
    src.application_properties.insert("kind".into(), PropValue::String("order".into()));
    src.application_properties.insert("blob".into(), PropValue::Binary(vec![1]));
    let c = OutgoingMessage::from_peeked(&src, &CloneOptions { from_dead_letter: true, ..Default::default() });
    assert_eq!(c.message.body, OutgoingBody::Bytes(b"{\"a\":1}".to_vec()));
    assert_eq!(c.message.message_id.as_deref(), Some("orig"));
    assert_eq!(c.message.session_id.as_deref(), Some("s"));
    assert_eq!(c.message.time_to_live, Some(Duration::from_secs(30)));
    assert_eq!(c.message.scheduled_enqueue_time, None);
    assert!(!c.message.properties.contains_key("DeadLetterReason"));
    assert_eq!(c.message.properties["kind"], TypedValue::String("order".into()));
    assert!(c.notes.iter().any(|n| n == "property blob (binary) was not copied"));
    let fresh = OutgoingMessage::from_peeked(&src, &CloneOptions { new_message_id: true, ..Default::default() });
    assert_ne!(fresh.message.message_id.as_deref(), Some("orig"));
}

#[test]
fn json_value_bodies_are_re_encoded_and_say_so() {
    let mut src = test_message(1);
    src.body = MessageBody::Value(serde_json::json!({"a": [1, 2]}));
    let c = OutgoingMessage::from_peeked(&src, &CloneOptions::default());
    assert!(matches!(c.message.body, OutgoingBody::AmqpJson(_)));
    assert_eq!(c.notes, ["body re-encoded from JSON"]);
}

#[test]
fn edits_set_clear_and_remove() {
    let mut m = OutgoingMessage { subject: Some("a".into()), properties: BTreeMap::from([("x".into(), TypedValue::Int(1))]), ..OutgoingMessage::text("b") };
    Edits { subject: Some(None), remove: vec!["x".into()], set: BTreeMap::from([("y".into(), TypedValue::Bool(true))]),
        body: Some(OutgoingBody::Bytes(b"new".to_vec())), ..Default::default() }.apply(&mut m);
    assert_eq!(m.subject, None);
    assert!(!m.properties.contains_key("x") && m.properties.contains_key("y"));
    assert_eq!(m.body, OutgoingBody::Bytes(b"new".to_vec()));
}

#[test]
fn message_ids_increment_keeping_padding() {
    let inc = MessageIdSource::Increment { base: "order-0009".into() };
    assert_eq!([inc.id_for(0), inc.id_for(1)], [Some("order-0009".into()), Some("order-0010".into())]);
    let plain = MessageIdSource::Increment { base: "order".into() };
    assert_eq!([plain.id_for(0), plain.id_for(1)], [Some("order".into()), Some("order-1".into())]);
    assert_eq!(MessageIdSource::Increment { base: "99".into() }.id_for(1), Some("100".into()));
    assert_eq!(MessageIdSource::Keep.id_for(3), None);
}

#[test]
fn files_are_used_round_robin() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&a, "A").unwrap(); std::fs::write(&b, "B").unwrap();
    let s = BodySource::Files(vec![a, b]);
    let bodies: Vec<_> = (0..3).map(|i| s.body_for(i).unwrap()).collect();
    assert_eq!(bodies, [OutgoingBody::Bytes(b"A".to_vec()), OutgoingBody::Bytes(b"B".to_vec()), OutgoingBody::Bytes(b"A".to_vec())]);
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::outgoing` → FAIL.
- [ ] **Step 3: Implement.** Limits: MessageId, SessionId, PartitionKey, ReplyToSessionId ≤ 128 characters; empty strings are refused ("leave it out instead"). `AmqpJson` re-encodes JSON → AMQP (`null`→Null, bool, integers → Long, other numbers → Double, string, array → List, object → Map with String keys). `to_amqp` puts the body in one Data section for `Bytes`, `Value(String)` for `AmqpString`, `Value(json→amqp)` for `AmqpJson`, `Body::Empty`… use an empty Data section for `Empty` (the broker refuses a message with no body section).
- [ ] **Step 4: Run** — `cargo test -p dispatched-core messaging::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/messaging/outgoing.rs crates/dispatched-core/src/messaging/mod.rs crates/dispatched-core/src/lib.rs
git commit -m "Core: outgoing messages with every system property, clone rules, edits, auto-increment ids"
```

---

### Task 7: Core: BrokeredMessageTemplate files and importing exported messages

**Files:**
- Create: `crates/dispatched-core/src/messaging/template.rs`, `crates/dispatched-core/src/import.rs`, `crates/dispatched-core/tests/fixtures/template.json`, `crates/dispatched-core/tests/fixtures/template.xml`
- Modify: `crates/dispatched-core/src/messaging/mod.rs`, `crates/dispatched-core/src/lib.rs`

**Interfaces:**
- Consumes: Task 5 (`PropType`, `TypedValue`, `parse_duration`), Task 6 (`OutgoingMessage`, `OutgoingBody`), `export::EXPORT_SCHEMA`.
- Produces:
  - `pub fn read_template(path: &Path) -> Result<OutgoingMessage>` (JSON when the first non-space byte is `{`, XML when `<`), plus `parse_template_json(&str)` and `parse_template_xml(&str)`. Body: UTF-8 text → `OutgoingBody::Bytes`. `label` → `subject`. Empty strings are treated as absent. `scheduledEnqueueTimeUtc` accepts RFC 3339 or `yyyy-MM-ddTHH:mm:ss` (UTC).
  - `pub struct ImportedMessage { pub source_sequence: Option<i64>, pub message: OutgoingMessage, pub notes: Vec<String> }`.
  - `pub fn read_messages(path: &Path) -> Result<Vec<ImportedMessage>>`: a JSON array or NDJSON of `dispatched.message/v1` objects (the M1c export format). Unknown `schema` values are refused with the line/index. Body: `{"kind":"binary","value":base64}` → `Bytes`, `text` → `AmqpString`, `value`/`sequence` → `AmqpJson` (note "body re-encoded from JSON"), `empty` → `Empty`. Properties: `{"type","value"}` → `TypedValue` via the same rules as `TypedValue::from_prop` (binary and `other` dropped with a note). `time_to_live_ms` → `time_to_live`. Broker-owned fields are ignored. At most 100,000 messages per file and 256 MiB (refused with a clear error).

- [ ] **Step 1: Write the failing tests**

```rust
// template.rs
#[test]
fn json_template_maps_every_field() {
    let t = parse_template_json(include_str!("../../tests/fixtures/template.json")).unwrap();
    assert_eq!(t.message_id.as_deref(), Some("MyMessageId"));
    assert_eq!(t.subject.as_deref(), Some("MyLabel"));
    assert_eq!(t.time_to_live, Some(Duration::from_secs(3600)));
    assert_eq!(t.properties["Country"], TypedValue::String("Italy".into()));
    assert_eq!(t.properties["Quantity"], TypedValue::Int(5));
    assert_eq!(t.body, OutgoingBody::Bytes(b"{\"orderId\":1}".to_vec()));
}

#[test]
fn xml_template_reads_the_incumbent_namespace_and_cdata_body() {
    let t = parse_template_xml(include_str!("../../tests/fixtures/template.xml")).unwrap();
    assert_eq!(t.correlation_id.as_deref(), Some("c-1"));
    assert_eq!(t.body, OutgoingBody::Bytes(b"<order id=\"1\"/>".to_vec()));
    assert_eq!(t.properties["When"], TypedValue::DateTime(datetime!(2026-10-17 9:00 UTC)));
}

#[test]
fn unknown_property_types_are_refused_by_name() {
    let e = parse_template_json(r#"{"body":"x","properties":[{"key":"a","type":"Decimal","value":"1"}]}"#).unwrap_err().to_string();
    assert!(e.contains("Decimal") && e.contains("String, Int32, Int64, Double, Boolean, DateTime, TimeSpan, Guid"), "{e}");
}
// import.rs
#[test]
fn exported_json_and_ndjson_read_back_as_outgoing_messages() {
    let mut m = crate::messaging::test_message(7);
    m.body = MessageBody::Binary(b"hello".to_vec());
    m.message_id = Some("id-7".into());
    m.time_to_live = Some(Duration::from_secs(90));
    m.application_properties.insert("n".into(), PropValue::Int(3));
    let dir = tempfile::tempdir().unwrap();
    for format in [ExportFormat::Json, ExportFormat::Ndjson] {
        let path = dir.path().join(format!("{format:?}"));
        crate::export::write_messages(&path, &[m.clone()], format, false).unwrap();
        let read = read_messages(&path).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].source_sequence, Some(7));
        assert_eq!(read[0].message.body, OutgoingBody::Bytes(b"hello".to_vec()));
        assert_eq!(read[0].message.message_id.as_deref(), Some("id-7"));
        assert_eq!(read[0].message.time_to_live, Some(Duration::from_secs(90)));
        assert_eq!(read[0].message.properties["n"], TypedValue::Int(3));
    }
}

#[test]
fn a_bad_line_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.ndjson");
    std::fs::write(&p, "{\"schema\":\"dispatched.message/v1\",\"sequence_number\":1,\"state\":\"active\",\"body\":{\"kind\":\"empty\"},\"application_properties\":{},\"delivery_count\":0}\nnot json\n").unwrap();
    let e = read_messages(&p).unwrap_err().to_string();
    assert!(e.contains("line 2"), "{e}");
}

#[test]
fn other_schemas_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.json");
    std::fs::write(&p, r#"[{"schema":"something/v9"}]"#).unwrap();
    assert!(read_messages(&p).unwrap_err().to_string().contains("dispatched.message/v1"));
}
```

(Adapt the `write_messages` call to its real signature in `export.rs`. The fixtures are hand-written from Decision 10's field list: `template.json` has all fields incl. `"timeToLive": "01:00:00"`, properties `Country` (String) and `Quantity` (Int32); `template.xml` uses `<brokeredMessage xmlns="http://schemas.microsoft.com/servicebusexplorer">`, a CDATA body and a `DateTime` property `2026-10-17T09:00:00Z`.)

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::template import::` → FAIL.
- [ ] **Step 3: Implement** with `serde_json` (a private `#[derive(Deserialize)]` mirror of the export shape, `#[serde(default)]` everywhere) and `quick-xml` matching local names (as `mgmt/xml.rs` does).
- [ ] **Step 4: Run** — `cargo test -p dispatched-core` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/messaging/template.rs crates/dispatched-core/src/import.rs crates/dispatched-core/tests/fixtures/template.json crates/dispatched-core/tests/fixtures/template.xml crates/dispatched-core/src/messaging/mod.rs crates/dispatched-core/src/lib.rs
git commit -m "Core: read BrokeredMessageTemplate JSON/XML and Dispatched export files for sending"
```

---

### Task 8: Core: management requests for the message plane

**Files:**
- Create: `crates/dispatched-core/src/messaging/lockops.rs`
- Modify: `crates/dispatched-core/src/messaging/management.rs` (`call_for_link`, error mapping), `crates/dispatched-core/src/messaging/mod.rs`

**Interfaces:**
- Consumes: `MgmtResponse`, `decode_bytes`, Task 1 errors, Task 6 `to_amqp`.
- Produces (request builders return `OrderedMap<String, Value>`; parsers take `MgmtResponse`):
  - `ManagementLink::call_for_link(&mut self, operation, body, associated_link: Option<&str>) -> Result<MgmtResponse>` (adds application property `associated-link-name`); `call` delegates with `None`.
  - `MgmtResponse::error` additionally maps status `410` → `Error::LockLost(..)` and descriptions containing `com.microsoft:message-lock-lost` / `session-lock-lost` → `LockLost`, `com.microsoft:session-cannot-be-locked` → `Conflict("… is locked by another receiver; stop that receiver or wait for its lock to expire")`, `409` → `Conflict`.
  - `schedule_request(messages: &[(Vec<u8> /* encoded AMQP */, String /* message id */, Option<String> /* session */, Option<String> /* partition key */)]) -> OrderedMap` (`messages` list of maps `{message-id, session-id?, partition-key?, message: binary}`); `parse_schedule_response(resp) -> Result<Vec<i64>>` (`sequence-numbers` long array).
  - `cancel_scheduled_request(seqs: &[i64])` (`sequence-numbers` long array); `parse_status_only(resp, op) -> Result<()>` (200/204).
  - `renew_lock_request(tokens: &[Uuid])` (`lock-tokens` uuid array); `parse_renew_lock_response -> Result<Vec<OffsetDateTime>>` (`expirations`).
  - `receive_by_sequence_request(seqs: &[i64], peek_lock: bool, session: Option<&str>)` (`sequence-numbers`, `receiver-settle-mode` uint: `1` peek-lock, `0` receive-and-delete, `session-id`); `parse_receive_by_sequence_response -> Result<Vec<(PeekedMessage, Uuid)>>` (`messages` list of `{message: binary, lock-token: uuid}`).
  - `update_disposition_request(tokens: &[Uuid], status: DispositionStatus, dead_letter: Option<(&str, &str)>, session: Option<&str>)` with `pub enum DispositionStatus { Completed, Abandoned, Suspended, Defered }` → `"completed" | "abandoned" | "suspended" | "defered"` (the broker's spelling).
  - `get_session_state_request(session)`, `parse_session_state_response -> Result<Option<Vec<u8>>>` (`session-state` binary or null); `set_session_state_request(session, Option<&[u8]>)` (null clears); `renew_session_lock_request(session)`, `parse_renew_session_lock_response -> Result<OffsetDateTime>` (`expiration`).
  - Operation name constants: `SCHEDULE = "com.microsoft:schedule-message"`, `CANCEL_SCHEDULED = "com.microsoft:cancel-scheduled-message"`, `RENEW_LOCK = "com.microsoft:renew-lock"`, `RECEIVE_BY_SEQUENCE = "com.microsoft:receive-by-sequence-number"`, `UPDATE_DISPOSITION = "com.microsoft:update-disposition"`, `GET_SESSION_STATE = "com.microsoft:get-session-state"`, `SET_SESSION_STATE = "com.microsoft:set-session-state"`, `RENEW_SESSION_LOCK = "com.microsoft:renew-session-lock"`.

- [ ] **Step 1: Write the failing tests** (shape tests per request plus each parser's 200/204/error paths)

```rust
#[test]
fn schedule_request_wraps_each_encoded_message() {
    let body = schedule_request(&[(vec![1, 2], "m1".into(), Some("s".into()), None)]);
    let Some(Value::List(items)) = body.get("messages") else { panic!() };
    let Value::Map(m) = &items[0] else { panic!() };
    assert_eq!(m.get(&Value::String("message-id".into())), Some(&Value::String("m1".into())));
    assert_eq!(m.get(&Value::String("session-id".into())), Some(&Value::String("s".into())));
    assert!(matches!(m.get(&Value::String("message".into())), Some(Value::Binary(b)) if b.as_slice() == [1, 2]));
}

#[test]
fn schedule_response_gives_sequence_numbers() {
    let r = resp(200, map([("sequence-numbers", Value::Array(Array(vec![Value::Long(5), Value::Long(6)])))]));
    assert_eq!(parse_schedule_response(r).unwrap(), [5, 6]);
}

#[test]
fn update_disposition_uses_the_broker_spelling_and_reason() {
    let b = update_disposition_request(&[Uuid::nil()], DispositionStatus::Suspended, Some(("Bad", "why")), None);
    assert_eq!(b.get("disposition-status"), Some(&Value::String("suspended".into())));
    assert_eq!(b.get("deadletter-reason"), Some(&Value::String("Bad".into())));
    assert_eq!(b.get("deadletter-description"), Some(&Value::String("why".into())));
    assert_eq!(update_disposition_request(&[], DispositionStatus::Defered, None, None).get("disposition-status"), Some(&Value::String("defered".into())));
}

#[test]
fn receive_by_sequence_returns_messages_with_lock_tokens() { /* map with messages: [{message: <encoded seq 9>, lock-token: uuid}] → seq 9 + that uuid */ }

#[test]
fn session_state_null_is_none_and_bytes_are_kept() {
    assert_eq!(parse_session_state_response(resp(200, map([("session-state", Value::Null)]))).unwrap(), None);
    assert_eq!(parse_session_state_response(resp(200, map([("session-state", Value::Binary(ByteBuf::from(vec![7])))]))).unwrap(), Some(vec![7]));
    assert!(matches!(set_session_state_request("s", None).get("session-state"), Some(Value::Null)));
}

#[test]
fn lock_lost_and_locked_sessions_map_to_their_errors() {
    assert!(matches!(MgmtResponse { status: 410, description: Some("lock lost".into()), body: Value::Null }.error("renew-lock"), Error::LockLost(_)));
    let busy = MgmtResponse { status: 400, description: Some("com.microsoft:session-cannot-be-locked: x".into()), body: Value::Null };
    assert!(matches!(busy.error("accept"), Error::Conflict(m) if m.contains("locked by another receiver")));
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::lockops messaging::management` → FAIL.
- [ ] **Step 3: Implement** the builders and parsers; `call_for_link` is a refactor of `call` (same timeouts).
- [ ] **Step 4: Run** — `cargo test -p dispatched-core messaging::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/messaging/lockops.rs crates/dispatched-core/src/messaging/management.rs crates/dispatched-core/src/messaging/mod.rs
git commit -m "Core: management requests for schedule, cancel, renew lock, deferred receive, disposition and session state"
```

---

### Task 9: Core: AMQP receiver links (peek-lock, receive-and-delete, settlement, sessions)

**Files:**
- Create: `crates/dispatched-core/src/messaging/receiver.rs`
- Modify: `crates/dispatched-core/src/messaging/connection.rs` (`ServiceBusConnection::open_receiver`), `crates/dispatched-core/src/messaging/mod.rs`

**Interfaces:**
- Consumes: Task 8 (`ManagementLink::call_for_link`, lockops), `ServiceBusConnection::{ensure_authorized, session_mut}`, `decode_message`, `map_receiver_attach_error`.
- Produces:
  - `pub enum ReceiveMode { PeekLock, ReceiveAndDelete }`; `pub enum SessionChoice { None, Id(String), NextAvailable }`; `pub struct ReceiverSpec { pub address: String, pub mode: ReceiveMode, pub session: SessionChoice }`.
  - `pub struct LockToken(pub Uuid)` (Copy, Hash, Serialize as the UUID string); `pub fn lock_token_from_tag(tag: &[u8]) -> Option<LockToken>` (`Uuid::from_bytes_le`, `None` unless 16 bytes).
  - `pub struct ReceivedMessage { pub message: PeekedMessage, pub lock_token: Option<LockToken> /* None for receive-and-delete */ }`.
  - `pub enum Settlement { Complete, Abandon, Defer, DeadLetter { reason: String, description: String } }`.
  - `pub(crate) struct AmqpReceiver` with: `name()`, `session_id() -> Option<&str>` (the accepted session, read from the remote attach's source filter), `receive(&mut self, max: u32, wait: Duration) -> Result<Vec<ReceivedMessage>>`, `settle(&mut self, token: LockToken, how: &Settlement) -> Result<()>`, `release(&mut self, token: LockToken) -> Result<()>`, `release_all(&mut self) -> Result<u64>`, `held(&self) -> usize`, `renew_lock(&mut self, tokens) -> Result<Vec<OffsetDateTime>>`, `receive_deferred(&mut self, seqs: &[i64]) -> Result<Vec<ReceivedMessage>>`, `session_state(&mut self) -> Result<Option<Vec<u8>>>`, `set_session_state(&mut self, Option<&[u8]>) -> Result<()>`, `renew_session_lock(&mut self) -> Result<OffsetDateTime>`, `close(self)`.
  - `pub(crate) async fn ServiceBusConnection::open_receiver(&mut self, spec: &ReceiverSpec) -> Result<AmqpReceiver>` (puts tokens for the address and its `$management`, attaches the data link and a private management link on the same session).
  - `pub(crate) fn credit_for(remaining: u64, chunk: u32) -> u32` = `min(remaining, chunk)` (never above `remaining`).

**Protocol rules (from research, binding):**
- Peek-lock: `receiver_settle_mode(Second)`, `credit_mode(Manual)`. Receive-and-delete: `sender_settle_mode(Settled)`, `credit_mode(Manual)`. Credit is granted per `receive` call with `credit_for(max − delivered, 50)` and topped up only by what was delivered, so outstanding credit never exceeds what the caller still wants.
- When `wait` passes with credit outstanding, call `drain()` and keep reading until the drain completes; deliveries that arrive during the drain are returned (receive-and-delete: they are already removed).
- Session filter: source filter `com.microsoft:session-filter` = described value (descriptor code `0x0000_0137_0000_000C`) holding the session id string, or `Null` for next available; build with `verify_incoming_source(false)` so the broker's chosen id is accepted; after attach, read the id back from `receiver.source()`'s filter. A next-available attach that times out with no session (`com.microsoft:timeout`, or no attach within `wait`) is `Ok(None)` from a separate `try_accept_next(...) -> Result<Option<AmqpReceiver>>`.
- Settlement over the link: Complete = `accept`; Abandon = `modify { delivery_failed: Some(true), undeliverable_here: Some(false) }`; Defer = `modify { delivery_failed: None, undeliverable_here: Some(true) }`; DeadLetter = `reject` with condition `com.microsoft:dead-letter` and info `DeadLetterReason` / `DeadLetterErrorDescription` (as `testing.rs` does); release = `release`. Deliveries are kept in a `HashMap<LockToken, Delivery<Body<Value>>>` until settled. A broker rejection with `com.microsoft:message-lock-lost` maps to `Error::LockLost`.
- Messages received by sequence number have no delivery: their tokens are kept in a separate set and settled with `update-disposition` (`Completed`, `Abandoned`, `Defered`, `Suspended` + reason) over the receiver's management link with `associated-link-name`.
- Renew lock and session operations go over the receiver's management link with `associated-link-name = name()` and `session-id` when the receiver has a session.

- [ ] **Step 1: Write the failing tests** (pure pieces; link behaviour is covered by Task 22's gate)

```rust
#[test]
fn credit_never_exceeds_what_is_still_wanted() {
    assert_eq!(credit_for(3, 50), 3);
    assert_eq!(credit_for(500, 50), 50);
    assert_eq!(credit_for(0, 50), 0);
}

#[test]
fn receive_and_delete_never_grants_more_credit_than_remaining() {
    // The credit plan for 120 wanted with 50 per grant and deliveries of 50, 50, 20.
    let mut plan = CreditPlan::new(120, 50);
    assert_eq!(plan.next_grant(), 50);
    plan.delivered(50); assert_eq!(plan.next_grant(), 50);
    plan.delivered(50); assert_eq!(plan.next_grant(), 20);
    plan.delivered(20); assert_eq!(plan.next_grant(), 0);
    assert!(plan.outstanding_max() <= 120);
}

#[test]
fn lock_tokens_read_the_delivery_tag_as_a_dotnet_guid() {
    let tag = [0x33, 0x22, 0x11, 0x00, 0x55, 0x44, 0x77, 0x66, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
    assert_eq!(lock_token_from_tag(&tag).unwrap().0.to_string(), "00112233-4455-6677-8899-aabbccddeeff");
    assert!(lock_token_from_tag(&[1, 2, 3]).is_none());
}

#[test]
fn session_filter_is_the_described_session_id_or_null() {
    let f = session_filter(&SessionChoice::Id("RMA-1".into())).unwrap();
    let v = f.get(&Symbol::from("com.microsoft:session-filter")).unwrap();
    assert!(matches!(v, Value::Described(d) if d.value == Value::String("RMA-1".into())));
    assert!(matches!(session_filter(&SessionChoice::NextAvailable).unwrap().values().next(), Some(Value::Described(d)) if d.value == Value::Null));
    assert!(session_filter(&SessionChoice::None).is_none());
}

#[test]
fn settlements_map_to_the_service_bus_outcomes() {
    assert!(matches!(outcome_for(&Settlement::Abandon), LinkOutcome::Modified { delivery_failed: Some(true), undeliverable_here: Some(false) }));
    assert!(matches!(outcome_for(&Settlement::Defer), LinkOutcome::Modified { delivery_failed: None, undeliverable_here: Some(true) }));
    let LinkOutcome::Rejected(e) = outcome_for(&Settlement::DeadLetter { reason: "R".into(), description: "D".into() }) else { panic!() };
    assert_eq!(e.condition, ErrorCondition::Custom(Symbol::from("com.microsoft:dead-letter")));
}
```

(`CreditPlan`, `session_filter`, `outcome_for` and `LinkOutcome` are private helpers the implementation uses, so the tested logic is the logic that runs.)

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::receiver` → FAIL.
- [ ] **Step 3: Implement** `AmqpReceiver` and `open_receiver` per the protocol rules. All network steps use `SETUP_TIMEOUT` (30 s) except `receive`'s own `wait`.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core messaging::` → PASS; `cargo clippy -p dispatched-core --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/messaging/receiver.rs crates/dispatched-core/src/messaging/connection.rs crates/dispatched-core/src/messaging/mod.rs
git commit -m "Core: peek-lock and receive-and-delete receivers with bounded credit, settlement and session receivers"
```

---

### Task 10: Core: AMQP sender, schedule and cancel

**Files:**
- Create: `crates/dispatched-core/src/messaging/sender.rs`
- Modify: `crates/dispatched-core/src/messaging/connection.rs` (`open_sender`, `schedule`, `cancel_scheduled`), `crates/dispatched-core/src/messaging/mod.rs`

**Interfaces:**
- Consumes: Task 6 (`OutgoingMessage::to_amqp`, `validate`), Task 8 (`schedule_request`, `cancel_scheduled_request`).
- Produces:
  - `pub(crate) struct AmqpSender` with `send_all(&mut self, batch: &[OutgoingMessage], mut each: impl FnMut(usize, Result<()>)) -> Result<()>` (pipelines up to `MAX_IN_FLIGHT = 64` unsettled sends, reports each outcome by index; an outcome other than `accepted` is that message's `Error::Protocol("the broker refused the message: …")`; a link-level failure fails the rest with that error and returns it), `close(self)`.
  - `pub(crate) async fn ServiceBusConnection::open_sender(&mut self, address: &str) -> Result<AmqpSender>`.
  - `pub(crate) async fn ServiceBusConnection::schedule(&mut self, address: &str, batch: &[OutgoingMessage], at: OffsetDateTime) -> Result<Vec<i64>>` (sets `scheduled_enqueue_time = at` on each encoded message, message id required: a missing one gets a new UUID, chunks of 100 per request).
  - `pub(crate) async fn ServiceBusConnection::cancel_scheduled(&mut self, address: &str, seqs: &[i64]) -> Result<()>` (chunks of 100).
  - `pub(crate) fn encode_for_schedule(m: &OutgoingMessage, at: OffsetDateTime, now) -> Result<(Vec<u8>, String, Option<String>, Option<String>)>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn schedule_encoding_carries_the_time_and_a_message_id() {
    let now = datetime!(2026-10-17 12:00 UTC);
    let at = now + time::Duration::hours(1);
    let (bytes, id, session, _) = encode_for_schedule(&OutgoingMessage { session_id: Some("s".into()), ..OutgoingMessage::text("x") }, at, now).unwrap();
    assert!(uuid::Uuid::parse_str(&id).is_ok(), "a message id is generated");
    assert_eq!(session.as_deref(), Some("s"));
    let decoded = decode_bytes(&bytes).unwrap();
    assert_eq!(decoded.scheduled_enqueue_time, Some(at));
}

#[test]
fn schedule_in_the_past_is_refused() {
    let now = datetime!(2026-10-17 12:00 UTC);
    assert!(matches!(encode_for_schedule(&OutgoingMessage::text("x"), now - time::Duration::minutes(1), now), Err(Error::Invalid(m)) if m.contains("in the past")));
}

#[test]
fn requests_are_chunked_by_100() {
    assert_eq!(chunks(&(0..250).collect::<Vec<i64>>()).map(|c| c.len()).collect::<Vec<_>>(), [100, 100, 50]);
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::sender` → FAIL.
- [ ] **Step 3: Implement.** Sends to a subscription path are refused before any I/O: `Error::Invalid("send to the topic `t`, not to its subscription")`.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core messaging::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/messaging/sender.rs crates/dispatched-core/src/messaging/connection.rs crates/dispatched-core/src/messaging/mod.rs
git commit -m "Core: pipelined sender, scheduling with sequence numbers, cancel scheduled"
```

---

### Task 11: Core: the message-plane traits, the fake broker and ExplorerPlane

**Files:**
- Create: `crates/dispatched-core/src/messaging/backend.rs`, `crates/dispatched-core/src/messaging/fake.rs`
- Modify: `crates/dispatched-core/src/explorer.rs`, `crates/dispatched-core/src/messaging/mod.rs`

**Interfaces:**
- Consumes: Tasks 9–10, `PeekSource`, `Explorer`'s `Lifecycle`/`drop_if_broken`.
- Produces:
  - ```rust
    #[async_trait]
    pub(crate) trait MessagingBackend: PeekSource + Send {
        async fn open_sender(&mut self, address: &str) -> Result<Box<dyn MessageSender>>;
        async fn schedule(&mut self, address: &str, batch: &[OutgoingMessage], at: OffsetDateTime) -> Result<Vec<i64>>;
        async fn cancel_scheduled(&mut self, address: &str, seqs: &[i64]) -> Result<()>;
        /// `Ok(None)` only for `SessionChoice::NextAvailable` when no session is available within `wait`.
        async fn open_receiver(&mut self, spec: &ReceiverSpec, wait: Duration) -> Result<Option<Box<dyn LockReceiver>>>;
        async fn entity_counts(&mut self, entity: &EntityPath) -> Result<Option<EntityCounts>>; // management counts, None when unavailable
        async fn session_ids(&mut self, address: &str) -> Result<Vec<String>>;                   // broker list ∪ peek tally
    }
    #[async_trait] pub(crate) trait MessageSender: Send { async fn send_all(&mut self, batch: &[OutgoingMessage], each: &mut (dyn FnMut(usize, Result<()>) + Send)) -> Result<()>; async fn close(self: Box<Self>); }
    #[async_trait] pub(crate) trait LockReceiver: Send { /* the AmqpReceiver methods from Task 9, by &mut self, plus */ async fn close(self: Box<Self>); }
    ```
  - `impl MessageSender for AmqpSender`, `impl LockReceiver for AmqpReceiver`.
  - `pub(crate) struct ExplorerPlane<'a>(&'a Explorer)` implementing `PeekSource` (via `PerBatch`) and `MessagingBackend`: each call locks the cached connection only for that call; `open_*` may retry once on a connection-level error (`with_one_retry`); `schedule` and `cancel_scheduled` never retry but drop a broken connection (`drop_if_broken`). `entity_counts` uses `ManagementClient` (`get_queue` / subscription details) and returns `Ok(None)` on `forbidden`. `pub(crate) fn Explorer::plane(&self) -> ExplorerPlane<'_>`.
  - `#[cfg(test)] pub(crate) struct FakeBroker` (Clone; shared `Arc<Mutex<State>>`), implementing all three traits plus `PeekSource`:
    - `add(&self, address, FakeMsg) -> i64` (sequence numbers increase per address), `FakeMsg { session: Option<String>, state: MessageState, body: &'static str, props: Vec<(&str, PropValue)>, message_id: Option<String> }` with `FakeMsg::active("body")`, `.in_session(id)`, `.deferred()`, `.scheduled()`, `.prop(k, v)`.
    - inspection: `all(address) -> Vec<PeekedMessage>`, `delivery_count(address, seq)`, `credit_granted_max() -> u32` (largest outstanding credit seen), `held_locks() -> usize`, `sent(address) -> Vec<OutgoingMessage>`, `session_state(address, id)`.
    - fault injection: `fail_next(FakeOp, Error)` with `FakeOp { Send, Settle, Receive, OpenReceiver, Schedule, Cancel }`, `expire_lock_on_next_settle()`, `lock_session_elsewhere(address, id)`, `deliver_late(n)` (n deliveries arrive only during the drain).
    - semantics: peek-lock locks a delivered message until settled or released; release keeps the delivery count; abandon raises it and dead-letters at `max_delivery_count = 10` into `{address}/$DeadLetterQueue` with reason `MaxDeliveryCountExceeded`; dead-letter moves it with the given reason; defer sets state deferred; complete removes; receive-and-delete removes on delivery; deferred and scheduled messages are never delivered by `receive`; `receive_deferred` returns deferred messages by sequence number with a lock; next-available accepts the lowest unlocked session id that has messages, then those with state.

- [ ] **Step 1: Write the failing tests** (fake semantics the ops rely on, in `fake.rs`)

```rust
#[tokio::test]
async fn peek_lock_holds_until_released_and_release_keeps_the_delivery_count() {
    let b = FakeBroker::default();
    let seq = b.add("q", FakeMsg::active("a"));
    let mut plane = b.clone();
    let mut r = plane.open_receiver(&ReceiverSpec::peek_lock("q"), WAIT).await.unwrap().unwrap();
    let got = r.receive(1, WAIT).await.unwrap();
    assert_eq!(got[0].message.sequence_number, seq);
    assert!(r.receive(1, Duration::from_millis(10)).await.unwrap().is_empty(), "locked: not delivered twice");
    r.release(got[0].lock_token.unwrap()).await.unwrap();
    assert_eq!(b.delivery_count("q", seq), 0);
}

#[tokio::test]
async fn abandon_raises_the_count_and_dead_letters_at_the_limit() { /* 10 abandons → message in q/$DeadLetterQueue with reason MaxDeliveryCountExceeded */ }

#[tokio::test]
async fn receive_and_delete_removes_on_delivery_and_records_credit() {
    let b = FakeBroker::default();
    for _ in 0..5 { b.add("q", FakeMsg::active("x")); }
    let mut r = b.clone().open_receiver(&ReceiverSpec::receive_and_delete("q"), WAIT).await.unwrap().unwrap();
    assert_eq!(r.receive(3, WAIT).await.unwrap().len(), 3);
    assert_eq!(b.all("q").len(), 2);
    assert!(b.credit_granted_max() <= 3);
}

#[tokio::test]
async fn next_available_session_skips_sessions_locked_elsewhere() { /* sessions a (locked elsewhere) and b → accepts b; then none → Ok(None) */ }

#[tokio::test]
async fn explorer_plane_never_retries_a_failed_schedule() {
    // A connector fake (as in explorer.rs tests) whose schedule fails once with Network: one call, the error is returned, the cached connection is dropped.
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core messaging::fake explorer::` → FAIL.
- [ ] **Step 3: Implement** the traits, `ExplorerPlane` and `FakeBroker`. Keep `fake.rs` focused: one `State` struct, one `impl` per trait.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/messaging/backend.rs crates/dispatched-core/src/messaging/fake.rs crates/dispatched-core/src/explorer.rs crates/dispatched-core/src/messaging/mod.rs
git commit -m "Core: message-plane traits, an in-memory fake broker, and the explorer's per-call plane"
```

---

### Task 12: Core: Operator, send, receive, deferred settle, cancel scheduled, session state

**Files:**
- Create: `crates/dispatched-core/src/ops/mod.rs`, `crates/dispatched-core/src/ops/send.rs`, `crates/dispatched-core/src/ops/receive.rs`, `crates/dispatched-core/src/ops/session_state.rs`
- Modify: `crates/dispatched-core/src/lib.rs` (`pub mod ops`; re-export `Operator`, specs)

**Interfaces:**
- Consumes: Task 4 (`Gate`, `Operation`, `Audited`), Task 3 (`JobRun`, `JobReport`, `Progress`), Task 6 (`OutgoingMessage`, `MessageIdSource`, `BodySource`), Task 11 (traits, `FakeBroker`, `Explorer::plane`).
- Produces:
  - `pub struct Operator { explorer: Arc<Explorer>, gate: Gate }`, `Operator::new(explorer: Arc<Explorer>, gate: Gate)`, `explorer(&self) -> &Explorer`, `gate(&self) -> &Gate`. Each public method below is a thin wrapper: build the `Operation` (with count), `self.gate.run(&op, || ops::x::run(&mut self.explorer.plane(), …))`. The `run` functions are `pub(crate)` and generic over `B: MessagingBackend + ?Sized`, so tests drive them with `FakeBroker` and a `Gate` over `MemoryAudit`.
  - `pub struct SendRequest { pub target: String /* queue or topic */, pub template: OutgoingMessage, pub body: Option<BodySource> /* overrides template.body per message */, pub ids: MessageIdSource, pub count: u64 /* ≥ 1, ≤ 100,000 */, pub schedule: Option<OffsetDateTime>, pub batch: bool }`. `Operator::send(&self, req: SendRequest, progress, cancel) -> Result<JobReport>`; non-batch sends one message per `send_all` call; `batch` sends all through one call; scheduled sends go through `schedule` in chunks and record `new_sequence_number` per item.
  - `Operator::cancel_scheduled(&self, entity: &EntityPath, seqs: &[i64], progress, cancel) -> Result<JobReport>` (unknown sequence numbers: the broker ignores them, so each requested one is first checked with a peek of the scheduled view and reported `missing` when absent).
  - `pub struct ReceiveRequest { pub entity: EntityPath, pub sub_queue: SubQueue, pub session: Option<String>, pub max: u32 /* ≤ 5,000 */, pub wait: Duration, pub action: ReceiveAction }`, `pub enum ReceiveAction { Settle(Settlement), Delete /* receive-and-delete */ }`. `Operator::receive(&self, req) -> Result<ReceiveOutcome>`; `pub struct ReceiveOutcome { pub messages: Vec<PeekedMessage>, pub report: JobReport }` (`impl Audited` via the report). Peek-lock settles each received message with the action as soon as it arrives; `Delete` uses a receive-and-delete link; both stop at `max`, at `wait` without a delivery, or on cancel, and both drain-collect.
  - `Operator::settle_deferred(&self, entity, sub_queue, seqs: &[i64], session: Option<&str>, how: Settlement, progress, cancel) -> Result<JobReport>` (receive by sequence number in chunks of 100, then `update-disposition`; seqs not returned are `missing`).
  - `Operator::session_state(&self, entity, session: &str) -> Result<Option<Vec<u8>>>` (read: no gate, no audit), `Operator::set_session_state(&self, entity, session: &str, state: Option<Vec<u8>>) -> Result<()>` (gated `SessionState`; `None` clears). Both accept the session with credit 0 (Decision 14).

- [ ] **Step 1: Write the failing tests** (`ops/*` against `FakeBroker` + `Gate` over `MemoryAudit` with an allow-all policy; a helper `fake_gate()` in `ops/mod.rs` tests)

```rust
#[tokio::test]
async fn send_n_copies_with_incrementing_ids() {
    let b = FakeBroker::default();
    let r = send::run(&mut b.clone(), &SendRequest { target: "q".into(), template: OutgoingMessage::text("x"),
        body: None, ids: MessageIdSource::Increment { base: "m-1".into() }, count: 3, schedule: None, batch: false }, &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 3);
    let ids: Vec<_> = b.sent("q").into_iter().map(|m| m.message_id.unwrap()).collect();
    assert_eq!(ids, ["m-1", "m-2", "m-3"]);
}

#[tokio::test]
async fn a_failed_send_is_one_failed_item_and_the_rest_continue() {
    let b = FakeBroker::default();
    b.fail_next(FakeOp::Send, Error::Protocol("refused".into()));
    let r = send::run(&mut b.clone(), &three_texts(), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!((r.succeeded(), r.failed()), (2, 1));
}

#[tokio::test]
async fn scheduled_sends_record_their_sequence_numbers() { /* schedule: Some(t) → items carry new_sequence_number, b.all("q") shows scheduled state */ }

#[tokio::test]
async fn peek_lock_receive_settles_each_message_and_stops_at_max() {
    let b = FakeBroker::default();
    for _ in 0..5 { b.add("q", FakeMsg::active("x")); }
    let out = receive::run(&mut b.clone(), &recv("q", 3, ReceiveAction::Settle(Settlement::Complete)), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(out.messages.len(), 3);
    assert_eq!(b.all("q").len(), 2);
    assert_eq!(b.held_locks(), 0);
    assert!(b.credit_granted_max() <= 3);
}

#[tokio::test]
async fn receive_and_delete_reports_messages_drained_after_the_wait() {
    let b = FakeBroker::default();
    for _ in 0..4 { b.add("q", FakeMsg::active("x")); }
    b.deliver_late(2);
    let out = receive::run(&mut b.clone(), &recv("q", 4, ReceiveAction::Delete), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(out.messages.len(), 4, "late deliveries are already removed, so they are returned");
    assert!(b.credit_granted_max() <= 4);
}

#[tokio::test]
async fn dead_letter_receive_carries_the_reason() { /* Settlement::DeadLetter{reason:"Manual",description:"bad"} → message in q/$DeadLetterQueue with that reason */ }

#[tokio::test]
async fn deferred_messages_settle_by_sequence_number_and_missing_ones_are_reported() {
    let b = FakeBroker::default();
    let d = b.add("q", FakeMsg::active("x").deferred());
    let r = receive::settle_deferred(&mut b.clone(), "q", &[d, 999], None, &Settlement::Complete, &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!((r.succeeded(), r.skipped()), (1, 1));
    assert!(matches!(r.items[1].outcome, ItemOutcome::Missing));
}

#[tokio::test]
async fn session_state_round_trips_and_a_locked_session_is_a_conflict() {
    let b = FakeBroker::default();
    b.add("s", FakeMsg::active("x").in_session("A"));
    session_state::set(&mut b.clone(), "s", "A", Some(b"{\"step\":2}".to_vec())).await.unwrap();
    assert_eq!(session_state::get(&mut b.clone(), "s", "A").await.unwrap(), Some(b"{\"step\":2}".to_vec()));
    session_state::set(&mut b.clone(), "s", "A", None).await.unwrap();
    assert_eq!(session_state::get(&mut b.clone(), "s", "A").await.unwrap(), None);
    b.lock_session_elsewhere("s", "A");
    assert!(matches!(session_state::get(&mut b.clone(), "s", "A").await, Err(Error::Conflict(_))));
}

#[tokio::test]
async fn gated_send_with_a_denying_policy_sends_nothing() {
    // `ops::gated(&gate, &op, work)` is the one helper every Operator method uses.
    let b = FakeBroker::default();
    let (gate, audit) = deny_gate();
    let op = Operation::send(Target::new(EntityPath::queue("q"), SubQueue::Active), 3, None);
    let e = gated(&gate, &op, || send::run(&mut b.clone(), &three_texts(), &NoProgress, &CancellationToken::new())).await.unwrap_err();
    assert!(matches!(e, Error::Denied(_)));
    assert!(b.sent("q").is_empty());
    assert_eq!(audit.entries()[0].outcome, Outcome::Denied);
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core ops::` → FAIL.
- [ ] **Step 3: Implement.** `ops::gated` is the single helper (`gate.run(op, work)`), so `Operator` methods contain no logic beyond building the `Operation` and calling `run`. Cancellation is checked before each message; a cancelled job returns its partial report with `cancelled = true` (not `Err(Cancelled)`), so the audit records `Cancelled` with counts.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core ops::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/ops crates/dispatched-core/src/lib.rs
git commit -m "Core: Operator with gated send, schedule, receive, deferred settle, cancel scheduled and session state"
```

---

### Task 13: Core: lock-and-match (find messages by sequence number with a lock)

**Files:**
- Create: `crates/dispatched-core/src/ops/locate.rs`
- Modify: `crates/dispatched-core/src/ops/mod.rs`

**Interfaces:**
- Consumes: Task 11 traits, `scan::collect_sequences`, Task 3 (`ItemResult`, `NotReached`).
- Produces:
  - `pub struct LocateLimits { pub idle: Duration /* 5 s */, pub max_examined: u64 /* 10,000 */, pub max_held: usize /* 2,000 */ }` (Default).
  - `pub(crate) enum Found { Locked { receiver_index: usize, received: ReceivedMessage }, Scheduled(PeekedMessage) }`.
  - `pub(crate) async fn locate<B, F, Fut>(plane: &mut B, address: &str, view: PeekView, seqs: &[i64], limits: &LocateLimits, cancel: &CancellationToken, mut act: F) -> Result<Vec<ItemResult>>` where `act(&mut dyn LockReceiver, Found) -> Fut` returns that message's `ItemResult`. Steps:
    1. Classify with `collect_sequences(plane, address, view, seqs, budget = max_examined, cancel, None)`: absent → `Missing`.
    2. Deferred ones (grouped by session): one receiver per session (`SessionChoice::Id` or `None`), `receive_deferred`, `act` each.
    3. Scheduled ones: `act(Found::Scheduled)` (the delete op cancels them; move/resubmit report `Skipped { reason: "scheduled messages cannot be moved; cancel or wait" }`).
    4. Active ones (grouped by session): a peek-lock receiver per group; `receive(1, idle)` repeatedly; a wanted sequence → `act` at once; another → kept locked (count towards `max_held`); stop when all wanted are found, on `idle`, at `max_examined`, at `max_held`, or on cancel; then `release_all`. Unfound wanted → `NotReached { why }`. A message seen twice (its lock expired and it came back) is skipped by sequence number.
    5. Every receiver is closed on every path (including errors and cancel).

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn finds_wanted_messages_and_releases_the_rest_without_raising_counts() {
    let b = FakeBroker::default();
    let seqs: Vec<i64> = (0..6).map(|i| b.add("q", FakeMsg::active(if i % 2 == 0 { "want" } else { "skip" }))).collect();
    let wanted = [seqs[0], seqs[2], seqs[4]];
    let results = locate(&mut b.clone(), "q", PeekView::Active, &wanted, &LocateLimits::default(), &CancellationToken::new(), complete_it).await.unwrap();
    assert!(results.iter().all(|r| matches!(r.outcome, ItemOutcome::Succeeded { .. })));
    assert_eq!(b.all("q").len(), 3);
    for s in [seqs[1], seqs[3], seqs[5]] { assert_eq!(b.delivery_count("q", s), 0, "released, not abandoned"); }
    assert_eq!(b.held_locks(), 0);
}

#[tokio::test]
async fn each_view_finds_its_own_state_and_reports_missing() {
    let b = FakeBroker::default();
    let d = b.add("q", FakeMsg::active("d").deferred());
    let s = b.add("q", FakeMsg::active("s").scheduled());
    let none = CancellationToken::new();
    let lim = LocateLimits::default();
    let r = locate(&mut b.clone(), "q", PeekView::Deferred, &[d, 404], &lim, &none, complete_or_cancel).await.unwrap();
    assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }), "deferred: received by sequence number");
    assert!(matches!(r[1].outcome, ItemOutcome::Missing));
    let r = locate(&mut b.clone(), "q", PeekView::Scheduled, &[s], &lim, &none, complete_or_cancel).await.unwrap();
    assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }), "scheduled: handed to act as Found::Scheduled");
    let r = locate(&mut b.clone(), "q", PeekView::Active, &[d], &lim, &none, complete_or_cancel).await.unwrap();
    assert!(matches!(r[0].outcome, ItemOutcome::Missing), "not in the active view");
    assert!(b.all("q").is_empty());
}

#[tokio::test]
async fn active_match_runs_inside_the_message_session() {
    let b = FakeBroker::default();
    let a = b.add("s", FakeMsg::active("x").in_session("A"));
    b.add("s", FakeMsg::active("y").in_session("B"));
    let r = locate(&mut b.clone(), "s", PeekView::Active, &[a], &LocateLimits::default(), &CancellationToken::new(), complete_it).await.unwrap();
    assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
    assert_eq!(r[0].session_id.as_deref(), Some("A"));
}

#[tokio::test]
async fn limits_stop_the_search_with_a_reason() {
    let b = FakeBroker::default();
    let mut last = 0;
    for _ in 0..50 { last = b.add("q", FakeMsg::active("x")); }
    let limits = LocateLimits { max_held: 10, ..Default::default() };
    let r = locate(&mut b.clone(), "q", PeekView::Active, &[last], &limits, &CancellationToken::new(), complete_it).await.unwrap();
    assert!(matches!(r[0].outcome, ItemOutcome::NotReached { why: NotReached::HeldLimit }));
    assert_eq!(b.held_locks(), 0);
}

#[tokio::test]
async fn lost_lock_fails_only_that_item() {
    let b = FakeBroker::default();
    let (x, y) = (b.add("q", FakeMsg::active("x")), b.add("q", FakeMsg::active("y")));
    b.expire_lock_on_next_settle();
    let r = locate(&mut b.clone(), "q", PeekView::Active, &[x, y], &LocateLimits::default(), &CancellationToken::new(), complete_it).await.unwrap();
    assert!(matches!(&r[0].outcome, ItemOutcome::Failed { kind: "lock_lost", reason } if reason.contains("run again")));
    assert!(matches!(r[1].outcome, ItemOutcome::Succeeded { .. }));
}

#[tokio::test]
async fn cancel_releases_held_locks_and_reports_not_reached() {
    let b = FakeBroker::default();
    let seqs: Vec<i64> = (0..5).map(|_| b.add("q", FakeMsg::active("x"))).collect();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let r = locate(&mut b.clone(), "q", PeekView::Active, &[seqs[0], seqs[4]], &LocateLimits::default(), &cancel,
        move |rx, f| { c2.cancel(); complete_it(rx, f) }).await.unwrap();
    assert!(matches!(r[0].outcome, ItemOutcome::Succeeded { .. }));
    assert!(matches!(r[1].outcome, ItemOutcome::NotReached { why: NotReached::Cancelled }));
    assert_eq!(b.held_locks(), 0);
}
```

(`complete_it` settles `Found::Locked` with `Complete` and returns `Succeeded`, mapping `LockLost` to `Failed { kind: "lock_lost", reason: "the lock expired; run again for this sequence number" }`; `complete_or_cancel` also cancels `Found::Scheduled`.)

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core ops::locate` → FAIL.
- [ ] **Step 3: Implement** `locate` as specified. The `act` closure gets the receiver that holds the lock, so settling happens on the same link.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core ops::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/ops/locate.rs crates/dispatched-core/src/ops/mod.rs
git commit -m "Core: lock-and-match by sequence number with release, limits, sessions and cancel"
```

---

### Task 14: Core: delete, move, resubmit, clone and resend

**Files:**
- Create: `crates/dispatched-core/src/ops/remove.rs`, `crates/dispatched-core/src/ops/resubmit.rs`
- Modify: `crates/dispatched-core/src/ops/mod.rs` (Operator methods)

**Interfaces:**
- Consumes: Task 13 (`locate`), Task 6 (`OutgoingMessage::from_peeked`, `Edits`, `CloneOptions`), Task 12 (`gated`).
- Produces:
  - `pub enum MessageSelection { Sequences(Vec<i64>), Matching { filter: MessageFilter, limit: u64 /* ≤ 100,000 */ } }`; `pub struct SelectSpec { pub entity: EntityPath, pub view: PeekView, pub selection: MessageSelection, pub limits: LocateLimits }`. `Matching` resolves to sequence numbers with a peek scan before confirming, so the confirmation always shows an exact count and the list.
  - `Operator::delete(&self, spec: SelectSpec, progress, cancel) -> Result<JobReport>`: active/deferred → complete; scheduled → cancel; DLQ and transfer DLQ in any state.
  - `pub struct ResubmitSpec { pub source: SelectSpec, pub target: String /* queue or topic; defaults to the source entity's queue/topic */, pub edits: Edits, pub new_message_id: bool, pub remove_original: bool, pub keep_dead_letter_properties: bool }`.
  - `Operator::resubmit(&self, spec: ResubmitSpec, progress, cancel) -> Result<JobReport>`; `Operator::move_messages(&self, source: SelectSpec, target: String, progress, cancel) -> Result<JobReport>` = resubmit with `remove_original: true`, no edits, ids kept, `JobKind::Move`.
  - Without `remove_original`: read the source messages with `collect_sequences`, clone + edit, send (one sender for the job), item `new_sequence_number` unknown (`None`), notes from cloning in the item's detail.
  - With `remove_original`: `locate` and, per found message: clone + edit from the locked message, send, then complete (Decision 5). The default target for a subscription source is its topic.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn delete_by_sequence_removes_only_those() {
    let b = FakeBroker::default();
    let dl = "q/$DeadLetterQueue";
    let s: Vec<i64> = (0..4).map(|_| b.add(dl, FakeMsg::active("x"))).collect();
    let r = remove::run(&mut b.clone(), &select(dl, PeekView::DeadLetter, &[s[1], s[3]]), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 2);
    assert_eq!(b.all(dl).iter().map(|m| m.sequence_number).collect::<Vec<_>>(), [s[0], s[2]]);
}

#[tokio::test]
async fn delete_cancels_scheduled_messages() { /* view Scheduled → cancel_scheduled called, message gone */ }

#[tokio::test]
async fn move_sends_then_completes_and_keeps_the_message_id() {
    let b = FakeBroker::default();
    let s = b.add("q/$DeadLetterQueue", FakeMsg { message_id: Some("m-7".into()), ..FakeMsg::active("x") });
    let r = resubmit::run(&mut b.clone(), &move_spec("q/$DeadLetterQueue", &[s], "target"), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 1);
    assert!(b.all("q/$DeadLetterQueue").is_empty());
    assert_eq!(b.sent("target")[0].message_id.as_deref(), Some("m-7"));
    assert!(!b.sent("target")[0].properties.contains_key("DeadLetterReason"));
}

#[tokio::test]
async fn move_keeps_the_original_when_the_send_fails() {
    let b = FakeBroker::default();
    let s = b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
    b.fail_next(FakeOp::Send, Error::Protocol("refused".into()));
    let r = resubmit::run(&mut b.clone(), &move_spec("q/$DeadLetterQueue", &[s], "target"), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.failed(), 1);
    assert_eq!(b.all("q/$DeadLetterQueue").len(), 1, "still there");
    assert_eq!(b.held_locks(), 0);
}

#[tokio::test]
async fn move_reports_possible_duplicate_when_complete_fails_after_send() {
    let b = FakeBroker::default();
    let s = b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
    b.fail_next(FakeOp::Settle, Error::LockLost("expired".into()));
    let r = resubmit::run(&mut b.clone(), &move_spec("q/$DeadLetterQueue", &[s], "target"), &NoProgress, &CancellationToken::new()).await.unwrap();
    let ItemOutcome::Failed { reason, .. } = &r.items[0].outcome else { panic!() };
    assert!(reason.starts_with("sent to target, but the original could not be removed") && reason.ends_with("it may be delivered twice"), "{reason}");
    assert_eq!(b.sent("target").len(), 1);
}

#[tokio::test]
async fn resubmit_without_removal_is_clone_and_resend_with_edits_and_new_ids() {
    let b = FakeBroker::default();
    let s = b.add("q/$DeadLetterQueue", FakeMsg { message_id: Some("orig".into()), ..FakeMsg::active("x").prop("kind", PropValue::String("a".into())) });
    let mut spec = resubmit_spec("q/$DeadLetterQueue", &[s], "q");
    spec.new_message_id = true;
    spec.edits.set.insert("kind".into(), TypedValue::String("b".into()));
    let r = resubmit::run(&mut b.clone(), &spec, &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 1);
    assert_eq!(b.all("q/$DeadLetterQueue").len(), 1, "original kept");
    let sent = &b.sent("q")[0];
    assert_ne!(sent.message_id.as_deref(), Some("orig"));
    assert_eq!(sent.properties["kind"], TypedValue::String("b".into()));
}

#[tokio::test]
async fn resubmit_from_an_active_queue_can_remove_the_original() { /* #771: source view Active, remove_original → source empty, target has it */ }

#[test]
fn a_subscription_source_defaults_to_its_topic() {
    assert_eq!(default_target(&EntityPath::subscription("events", "all")), "events");
    assert_eq!(default_target(&EntityPath::queue("orders")), "orders");
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core ops::remove ops::resubmit` → FAIL.
- [ ] **Step 3: Implement** both ops and the four `Operator` methods (`delete`, `resubmit`, `move_messages`, plus `resolve_selection` shared by them to turn `Matching` into sequence numbers before the gate).
- [ ] **Step 4: Run** — `cargo test -p dispatched-core ops::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/ops/remove.rs crates/dispatched-core/src/ops/resubmit.rs crates/dispatched-core/src/ops/mod.rs
git commit -m "Core: delete, move (at-least-once), resubmit and clone-and-resend jobs"
```

---

### Task 15: Core: purge and the DLQ summary

**Files:**
- Create: `crates/dispatched-core/src/ops/purge.rs`, `crates/dispatched-core/src/messaging/dlq.rs`
- Modify: `crates/dispatched-core/src/ops/mod.rs`, `crates/dispatched-core/src/explorer.rs` (`dlq_summary`), `crates/dispatched-core/src/messaging/mod.rs`, `crates/dispatched-core/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 11–13, `scan::walk_messages`, `count_messages`, `EntityCounts`.
- Produces:
  - `pub enum PurgeScope { Active, DeadLetter, TransferDeadLetter, All }` (kebab-case; `All` = active + deferred + DLQ + transfer DLQ).
  - `pub struct PurgePlan { pub entity: EntityPath, pub scope: PurgeScope, pub per_queue: Vec<(SubQueue, CountEstimate)>, pub deferred: u64, pub total: CountEstimate }` and `Operator::plan_purge(&self, entity: &EntityPath, scope) -> Result<PurgePlan>` (read: management counts first → `Exact`; else peek counts → `About` or `AtLeast(10,000)`; deferred counted by peek).
  - `Operator::purge(&self, plans: Vec<PurgePlan>, progress, cancel) -> Result<JobReport>`: one gate confirmation **per entity** (each `PurgePlan` is its own `Operation::purge` with its `total`), entities purged in order; a denied entity is reported as one `Skipped { reason: "denied: …" }` item and the rest continue. Per sub-queue: receive-and-delete with credit bounded by that queue's remaining allowance (`CountEstimate::bound`); on session-enabled entities (`RequiresSession`, or a session-required error on the first attach) loop `SessionChoice::NextAvailable` until `Ok(None)`, plus the listed session ids not yet visited; a session that is locked elsewhere → `Skipped { reason: "session X is locked by another receiver" }`; deferred messages → `receive_deferred` + complete in chunks of 100. Items are per message (sequence number, session). When the allowance is used up and messages remain, add the note `"{n} removed from {address}; more remain, run purge again"`.
  - `pub struct ReasonGroup { pub reason: Option<String>, pub description: Option<String>, pub count: u64, pub first_sequence: i64, pub last_sequence: i64, pub oldest: Option<OffsetDateTime>, pub newest: Option<OffsetDateTime> }`, `pub struct DlqSummary { pub groups: Vec<ReasonGroup> /* by count desc, then reason */, pub scanned: u64, pub truncated: bool }`, `pub(crate) async fn summarize(src, address, cap) -> Result<DlqSummary>`, `Explorer::dlq_summary(&self, entity: &EntityPath, sub_queue: SubQueue) -> Result<DlqSummary>` (cap 10,000; read-only).

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn purge_never_removes_more_than_confirmed() {
    let b = FakeBroker::default();
    for _ in 0..10 { b.add("q", FakeMsg::active("x")); }
    let plan = plan_with("q", PurgeScope::Active, CountEstimate::AtLeast(6));
    let r = purge::run(&mut b.clone(), &plan, &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 6);
    assert_eq!(b.all("q").len(), 4);
    assert!(b.credit_granted_max() <= 6);
    assert!(r.notes.iter().any(|n| n == "6 removed from q; more remain, run purge again"));
}

#[tokio::test]
async fn purge_all_covers_deferred_and_dead_letters_but_keeps_scheduled() {
    let b = FakeBroker::default();
    b.add("q", FakeMsg::active("a"));
    b.add("q", FakeMsg::active("d").deferred());
    let s = b.add("q", FakeMsg::active("s").scheduled());
    b.add("q/$DeadLetterQueue", FakeMsg::active("x"));
    let plan = plan_with_counts("q", PurgeScope::All, &[(SubQueue::Active, 1), (SubQueue::DeadLetter, 1)], 1);
    let r = purge::run(&mut b.clone(), &plan, &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 3);
    assert_eq!(b.all("q").iter().map(|m| m.sequence_number).collect::<Vec<_>>(), [s]);
    assert!(b.all("q/$DeadLetterQueue").is_empty());
}

#[tokio::test]
async fn purge_drains_every_session_and_skips_locked_ones() {
    let b = FakeBroker::default();
    b.add("s", FakeMsg::active("1").in_session("A"));
    b.add("s", FakeMsg::active("2").in_session("B"));
    b.add("s", FakeMsg::active("3").in_session("C"));
    b.lock_session_elsewhere("s", "B");
    let r = purge::run(&mut b.clone(), &plan_with("s", PurgeScope::Active, CountEstimate::Exact(3)).sessions(), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.succeeded(), 2);
    assert!(r.items.iter().any(|i| matches!(&i.outcome, ItemOutcome::Skipped { reason } if reason == "session B is locked by another receiver")));
}

#[tokio::test]
async fn bulk_purge_confirms_each_entity_and_continues_after_a_denial() {
    // Gate whose policy denies only target "b": entity a purged, b reported Skipped("denied: …"), c purged; MemoryAudit has a Denied entry for b.
}

#[tokio::test]
async fn dlq_summary_groups_reasons_and_descriptions() {
    let b = FakeBroker::default();
    let dl = "q/$DeadLetterQueue";
    for (r, d) in [("MaxDeliveryCountExceeded", "x"), ("MaxDeliveryCountExceeded", "x"), ("Manual", "bad id"), ("MaxDeliveryCountExceeded", "y")] {
        b.add(dl, FakeMsg::active("m").prop("DeadLetterReason", PropValue::String(r.into())).prop("DeadLetterErrorDescription", PropValue::String(d.into())));
    }
    let s = summarize(&mut b.clone(), dl, 10_000).await.unwrap();
    assert_eq!(s.groups.iter().map(|g| (g.reason.as_deref().unwrap(), g.description.as_deref().unwrap(), g.count)).collect::<Vec<_>>(),
        [("MaxDeliveryCountExceeded", "x", 2), ("Manual", "bad id", 1), ("MaxDeliveryCountExceeded", "y", 1)]);
    assert_eq!((s.scanned, s.truncated), (4, false));
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core ops::purge messaging::dlq` → FAIL.
- [ ] **Step 3: Implement.** `PurgePlan::sessions()` is a test convenience; production plans learn "session entity" from `EntityDetails` (`requires_session`) when the management read works, else from the first attach error, and the plane's `session_ids` supplies listed ids.
- [ ] **Step 4: Run** — `cargo test -p dispatched-core ops:: messaging::` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/ops/purge.rs crates/dispatched-core/src/messaging/dlq.rs crates/dispatched-core/src/ops/mod.rs crates/dispatched-core/src/explorer.rs crates/dispatched-core/src/messaging/mod.rs crates/dispatched-core/src/lib.rs
git commit -m "Core: bounded purge across sub-queues, sessions and entities; DLQ summary by reason"
```

---

### Task 16: Core: import and replay

**Files:**
- Create: `crates/dispatched-core/src/ops/import.rs`
- Modify: `crates/dispatched-core/src/ops/mod.rs`

**Interfaces:**
- Consumes: Task 7 (`ImportedMessage`), Task 12 (send plumbing, `gated`).
- Produces:
  - `pub struct ImportOptions { pub new_message_id: bool, pub keep_sessions: bool /* default true */, pub drop_dead_letter_properties: bool /* default true */ }` (Default with those values).
  - `Operator::import(&self, target: String, messages: Vec<ImportedMessage>, opts: ImportOptions, progress, cancel) -> Result<JobReport>` (`Operation::import`, `JobKind::Import`); one sender for the job; items carry `sequence_number = source_sequence` and the import notes as detail; an invalid message (`validate` fails) is a `Failed { kind: "invalid" }` item and the rest continue.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn import_sends_each_message_and_reports_by_source_sequence() {
    let b = FakeBroker::default();
    let msgs = vec![imported(3, "a"), imported(9, "b")];
    let r = import::run(&mut b.clone(), "q", msgs, &ImportOptions::default(), &NoProgress, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.items.iter().map(|i| i.sequence_number).collect::<Vec<_>>(), [Some(3), Some(9)]);
    assert_eq!(b.sent("q").len(), 2);
}

#[tokio::test]
async fn invalid_messages_fail_alone() { /* one message with a 200-char MessageId → Failed{kind:"invalid"}, the other sent */ }

#[tokio::test]
async fn options_strip_sessions_and_dead_letter_properties() { /* keep_sessions false → sent.session_id None; DeadLetterReason dropped by default */ }
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core ops::import` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — `cargo test -p dispatched-core` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/ops/import.rs crates/dispatched-core/src/ops/mod.rs
git commit -m "Core: import and replay exported messages as a send job"
```

---

### Task 17: Core: carry-overs (entity kind resolution, case-insensitive user properties)

**Files:**
- Modify: `crates/dispatched-core/src/explorer.rs` (`resolve_kind`), `crates/dispatched-core/src/mgmt/mod.rs` (`get_entity_kind` from one ATOM GET), `crates/dispatched-core/src/filter/expr.rs`, `crates/dispatched-core/tests/fixtures/` (an ATOM topic entry fixture if none exists)

**Interfaces:**
- Produces:
  - `ManagementClient::entity_kind(&self, name: &str) -> Result<EntityKind>` (GET `/{name}`: `QueueDescription` → Queue, `TopicDescription` → Topic, empty feed → `NotFound`).
  - `Explorer::resolve_kind(&self, entity: &EntityPath) -> Result<Option<EntityKind>>` per Decision 15 (subscriptions → `Some(Subscription)` with no call; ATOM; on `Forbidden` the `peek-message` probe; never errors for "can't tell": `Ok(None)`; `NotFound` stays an error).
  - `pub(crate) fn kind_from_probe(result: &Result<Vec<PeekedMessage>>) -> Option<EntityKind>`.
  - Filter: `Field::User(name)` lookup = exact key, else first key equal ignoring ASCII case.

- [ ] **Step 1: Write the failing tests**

```rust
// mgmt/mod.rs (mockito, like the existing ATOM tests)
#[tokio::test]
async fn entity_kind_reads_the_description_type() { /* queue fixture → Queue; topic fixture → Topic; empty feed → NotFound */ }
// explorer.rs
#[test]
fn probe_results_map_to_kinds() {
    assert_eq!(kind_from_probe(&Ok(vec![])), Some(EntityKind::Queue));
    assert_eq!(kind_from_probe(&Err(Error::Protocol("com.microsoft:peek-message returned 400: cannot peek a topic".into()))), Some(EntityKind::Topic));
    for e in [Error::Forbidden { needed: None, detail: "x".into() }, Error::Throttled { retry_after: None }, Error::Network("down".into()), Error::NotFound("x".into())] {
        assert_eq!(kind_from_probe(&Err(e)), None);
    }
}
// filter/expr.rs
#[test]
fn user_property_names_match_ignoring_case_after_an_exact_match() {
    let mut m = test_message(1);
    m.application_properties.insert("OrderType".into(), PropValue::String("rush".into()));
    assert!(PropertyFilter::parse("user.ordertype = 'rush'").unwrap().matches(&m));
    m.application_properties.insert("ordertype".into(), PropValue::String("slow".into()));
    assert!(PropertyFilter::parse("user.ordertype = 'slow'").unwrap().matches(&m), "exact name wins");
    assert!(PropertyFilter::parse("user.ORDERTYPE = 'rush'").unwrap().matches(&m), "first in map order: OrderType < ordertype");
}
```

(`kind_from_probe` reads the status from `Protocol` texts of the form `… returned {status}: …` — the format `MgmtResponse::error` already produces; `Unsupported` also counts as a refusal → Topic.)

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched-core explorer:: filter:: mgmt::` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — `cargo test -p dispatched-core` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/src/explorer.rs crates/dispatched-core/src/mgmt/mod.rs crates/dispatched-core/src/filter/expr.rs crates/dispatched-core/tests/fixtures
git commit -m "Core: resolve pinned entity kinds (R23); user property names match ignoring case"
```

---

### Task 18: CLI: safety plumbing, jobs output, activity commands

**Files:**
- Create: `crates/dispatched-cli/src/safety.rs`, `crates/dispatched-cli/src/jobs.rs`, `crates/dispatched-cli/src/commands/activity.rs`
- Modify: `crates/dispatched-cli/src/main.rs` (global `MutateArgs`, `Activity` command, exit codes), `crates/dispatched-cli/src/resolve.rs` (`Resolved`), `crates/dispatched-cli/src/context.rs` (lazy activity log), `crates/dispatched-cli/src/commands/mod.rs`, every caller of `resolve::resolve`

**Interfaces:**
- Consumes: core `Gate`, `Actor`, `ProfileRef`, `ActivityLog`, `paths::open_activity_log`, `JobReport`, `JobEvent`, `Progress`, `Operator`.
- Produces:
  - `pub struct Resolved { pub namespace: Namespace, pub profile: ProfileRef }` from `resolve::resolve` (profile selection → its name and `production`; `--connection-string`/`--namespace` → `ProfileRef { name: <host>, production: false }`).
  - `#[derive(clap::Args)] pub struct MutateArgs { /// Do not ask for confirmation. #[arg(long)] pub yes: bool, /// Confirm an operation on a production profile by naming its entity (repeat for several entities). #[arg(long = "confirm-prod", value_name = "ENTITY")] pub confirm_prod: Vec<String>, /// Write the per-message result report here (JSON, or NDJSON for .ndjson/.jsonl). #[arg(long)] pub report: Option<PathBuf> }` — flattened into every mutating command.
  - `pub struct CliPolicy { yes: bool, confirm_prod: Vec<EntityPath>, interactive: bool, prompt: Box<dyn Fn(&str) -> io::Result<String> + Send + Sync> }` implementing `Policy`:
    - production: `Allow`-free: returns `ConfirmedTyped(req.target)` only if `yes` and some `confirm_prod` entry's `base()` equals `req.target` ignoring ASCII case; otherwise `Deny("{target} is on production profile {profile}: pass --yes --confirm-prod {target}")` (profile name taken from the summary's `on …`; pass it in via the request summary — the policy receives only `ConfirmRequest`, so the text names the target and flags).
    - non-production with `yes` → `Allow`; interactive (stdin and stderr are TTYs) → prints the summary and `Continue? [y/N] ` to stderr, `y`/`yes` → `Allow`; otherwise `Deny("{summary}. Pass --yes to confirm without a prompt")`.
  - `pub fn cli_gate(ctx: &Ctx, profile: ProfileRef, args: &MutateArgs) -> anyhow::Result<Gate>` (opens the activity log through `Ctx::activity()`, `Origin::Cli`).
  - `pub struct StderrProgress` (`Progress`: one line per 100 items and at the end, `"{done}/{total} …"`, only when stderr is a TTY).
  - `pub fn finish_job(report: &JobReport, format: Format, report_path: Option<&Path>) -> anyhow::Result<JobExit>` (prints `summary_line()` and a failures table, or the report JSON with `--format json`; writes the report file; `JobExit::{Ok, PartlyFailed}`), and `main` maps `PartlyFailed` → exit code 2.
  - `pub fn cancel_on_ctrl_c() -> CancellationToken` (tokio signal; second Ctrl-C exits at once).
  - `dispatched activity ls [--limit N] [--origin cli|gui|mcp] [--profile NAME]` (table: time, origin, profile, action, entity, outcome, counts, summary; `--format json` prints the entries) and `dispatched activity export --out PATH [--force]` (JSON, or NDJSON with `--format ndjson`).

- [ ] **Step 1: Write the failing tests** (unit tests in the new modules and `main.rs`)

```rust
fn req(target: &str, production: bool) -> ConfirmRequest { /* verb purge, summary "Remove 3 messages from orders/$DeadLetterQueue on prod-weu", origin Cli */ }

#[tokio::test]
async fn production_needs_yes_and_matching_confirm_prod() {
    let p = |yes, prod: &[&str]| CliPolicy::for_test(yes, prod, false, "n");
    assert!(matches!(p(true, &[]).confirm(&req("orders", true)).await, Decision::Deny(m) if m.contains("--confirm-prod orders")));
    assert!(matches!(p(false, &["orders"]).confirm(&req("orders", true)).await, Decision::Deny(_)), "--yes is also needed");
    assert!(matches!(p(true, &["other"]).confirm(&req("orders", true)).await, Decision::Deny(_)));
    assert_eq!(p(true, &["ORDERS"]).confirm(&req("orders", true)).await, Decision::ConfirmedTyped("orders".into()));
    assert_eq!(p(true, &["events/subscriptions/all"]).confirm(&req("events/Subscriptions/all", true)).await,
        Decision::ConfirmedTyped("events/Subscriptions/all".into()));
}

#[tokio::test]
async fn interactive_prompt_accepts_only_yes() {
    assert_eq!(CliPolicy::for_test(false, &[], true, "y").confirm(&req("orders", false)).await, Decision::Allow);
    assert!(matches!(CliPolicy::for_test(false, &[], true, "").confirm(&req("orders", false)).await, Decision::Deny(_)));
}

#[tokio::test]
async fn non_tty_without_yes_is_refused() {
    assert!(matches!(CliPolicy::for_test(false, &[], false, "y").confirm(&req("orders", false)).await, Decision::Deny(m) if m.contains("--yes")));
}

#[test]
fn connection_string_selections_are_never_production() { /* resolve with EMU connection string → profile.name "localhost", production false; a saved profile with production=true → production true */ }

#[test]
fn job_exit_codes() {
    assert_eq!(finish_job_for_test(&report(1, 0)), JobExit::Ok);
    assert_eq!(finish_job_for_test(&report(1, 1)), JobExit::PartlyFailed);
}

#[test]
fn activity_commands_parse() {
    assert!(Cli::try_parse_from(["dispatched", "activity", "ls", "--origin", "gui", "--limit", "5"]).is_ok());
    assert!(Cli::try_parse_from(["dispatched", "activity", "ls", "--origin", "robot"]).is_err());
    assert!(Cli::try_parse_from(["dispatched", "activity", "export"]).is_err(), "--out is required");
}

#[tokio::test]
async fn ctrl_c_cancels_the_job_token() {
    // The handler is a function of a signal future: `cancel_on(fut)`; resolving the future cancels the token.
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let token = cancel_on(async move { let _ = rx.await; });
    tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), token.cancelled()).await.unwrap();
}
```

- [ ] **Step 2: Run to verify they fail** — `df -h /System/Volumes/Data && cargo test -p dispatched` → FAIL.
- [ ] **Step 3: Implement.** `Ctx::activity()` opens `paths::open_activity_log()` lazily (like the store); tests use `ActivityLog::open_in_memory()` via a `Ctx::from_parts`-style constructor. Existing commands switch to `Resolved.namespace` with no behaviour change.
- [ ] **Step 4: Run** — `cargo test -p dispatched` → PASS; `cargo clippy -p dispatched --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-cli/src
git commit -m "CLI: confirmation policy (--yes, --confirm-prod), job output and exit codes, activity ls/export"
```

---

### Task 19: CLI: send, scheduled cancel, import, receive, deferred

**Files:**
- Create: `crates/dispatched-cli/src/commands/send.rs`, `crates/dispatched-cli/src/commands/receive.rs`
- Modify: `crates/dispatched-cli/src/main.rs`, `crates/dispatched-cli/src/commands/mod.rs`

**Interfaces:**
- Consumes: Task 18 (`MutateArgs`, `cli_gate`, `finish_job`, `StderrProgress`, `cancel_on_ctrl_c`), core `Operator`, `SendRequest`, `ReceiveRequest`, `ReceiveAction`, `Settlement`, `read_template`, `read_messages`, `parse_prop_arg`, `parse_duration`, `parse_when`, `MessageIdSource`, `BodySource`.
- Produces (clap definitions; every command flattens `MutateArgs`):
  - `send <ENTITY> [--body TEXT | --file PATH... | --template PATH] [--prop KEY=VALUE[:TYPE]]... [--message-id ID [--increment-id]] [--session-id ID] [--correlation-id ID] [--subject TEXT] [--to TEXT] [--reply-to TEXT] [--reply-to-session-id ID] [--content-type TEXT] [--partition-key KEY] [--ttl DURATION] [--schedule TIME] [--count N (1..=100000, default 1)] [--batch]`. `--body`, `--file` and `--template` conflict pairwise (template + `--file` is allowed: files replace the template body); flags override template fields; `--increment-id` requires `--message-id`. Prints the summary line and, for `--schedule`, the new sequence numbers (`scheduled cancel` takes them).
  - `scheduled cancel <ENTITY> (--seq N... | --all)` (`--all` peeks the scheduled view, up to 100,000).
  - `import <ENTITY> --file PATH [--new-message-id] [--no-sessions] [--keep-dead-letter-properties]`.
  - `receive <ENTITY> [--dlq | --transfer-dlq] [--session ID] [--top N (1..=5000, default 1)] [--wait SECS (default 5)] (--then complete|abandon|defer|dead-letter [--reason TEXT] [--description TEXT] | --delete)`; exactly one of `--then` and `--delete` is required (no default, spec §5); prints the received messages with the browse table / `peek_json` shape, then the summary.
  - `deferred ls <ENTITY> [--top N]` (= `peek --view deferred`), `deferred receive <ENTITY> --seq N... --then complete|abandon|defer|dead-letter [--reason] [--description] [--session ID]`, `deferred delete <ENTITY> --seq N... [--session ID]` (= `delete --view deferred`, implemented in Task 20 and called from here).

- [ ] **Step 1: Write the failing tests** (parsing and request-building; the network path is covered by Task 22)

```rust
#[test]
fn send_flags_build_the_request() {
    let req = send_request_from(&parse(["send", "orders", "--body", "{}", "--prop", "n=1:int", "--message-id", "m-01", "--increment-id",
        "--ttl", "2h", "--count", "3", "--content-type", "application/json"])).unwrap();
    assert_eq!(req.target, "orders");
    assert_eq!(req.count, 3);
    assert!(matches!(req.ids, MessageIdSource::Increment { ref base } if base == "m-01"));
    assert_eq!(req.template.time_to_live, Some(Duration::from_secs(7200)));
    assert_eq!(req.template.properties["n"], TypedValue::Int(1));
}

#[test]
fn send_refuses_conflicting_bodies_and_subscription_targets() {
    assert!(try_parse(["send", "q", "--body", "a", "--file", "f"]).is_err());
    assert!(send_request_from(&parse(["send", "events/subscriptions/all", "--body", "x"])).unwrap_err().to_string().contains("send to the topic"));
    assert!(try_parse(["send", "q", "--increment-id"]).is_err());
}

#[test]
fn receive_requires_an_explicit_action() {
    assert!(try_parse(["receive", "q"]).is_err(), "no default action");
    assert!(try_parse(["receive", "q", "--then", "complete", "--delete"]).is_err());
    let r = receive_request_from(&parse(["receive", "q", "--dlq", "--top", "5", "--then", "dead-letter", "--reason", "Manual"])).unwrap();
    assert_eq!(r.sub_queue, SubQueue::DeadLetter);
    assert!(matches!(r.action, ReceiveAction::Settle(Settlement::DeadLetter { ref reason, .. }) if reason == "Manual"));
    assert!(matches!(receive_request_from(&parse(["receive", "q", "--delete"])).unwrap().action, ReceiveAction::Delete));
    assert!(try_parse(["receive", "q", "--delete", "--top", "6000"]).is_err());
}

#[test]
fn scheduled_cancel_needs_seq_or_all() {
    assert!(try_parse(["scheduled", "cancel", "q"]).is_err());
    assert!(try_parse(["scheduled", "cancel", "q", "--seq", "5", "--seq", "6"]).is_ok());
    assert!(try_parse(["scheduled", "cancel", "q", "--all", "--seq", "5"]).is_err());
}

#[test]
fn schedule_time_accepts_relative_and_absolute() { /* --schedule +10m and 2030-01-01T00:00:00Z parse; "yesterday" fails naming both forms */ }
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched` → FAIL.
- [ ] **Step 3: Implement.** Each command: resolve → `Arc<Explorer>` → `Operator::new(explorer, cli_gate(..)?)` → run with `StderrProgress` and `cancel_on_ctrl_c()` → `explorer.close()` → `finish_job`. A `--count > 1` with a fixed `--message-id` prints the note `Duplicate detection may drop copies with the same MessageId; use --increment-id`.
- [ ] **Step 4: Run** — `cargo test -p dispatched` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-cli/src
git commit -m "CLI: send, schedule, scheduled cancel, import, receive and deferred commands"
```

---

### Task 20: CLI: DLQ and repair commands

**Files:**
- Create: `crates/dispatched-cli/src/commands/repair.rs`
- Modify: `crates/dispatched-cli/src/main.rs`, `crates/dispatched-cli/src/commands/mod.rs`

**Interfaces:**
- Consumes: Task 18, core `Operator::{delete, resubmit, move_messages, plan_purge, purge}`, `Explorer::dlq_summary`, `SelectSpec`, `MessageSelection`, `ResubmitSpec`, `Edits`, `PurgeScope`, `build_filter`.
- Produces:
  - Shared `#[derive(clap::Args)] struct SelectArgs { entity: String, #[arg(long)] view: Option<String>, #[arg(long)] dlq: bool, #[arg(long)] transfer_dlq: bool, #[arg(long = "seq", value_name = "N")] seqs: Vec<i64>, #[arg(long, conflicts_with = "seqs")] all: bool, /* filter flags as in ViewArgs: --filter, --body-contains, --body-regex, --body-json, --body-json-eq, --session */ #[arg(long, default_value_t = 100_000)] limit: u64 }` — one of `--seq` or `--all` required; `--all` uses the filter flags (`MessageSelection::Matching`).
  - `delete <SELECT>`, `move <SELECT> --to TARGET`, `resubmit <SELECT> [--to TARGET] [--new-message-id] [--remove-original] [--keep-dead-letter-properties] [--set-body TEXT | --set-body-file PATH] [--set-prop K=V[:T]]... [--remove-prop K]... [--set-subject T] [--set-content-type T] [--set-session-id ID] [--set-correlation-id ID] [--set-ttl DURATION]`, `purge <ENTITY>... [--scope active|dlq|transfer-dlq|all (default active)]`.
  - `dlq summary <ENTITY> [--transfer-dlq]` (table: count, reason, description, first/last sequence; JSON: `DlqSummary`), `dlq resubmit|move|delete <SELECT>` (= the top-level command with `--dlq` implied; `--view` refused), `dlq purge <ENTITY>... [--transfer-dlq]` (= `purge --scope dlq`).
  - The confirmation of `purge` prints the plan per entity before asking (`Remove 1,284 messages from orders/$DeadLetterQueue on dev`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn select_args_need_seq_or_all_and_map_views() {
    assert!(try_parse(["delete", "q"]).is_err());
    let s = select_spec_from(&parse(["dlq", "delete", "q", "--seq", "4", "--seq", "9"])).unwrap();
    assert_eq!(s.view, PeekView::DeadLetter);
    assert!(matches!(s.selection, MessageSelection::Sequences(ref v) if v == &[4, 9]));
    assert!(try_parse(["dlq", "delete", "q", "--view", "active", "--seq", "1"]).is_err());
    let s = select_spec_from(&parse(["delete", "q", "--all", "--filter", "user.kind = 'x'"])).unwrap();
    assert!(matches!(s.selection, MessageSelection::Matching { limit: 100_000, .. }));
}

#[test]
fn resubmit_edits_build() {
    let spec = resubmit_spec_from(&parse(["dlq", "resubmit", "q", "--seq", "1", "--set-prop", "retry=1:int", "--remove-prop", "DeadLetterReason",
        "--set-subject", "again", "--new-message-id", "--remove-original"])).unwrap();
    assert_eq!(spec.target, "q");
    assert!(spec.new_message_id && spec.remove_original);
    assert_eq!(spec.edits.set["retry"], TypedValue::Int(1));
    assert_eq!(spec.edits.subject, Some(Some("again".into())));
}

#[test]
fn move_needs_a_target_and_purge_takes_many_entities() {
    assert!(try_parse(["move", "q", "--dlq", "--seq", "1"]).is_err());
    assert!(try_parse(["purge", "a", "b", "c", "--scope", "all", "--yes", "--confirm-prod", "a", "--confirm-prod", "b"]).is_ok());
    assert!(try_parse(["purge", "a", "--scope", "everything"]).is_err());
}

#[test]
fn dlq_summary_table_lists_groups_by_count() { /* render a DlqSummary with two groups → header "Count  Reason  Description  First seq  Last seq", rows in order, a "Scanned 10,000 messages (capped)" note when truncated */ }
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — `cargo test -p dispatched` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-cli/src
git commit -m "CLI: resubmit, move, delete, purge and dlq summary/resubmit/move/delete/purge"
```

---

### Task 21: CLI: session state, and resolved kinds in the limited listing

**Files:**
- Create: `crates/dispatched-cli/src/commands/state.rs`
- Modify: `crates/dispatched-cli/src/main.rs` (`SessionCmd::State`), `crates/dispatched-cli/src/commands/browse.rs` (`fallback_tables` / `fallback_json` take kinds)

**Interfaces:**
- Consumes: `Operator::{session_state, set_session_state}`, `Explorer::resolve_kind`.
- Produces:
  - `session state get <ENTITY> <SESSION> [--out PATH]` (prints UTF-8 state as text, otherwise base64 with a note; `--out` writes raw bytes; `(no state)` when none; JSON: `{"session_id", "state_base64", "state_text"}`).
  - `session state set <ENTITY> <SESSION> (--value TEXT | --file PATH)` + `MutateArgs`; `session state clear <ENTITY> <SESSION>` + `MutateArgs`. State over 256 KiB is refused before any call ("session state is limited to 256 KB on Standard; use a smaller value").
  - `fallback_tables(known: &[(EntityPath, Option<EntityKind>)])` / `fallback_json` show `queue`, `topic`, `subscription` or `unknown`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn state_commands_parse() {
    assert!(try_parse(["session", "state", "get", "q", "RMA-1"]).is_ok());
    assert!(try_parse(["session", "state", "set", "q", "RMA-1"]).is_err(), "needs --value or --file");
    assert!(try_parse(["session", "state", "clear", "q", "RMA-1", "--yes"]).is_ok());
}

#[test]
fn state_is_shown_as_text_or_base64() {
    assert_eq!(render_state(Some(b"{\"step\":2}")), "{\"step\":2}");
    assert_eq!(render_state(Some(&[0xff, 0x00])), "/wA= (binary, base64)");
    assert_eq!(render_state(None), "(no state)");
}

#[test]
fn limited_listing_names_resolved_kinds() {
    let t = fallback_tables(&[(EntityPath::queue("orders"), Some(EntityKind::Queue)), (EntityPath::queue("events"), Some(EntityKind::Topic)), (EntityPath::queue("x"), None)]);
    assert!(t.contains("orders") && t.contains("queue") && t.contains("topic") && t.contains("unknown"), "{t}");
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p dispatched` → FAIL.
- [ ] **Step 3: Implement.** `ls` resolves kinds for the known entities only in the limited (no-Manage) path, at most 20 entities, sequentially.
- [ ] **Step 4: Run** — `cargo test -p dispatched` → PASS.
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-cli/src
git commit -m "CLI: session state get/set/clear; the limited listing shows resolved entity kinds"
```

---

### Task 22: The M2a emulator gate

**Files:**
- Create: `crates/dispatched-core/tests/emulator_m2a.rs`
- Modify: `testing/emulator/Config.json`, `crates/dispatched-core/Cargo.toml` (`[[test]] name = "emulator_m2a" required-features = ["test-support"]`), `.github/workflows/ci.yml` (add `--test emulator_m2a`; the "1 passed" count check becomes 4)

**Interfaces:**
- Consumes: the public core API only (`Operator`, `Gate` with `AllowAll` and `ActivityLog::open_in_memory()`, `Explorer`, `testing::*` seeding).
- Config.json additions (namespace `sbemulatorns`, all `DefaultMessageTimeToLive: PT1H`, `LockDuration: PT30S`, `MaxDeliveryCount: 10`, `DeadLetteringOnMessageExpiration: false`): queues `m2a-ops`, `m2a-target`, `m2a-sessions` (`RequiresSession: true`), `m2a-purge-a`, `m2a-purge-b`; topic `m2a-topic` with subscription `m2a-sub` (no rules = default true filter).

- [ ] **Step 1: Write the gate** — one `#[tokio::test(flavor = "multi_thread")] async fn m2a_operate()` that prints `skipped: …` and returns unless `DISPATCHED_EMULATOR=1` (same wording as the other gates), uses a per-run id in a `run` property, and filters every assertion by it. Steps, each asserting the activity log has a started + finished pair with the right outcome:
  1. **Send:** `send` 3 copies with `Increment { base: "{run}-0001" }`, typed props (`int`, `timespan`, `guid`, `datetime`), TTL 10 min, ContentType → peek shows the ids `…-0001..0003`, the property types, and `time_to_live == 600 s`.
  2. **Batch and topic:** `send` 5 with `batch: true` to `m2a-topic` → peek `m2a-topic/Subscriptions/m2a-sub` has 5 of this run.
  3. **Schedule and cancel:** schedule 2 for +30 min → 2 sequence numbers; the scheduled view shows them; `cancel_scheduled` both plus one unknown → 2 succeeded, 1 missing; the view no longer has them.
  4. **Peek-lock receive + settle:** send 4; `receive` max 1 each with `Abandon` (delivery count becomes 1), `Defer` (state deferred), `DeadLetter { "Manual", "gate" }` (in the DLQ with that reason), `Complete` (gone).
  5. **Receive-and-delete bound:** send 5; `receive` max 3 `Delete` → exactly 3 returned and exactly 2 of this run remain.
  6. **Deferred:** `settle_deferred` the deferred one with `Complete` → gone.
  7. **Renew lock:** open a receiver through the crate's test hook `testing::renew_lock_once(conn, address)` (added to `messaging/testing.rs`: receive one, renew, check the new `locked_until` is later, abandon) → passes (proves the lock-token byte order).
  8. **Delete by sequence (active, #872):** send 4; delete the 2nd and 4th → those gone; the 1st and 3rd still there with delivery count 0 (release does not raise it — if this assertion fails, stop and report: Decision 4 depends on it).
  9. **Move DLQ → target:** dead-letter 2 messages of this run (seeding helper); `move_messages` to `m2a-target` → DLQ has none of this run, target has both with their MessageIds.
  10. **Resubmit with edits, keep original:** dead-letter 1; `resubmit` to `m2a-ops` with `new_message_id` and `set` prop `retried=true` → the DLQ still has it; `m2a-ops` has a copy with a new id and `retried`.
  11. **Sessions:** send to `m2a-sessions` in sessions `{run}-A` (2) and `{run}-B` (1); `set_session_state(A, b"step-2")` → `session_state(A) == Some(b"step-2")`; clear → `None`; delete by sequence the B message (lock-and-match inside session B) → gone.
  12. **DLQ summary:** `dlq_summary(m2a-ops, DeadLetter)` has a `Manual`/`gate` group with a count ≥ 1.
  13. **Purge across entities:** first drain both purge queues of leftovers with `testing::drain`; send 3 to `m2a-purge-a`, 2 to `m2a-purge-b`, and dead-letter 1 of those in `m2a-purge-b`; `plan_purge` both with `All` → `total.bound()` is 3 for each; send 1 more to `m2a-purge-a` *after* planning; `purge` → `m2a-purge-b` is empty, `m2a-purge-a` has exactly 1 message left (the bound held) and the report has the "more remain" note.
  14. **Import:** export 2 messages of `m2a-ops` to a temp file with `write_messages`, `import` them into `m2a-target` → 2 sent, bodies equal.
  15. **R23:** `resolve_kind(m2a-ops) == Some(Queue)`, `resolve_kind(EntityPath::queue("m2a-topic")) == Some(Topic)`.
  16. **Denied path:** an `Operator` over a `DenyAll` gate: `purge` returns `Error::Denied`, the queue is untouched, the log has a `denied` entry.

- [ ] **Step 2: Ask the user**, then restart the emulator with the new Config.json: `df -h /System/Volumes/Data && testing/emulator/emulator.sh down && testing/emulator/emulator.sh up`. If the user says not now, mark the gate "not run" in the task report and continue to Task 23.
- [ ] **Step 3: Run the gate**

Run: `DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m2a -- --nocapture`
Expected: `test result: ok. 1 passed`. An emulator capability that is missing (for example session state or receive-by-sequence-number) is not papered over: the implementer reports it, and the controller rules on it (the op stays, the gate step becomes an `Unsupported` assertion, and the live-verification note records it for a cloud check).

- [ ] **Step 4: Run all gates and the full suite** — `DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1 --test emulator_m1c --test emulator_m2a` → 4 × `1 passed`; `cargo test --workspace` → PASS (no emulator).
- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/dispatched-core/tests/emulator_m2a.rs crates/dispatched-core/Cargo.toml crates/dispatched-core/src/messaging/testing.rs testing/emulator/Config.json .github/workflows/ci.yml
git commit -m "Test: M2a emulator gate exercises every mutating op"
```

---

### Task 23: Docs: spec decisions, live-verification checklist, README

**Files:**
- Modify: `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` (new "## 14. M2a decisions (2026-10-17)" listing Decisions 1–18 in the spec's voice, plus the deviation "batch send is pipelined, not atomic"), `README.md` (CLI section: the new commands, `--yes`/`--confirm-prod`, exit codes, `activity`)
- Create: `docs/live-verification-m2a.md` (same shape as `live-verification-m1c.md`: emulator results from Task 22, and an unchecked cloud checklist for M2b's live check: every gate step on a Standard and a Premium namespace with Entra and SAS, a Listen-only key (receive works, send is `forbidden` naming Send), a Send-only key, a partitioned entity (delete by sequence), the `released` delivery-count behaviour, session state on Premium, purge on a session entity with a running consumer; follow-ups: Tauri commands, Jobs drawer, Activity log UI, composer UI, template collection files (Decision 10), atomic batch)

- [ ] **Step 1: Write the three documents.** No code. Every claim about emulator behaviour comes from Task 22's actual run; anything not run says "not run".
- [ ] **Step 2: Check** — `grep -n "M2a decisions" docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` finds the section; `cargo test -p dispatched` still passes (README is not tested, nothing else changed).
- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-10-13-dispatched-v1-design.md docs/live-verification-m2a.md README.md
git commit -m "Docs: M2a decisions in the spec, live-verification checklist, CLI reference"
```

---

## Self-review notes (for the executor)

- **Spec coverage (M2 row, core + CLI):** receive and settle → 9, 12, 19; composer core → 5, 6, 7, 19; clone & resend → 6, 14 (`resubmit` without removal); DLQ resubmit/move/delete → 13, 14, 20; purge (active/DLQ/all, sessions, across entities) → 15, 20; scheduled cancel → 10, 12, 19; deferred list/receive/settle/delete → 12, 13, 19; sessions and session state → 9, 12, 21; Jobs (core model, per-message results, export, retry list) → 3, 18; Activity log (SQLite, tagged, filterable, exportable) → 2, 4, 18; production checks → 1, 4, 18; DLQ summary → 15, 20; import from file → 7, 16, 19; R23 → 17, 21; case-insensitive user properties → 17; TTL duration → 5. Desktop (Jobs drawer and Activity log UI, composer and resubmit sheets, app Tauri commands) and "CLI or MCP actions refresh the views they affected" are M2b; MCP is M4 (Decision 18).
- **Names used across tasks:** `Operation`, `Target`, `CountEstimate::bound`, `Gate::run`, `Audited`, `ActivityLog`, `MemoryAudit`, `JobRun`, `JobReport`, `ItemOutcome::{Succeeded, Failed, Skipped, Missing, NotReached}`, `MessagingBackend`, `LockReceiver`, `MessageSender`, `FakeBroker`, `ExplorerPlane`, `Operator`, `SelectSpec`, `MessageSelection`, `ResubmitSpec`, `PurgePlan`, `PurgeScope`, `ReceiveRequest`, `ReceiveAction`, `Settlement`, `SessionChoice`, `LockToken`, `OutgoingMessage`, `Edits`, `CloneOptions`, `MessageIdSource`, `BodySource`, `TypedValue`, `PropType`, `ImportedMessage`, `DlqSummary`. If a task renames one, it updates every later task's use in its report.
- **What a reviewer should push on:** that no `pub` path reaches `MessagingBackend` without `Gate`; that credit can never exceed the remaining allowance (read `CreditPlan`); that every receiver is closed and every held lock released on error and cancel paths; that activity entries hold no secret.

---

Plan complete and saved to `docs/superpowers/plans/2026-10-17-m2a-operate-core-cli.md`. Please review the plan. Does it capture what you want?
