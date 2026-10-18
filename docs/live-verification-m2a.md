# M2a live verification — 2026-10-18

M2a is the operate core and the CLI. The emulator column comes from the gate `emulator_m2a` (steps 1 to 16) and the local test runs. No cloud namespace was used: the cloud columns are an unchecked list for M2b's live check. Only the checks listed as exercised were run. Everything else says "not run", "emulator only" or "unit tests".

| Check | Emulator (SAS) | Cloud + Entra | Cloud + SAS |
|---|---|---|---|
| Send, batch send to a topic, scheduled send (sequence numbers returned) | ✅ gate steps 1 to 3 | not run | not run |
| Receive: complete, abandon, defer, dead-letter (settled on the message each receive returned) | ✅ gate step 4 | not run | not run |
| Receive and delete, bounded, stops at `--top` | ✅ gate step 5 (exactly 3 returned, exactly 2 of this run left; notes `[]`, the drain did not end on the grace) | not run | not run |
| Cancel scheduled, with an unknown sequence number reported `missing` | ✅ gate step 3 (job outcome `Succeeded`) | not run | not run |
| Delete by sequence number (active): the others keep delivery count 0 | ✅ gate step 8 (Decision 19 holds on the job path) | not run | not run |
| Move from the dead-letter queue (step 9), resubmit with edits keeping the original (step 10) | ✅ gate steps 9 and 10 | not run | not run |
| Resubmit with remove-original | unit tests with the fake broker only; not in the gate | not run | not run |
| Settle a deferred message | ✅ gate step 6 | not run | not run |
| Dead-letter summary | ✅ gate step 12 (one `Manual`/`gate` group, count 1, first = last sequence number) | not run | not run |
| Session state get, set, clear, and delete inside a session | ✅ gate step 11 (session A kept both messages after the delete in session B) | not run | not run |
| Plain receive on a session queue is refused | ✅ gate: `Invalid("m2a-sessions requires sessions; open a session receiver")`, logged `Failed` | not run | not run |
| Bulk purge across entities and sub-queues | ✅ gate step 13 (`About(3)` and `About(2)`; `3 removed from m2a-purge-a; more remain, run purge again`; `m2a-purge-b` empty) | not run | not run |
| Session-queue purge in one run; short receive-and-delete without a note | ✅ CLI against the emulator (final-review fix): `purge m2a-sessions` removed 34 of 34 messages across 16 sessions in one run, no note; `receive m2a-target --delete --top 50` on 3 returned 3, notes `[]` | not run | not run |
| Export, then import with the same MessageIds; bodies equal | ✅ gate step 14 (NDJSON) | not run | not run |
| Entity kind resolution, queue and topic | ✅ gate step 15 | not run | not run |
| Denied purge: one `Skipped` item with `denied: …`, one note, queue untouched | ✅ gate step 16 | not run | not run |
| Activity log: one started and one finished entry per op, right outcome | ✅ gate (every gated step) | not run | not run |
| Typed properties (int, timespan, guid, datetime) and a 600 s TTL round-trip | ✅ gate (peek read-back) | not run | not run |
| `renew-lock` with the lock token (lock-token byte order) | ✅ gate: expiry moved later in both runs | not run | not run |
| Link semantics: `released` leaves the delivery count at 0, abandon raises it to 1 | ✅ Task 9 probe (a)/(b), twice | not run | not run |
| `update-disposition` settles a link-received lock token | ✅ Task 9 probe (c) | not run | not run |
| Drain: ends on the 2 s grace, link stays quiet afterwards | ✅ Task 9 probe (d) | not run | not run |
| Topic fan-out after a batch send | ✅ settles within a second (see GATE-NOTE) | not run | not run |
| Standard and Premium namespaces | n/a | not run | not run |
| Management counts used by the purge plan | not available on the emulator | not run | not run |
| Broker session list | not available on the emulator (empty) | not run | not run |

**Emulator gates (local):** `emulator_m0`, `emulator_m1`, `emulator_m1c` and `emulator_m2a` pass, run exactly as CI runs them (`DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1 --test emulator_m1c --test emulator_m2a -- --nocapture`). No `skipped:` line; three `1 passed` and one `2 passed`. Run twice before the review fixes and twice after (m2a: 42.6 s, 42.9 s, then 49.2 s, 48.0 s).

**Task 9 probe, verbatim (identical on every run, after the fix round):**

```
link probe: LinkProbe { released_delivery_count: 0, abandoned_delivery_count: 1, management_complete_removed: true, management_complete_error: None, drain_returned: 2, drain_credit_left: 1, left_after_drain: 0, late_arrival_kept: true, removed_unreturned: 0, close_uncertain: 0 }
```

- (a) `released` left the delivery count at 0; (b) abandon through `update-disposition` raised it to 1.
- (c) `update-disposition` with the delivery-tag token and `associated-link-name` completed a link-received message with no error, and a peek found nothing.
- (d) Asked for 3 with a 2 s wait, 2 were returned. The drain ended on `DRAIN_GRACE`: fe2o3 still reports 1 credit (`drain_credit_left: 1`). The link was quiet afterwards: a message sent later stayed in the queue (`late_arrival_kept`), and `close` returned nothing removed (`removed_unreturned: 0`) and completed its drain, sweep and detach (`close_uncertain: 0`), which settles the unconfirmed drain, so a short receive-and-delete carries no note (final-review fix I2).

