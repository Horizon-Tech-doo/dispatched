# M3a live verification — 2026-10-19

M3a is entity management in the core and the CLI: create, update and delete of queues, topics, subscriptions, rules and entity SAS policies; status changes; duplicate and rename; bulk delete; ServiceBusExplorer-compatible XML export and import; and the listener. The emulator column comes from the gate `emulator_m3a` (steps 1 to 12) and the local test runs. No cloud namespace was used: the cloud columns are an unchecked list. Only the checks listed as exercised were run. Everything else says "not run", "emulator only", "unit tests" or names the GATE-NOTE that skipped it.

The gate creates every entity it uses, named `m3a-{run}-…`, and a guard deletes them on every path, panics included (seen once while writing the gate: a failed step 1 left `m3a-…-q`, and the guard deleted it). After the runs below, the emulator held only the entities of `testing/emulator/Config.json`.

| Check | Emulator (SAS) | Cloud + Entra | Cloud + SAS |
|---|---|---|---|
| 1. Create a queue with non-default settings (lock 45 s, max delivery 7, TTL 30 min, dead-lettering on expiration, metadata); it reads back as set | ✅ gate step 1, status `Active`; metadata not checked (GATE-NOTE: dropped) | not run | not run |
| 2. Update two settings (max delivery 3, lock 60 s) by read-modify-write; every other setting unchanged | ✅ gate step 2 (whole `QueueSettings` compared) | not run | not run |
| 3. Status to `SendDisabled`, then back to `Active`; the other settings kept | ✅ gate step 3 | not run | not run |
| 4. Duplicate a queue: the copy has the same settings | ✅ gate step 4 | not run | not run |
| 4. Duplicate gives the copy new policy keys | unit tests only (the emulator drops policies) | not run | not run |
| 5. Rename an empty queue: the new one has the settings, the old one is `NotFound` | ✅ gate step 5 | not run | not run |
| 6. Rename of a queue holding a message is refused before the gate | ✅ gate step 6: `Conflict("… holds about 1 message; about 1 message would be lost; move them first")`; no activity entry, no target created, the source kept | not run | not run |
| 7. Topic, and a subscription created with an initial SQL rule; read back | ✅ gate step 7: topic TTL, subscription lock and max delivery, the rule's filter by name, and the rule list is `["eu"]` (no `$Default`); metadata not checked (GATE-NOTE: dropped) | not run | not run |
| 7. Add a rule to an existing subscription, then remove it | ✅ gate step 7 (the added rule reads back; after removal a GET fails) | not run | not run |
| Correlation filter with properties; an empty correlation filter refused (#794) | unit tests only (mockito, import and create) | not run | not run |
| 8. Add and remove an entity SAS policy | ⚠️ the gated add succeeds and is logged; the read-back, the rights and the removal are **not checked** (GATE-NOTE: the emulator drops policies). Unit tests cover them | not run | not run |
| 8. A policy change keeps the queue's settings | ✅ gate step 8 | not run | not run |
| Renew a key (`policy renew`); `policy keys` | unit tests only | not run | not run |
| 9. Export to incumbent XML (two queues, a topic with a subscription and its SQL rule), delete them, import, read back equal | ✅ gate step 9: no keys in the file without `include_keys`; four entities parsed and imported; queue, topic and subscription settings and the rule filter read back equal | not run | not run |
| 9. Importing the same file again skips every entity ("already exists") | ✅ gate step 9 (0 succeeded, every item skipped, logged) | not run | not run |
| Import `--overwrite`; a subscription of an existing topic | unit tests only | not run | not run |
| A real ServiceBusExplorer export file | not run (the parser test uses a fixture written from `ImportExportHelper.cs`) | not run | not run |
| 10. Bulk delete of two queues and a topic (with its subscription) in one operation | ✅ gate step 10 (three items; all three `NotFound` afterwards) | not run | not run |
| 11. Listener `Watch`: five messages shown and left, delivery count 0, not gated | ✅ gate step 11 (5 received, 0 removed, no activity entry, peek shows 5 at delivery count 0) | not run | not run |
| 11. Listener `Complete`, concurrency 2, cancelled after 5; the queue is empty | ✅ gate step 11 (5 received, 5 removed, no failure items, about 2 s; peek shows 0) | not run | not run |
| Listener `Delete` with prefetch; sessions (`--session`, each session); Ctrl-C sweep reporting | unit tests only (fake broker) | not run | not run |
| 12. One started and one finished entry for every gated step | ✅ gate: 16 gated steps, 16 started, 16 finished (each pair checked by op id, outcome `Succeeded`) | not run | not run |
| Forwarding (`ForwardTo`, `ForwardDeadLetteredMessagesTo`) with supplementary authorization | unit tests only (mockito) | not run | not run |
| Unknown elements survive an update in schema position | unit tests only | not run | not run |
| Standard and Premium namespaces; partitioned entities | n/a | not run | not run |

**Emulator gates (local):**
- `emulator_m3a` passes, run twice on its own (`DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-core --features test-support --test emulator_m3a -- --nocapture`): 19.5 s and 18.4 s. No `skipped:` line, `m3a_manage: all steps passed`.
- All core gates together, as CI runs them (`--test emulator_m0 --test emulator_m1 --test emulator_m1c --test emulator_m2a --test emulator_m3a`): four `1 passed` and one `2 passed`, no `skipped:`.
- The desktop gate `emulator_m2b` (`cargo test --locked -p dispatched-app --lib emulator_m2b`) passes: `emulator_m2b: all steps passed`.

**Local checks:** fmt and clippy (`--workspace --all-targets --features dispatched-core/test-support -D warnings`) are clean. Tests: core 662, CLI 174 plus 1, app 141 (`cargo test --locked --workspace --features dispatched-core/test-support`; the emulator binaries print `skipped:` there).

**GATE-NOTEs, verbatim (identical on both runs except the run id and tracking ids)**

```
GATE-NOTE: 1 read back: the emulator dropped UserMetadata (set Some("m3a gate 2b3361e4"), read back none); metadata not checked here
GATE-NOTE: 7 topic: the emulator dropped UserMetadata (set Some("m3a topic 2b3361e4"), read back none); metadata not checked here
GATE-NOTE: 7 subscription: the emulator dropped UserMetadata (set Some("m3a sub 2b3361e4"), read back none); metadata not checked here
GATE-NOTE: 8 the emulator accepted the policy PUT but reads back no policy (it drops SAS policies); the read-back and the removal were not checked here (mockito covers them; the cloud checklist has them)
GATE-NOTE: a DELETE of a missing queue is not NotFound on the emulator (Err(Protocol("management API returned 400: Bad Request. …"))); the service answers 404 (mockito covers the mapping)
```

**Emulator differences (what to compare on the cloud)**
- `UserMetadata` is accepted and dropped on queues, topics and subscriptions. Probed with a raw PUT outside the client as well: 201, and the read-back has no `UserMetadata` element. The gate's update step changes the lock duration instead of the metadata for this reason.
- `DefaultMessageTimeToLive` is capped at one hour: a PUT with `P2D` is refused with 400 ("The value supplied must be between 00:00:01 and 01:00:00"). The gate uses 30 and 45 minutes. The service allows up to `TimeSpan.MaxValue`.
- Entity SAS policies are accepted (200) and dropped, so nothing about policies beyond the gated call and its activity entries is checked here.
- A DELETE of a missing queue answers 400, not 404. The cleanup guard treats any failure as "already gone".
- Listing rules: the earlier probe's 500 holds only for the subscriptions of `Config.json` (`m2a-topic/m2a-sub` still answers 500). For a subscription created over the management API the listing works, so the gate checked the rule list and exported its topic through `collect_for_export`. The gate still probes both and would fall back to single-rule reads with a GATE-NOTE if the listing failed; it did not fail in either run.
- Message counts: the rename refusal saw `about 1 message`, from the peek fallback; the emulator has no management counts (as in M2a).

## Decision

M3a manage core and CLI: **GO for the emulator.** Every gated management operation, the export and import round trip, the bulk delete and the listener's `Watch` and `Complete` modes pass on the local emulator with their activity entries, twice. Policies, metadata, `Delete`-mode and session listeners, correlation rules and a real incumbent file are not proven here; they are on the cloud checklist, and nothing on it has been run.

## Cloud checklist (not run)

Run the gate's steps 1 to 12 by hand with the CLI on a Standard and on a Premium namespace, each with Entra and with SAS, then:

- [ ] Metadata: create a queue, a topic and a subscription with `--metadata`; it reads back; `--metadata ""` clears it.
- [ ] TTL and auto-delete above one hour, and `never` (`TimeSpan.MaxValue`), read back exactly.
- [ ] Partitioned entities: create a partitioned queue and topic (Standard: `--partitioning`; Premium: the namespace's partitioning), update one setting, duplicate, rename an empty one, bulk delete. `--partitioning` and `--sessions` are refused on update.
- [ ] Premium: `MaxMessageSizeInKilobytes` set on create and read back.
- [ ] Policies on a queue and on a topic: `policy add` with Send, Listen and Manage (Manage needs both); `policy ls` shows names and rights, no keys; `policy keys` prints both after the stderr warning; `policy renew` changes only that key and the old key stops working; `policy rm`; the 12-policy limit. Then send with the new Send-only key and fail to receive with it.
- [ ] A rename keeps the policy keys; a duplicate gives new ones (the old key fails on the copy).
- [ ] Rules: `rule add` with a SQL filter and an action; with a correlation filter using several system fields and user properties of typed values (int, long, bool, string); `rule ls` shows them; send matching and non-matching messages and peek the subscription. A rule update keeps the value types of unchanged properties.
- [ ] An empty correlation filter (no field, no property, or a property with an empty name) is refused on create, `rule add` and import, before any confirmation. Compare with what the service would do: the refusal says it matches no message (#794).
- [ ] Forwarding: a queue and a subscription with `--forward-to` and `--forward-dlq-to` to an entity the identity can send to (the supplementary authorization headers); messages arrive at the target; `--forward-to ""` clears it. Once with Entra, once with SAS.
- [ ] Unknown elements: update a queue that carries an element the client does not model (Premium, or a property set in the portal) and check the element is kept and the service accepts its position.
- [ ] A real ServiceBusExplorer export file: export a namespace with ServiceBusExplorer (queues, topics, subscriptions with SQL and correlation rules with properties, `$Default`, policies with and without keys), `entity import` it into an empty namespace, then `entity export` it and import the result into ServiceBusExplorer. Pass: every entity, setting and rule equal both ways; the incumbent opens the file without error.
- [ ] Import into a namespace where some entities exist: they are skipped "already exists"; with `--overwrite` they are updated and their other settings kept; a subscription of an existing topic is created.
- [ ] `--include-keys` export warns on stderr and writes the keys (file mode 0600 on unix); import with keys keeps them.
- [ ] Delete summaries use management counts: a queue with messages reads "… and its n messages", a topic counts its subscriptions and their messages; on production, `--confirm-prod` must name every entity of a bulk delete and both names of a rename.
- [ ] A DELETE of a missing entity maps to `not-found`.
- [ ] Listener, each mode, on a queue and a subscription:
  - [ ] `listen q` (Watch): new messages appear within about a second; delivery counts stay 0; nothing locked; another consumer still receives them.
  - [ ] `listen q --complete --concurrency 4` under a producer sending 20 messages a second: every message sent is printed once and the queue ends empty.
  - [ ] `listen q --delete --prefetch 50`, Ctrl-C during the stream: every removed message is printed or counted, the report notes any uncertain credit, and the count of messages sent equals printed plus left.
  - [ ] Session entity: `--session <id>` takes only that session; `--each-session` moves on when a session is quiet for 5 s and takes a session that arrives later.
  - [ ] A dropped connection during a long listen: the receiver is reported failed, the others go on, exit 2 (no reconnect, by design).

## Follow-ups

- ~~M3b: the desktop screens for every manage operation, the listener UI (rolling grid, rate chart, save or resubmit from it), the forwarding-target picker, and `RuleFilter::True` in `RulesView.tsx`.~~ Done in M3b (`docs/live-verification-m3b.md`).
- Final review: `ops/receive.rs`'s cancel path takes buffered messages back before the close, which resets the shortfall to 0, so a failed close could under-report its "may have been removed" note (the listener avoids this; see §16 Decision 21).
- A SqlFilter's `Parameters` and `CompatibilityLevel` are not written back, so a rule update drops a parametrised filter's parameters.
- Entities that forward to a renamed entity are not updated; the rename could list them.
- The listener does not reconnect; a long-running listen ends on a dropped connection.
- ~~Single-subscription GET is now in core (`get_subscription`); the desktop can use it in M3b.~~ Used by the M3b edit form.
- MCP tools for manage and listen: M4.