**Local checks:** fmt and clippy are clean. Tests: core 584, CLI 148 plus 1 (`cargo test --locked`).

**GATE-NOTEs and emulator differences (what to compare on the cloud)**
- The topic subscription lags the batch send. Right after an accepted send of 5 to `m2a-topic`, `m2a-sub` showed 3 of 5, then 1 of 5 and 2 of 5 on later runs, and reached 5 within a second. The gate polls for additive checks and asserts the exact count once it holds. The product promises nothing about this.
- Dead-lettering lags too. In one run step 9's dead-letter queue showed 1 of 2 at first and step 10's showed 0 of 1; both settled within the polling window.
- There are no management counts on the emulator, so `plan_purge` falls back to peek counts (`About`). The management path, and "active minus deferred", are not exercised.
- The emulator's broker session list is empty (as in M1c), so listing relies on the peek tally.
- Session receivers: the gate forced a fix. The plan sent `com.microsoft:session-filter` as a described value; the emulator's gateway refused the attach ("contains invalid filter type … we found type 'E.EK'"). The Azure SDKs send a plain string, or null for the next available session. The receiver now does the same (commit `c3b5040`, Decision 38). After the fix the by-id receiver ran on the emulator in step 11 (state get, set, clear, delete inside a session). The next-available receiver and the accepted-id read back have unit tests only, no live run.
- The session-required refusal on the emulator has the condition `amqp:not-allowed` and a description containing "requires sessions", which is what the mapping expects. The cloud's wording is not checked.
- Not run on the emulator: `m2a-sessions` is not drained between runs (session ids include the run id and messages expire after 1 h).

## Decision

M2a operate core and CLI: **GO for the emulator.** Every mutating operation passes its gate step, with its activity entries, on the local emulator. The cloud checklist below is M2b's live check and nothing on it has been run.

## Cloud checklist (M2b live check, not run)

Run every gate step (1 to 16), and resubmit with remove-original, which the gate does not cover, on a Standard and on a Premium namespace, each with Entra and with SAS, then:

- [ ] A Listen-only key: receive works, send fails `forbidden` and names Send.
- [ ] A Send-only key: send works, receive fails `forbidden` and names Listen.
- [ ] A partitioned entity: delete by sequence number.
- [ ] `released` and the delivery count (Task 9 probe (a) and gate step 8 on the cloud): after a lock-and-match delete the unmatched messages still have delivery count 0. If `released` raises the count, Decision 19 must be reworked (fallback: count +1 and `max_examined` 1,000).
- [ ] `update-disposition` settles a link-received lock token (probe (c)).
- [ ] The drain grace and the close (probe (d)): on a short receive-and-delete, does the cloud complete the close and stay quiet? Pass: `close_uncertain` 0, `late_arrival_kept` true (a message sent after the drain is still in the queue after the close) and no note. Fail: any of them otherwise. `close_uncertain` 0 alone proves nothing: a transfer fe2o3 dropped during the detach looks the same.
- [ ] Concurrent producer during a purge and during a receive-and-delete (cloud): a second process sends a steady stream to the entity (say 20 messages a second) while `purge <entity> --yes --report p.json` runs, and again while `receive <entity> --delete --top 500 --wait 2 --yes --report r.json` runs. Pass: every message the producer sent is either in the report or still in the entity afterwards (compare MessageIds), and the purge removed no more than it confirmed. Fail: any message neither reported nor left.
- [ ] A session-queue purge (several sessions, each holding fewer messages than the allowance) removes every session's messages in one run with no note. Pass: one run, everything removed, no note. Fail: it stops after a session, or adds a "may have been removed" note.
- [ ] Lock renewal during a long lock-and-match search (more messages than one lock duration allows; no `lock-time` stop and no lost lock).
- [ ] Session receivers with the plain filter: by id (`session state get/set/clear`, `delete` or `receive --session`), next available (purge of a session queue), and the accepted session id read back correctly. If the cloud wanted the described form, the receiver would need a fallback; unlikely, since the SDKs send plain values.
- [ ] The "requires sessions" wording of the refusal on a plain receive from a session entity (the emulator's matches the mapping).
- [ ] Session state on Premium, including state above 256 KiB with `--allow-large` (the limit is the entity's maximum message size).
- [ ] Purge on a session entity with a running consumer: its sessions are skipped (`session-cannot-be-locked`) and the report says so.
- [ ] Purge plans use management counts, and the "active minus deferred" bound is right with deferred messages present.
- [ ] The broker session list is not empty on a session entity.
- [ ] A topic subscription shows all messages of a batch send at once, or how long it lags.
- [ ] Dead-lettering timing: does the dead-letter queue show the message at once?

## Follow-ups (M2b unless noted)

- Tauri commands for the operate core, the Jobs drawer, the Activity log UI, the composer and resubmit sheets.
- Template collection files (Decision 34), which need real sample files.
- An atomic batch send (Decision 28).
- R23's desktop half: the tree and the palette show the resolved kind; they say "queue" today.
- An all-beyond-cap deferred estimate reads `AtLeast(0)`; the desktop copy should say "an unknown number of deferred messages".
- The next-available session receive holds the explorer lock during the accept wait.
- Management has no single-subscription GET.
- Live progress for lock-and-match: found items reach the progress sink only at the end.
- MCP actions (M4).
