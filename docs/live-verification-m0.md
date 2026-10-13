# M0 live verification — 2026-10-14

| Check | Emulator (SAS) | Cloud + Entra | Cloud + SAS |
|---|---|---|---|
| Sign-in | n/a | ✅ browser PKCE + loopback, refresh token in macOS Keychain | n/a |
| Namespace discovery (ARM) | n/a | ✅ found `sbdispatchedm004ed96` (Basic, westeurope) | n/a |
| Connect (AMQP + CBS) | ✅ plain AMQP :5672 | ✅ TLS :5671, JWT put-token | ✅ TLS :5671, SAS put-token |
| List queues (ATOM) + counts | ✅ list; counts absent (emulator sends no CountDetails) | ✅ `orders` 2 active / 1 DLQ | ✅ same counts |
| Peek active | ✅ | ✅ 2 messages, seq 2–3, bodies + subject | ✅ (root key and Listen-only key) |
| Peek DLQ with reason | ✅ `m0-gate` + description | ✅ `MaxDeliveryCountExceeded` + description | ✅ |
| Empty / past-end peek is empty | ✅ | n/a | n/a |
| Missing entity | — | ✅ `error: not found: nope` | — |
| Least privilege | — | — | ✅ Listen-only: `ls` → `access denied (needs Manage,EntityRead)`, peek works; Send-only: peek → `access denied (needs Listen)` |
| Desktop window: connect → list → peek | ✅ builds and launches; Explorer calls covered by the gate test | n/a (Entra in GUI is M1) | not run (same Explorer path as CLI SAS) |

**Emulator gate (local, Apple Silicon via the arm64 compose override):** `test result: ok. 1 passed`, run 4× across Tasks 16–18 and the final fix wave. GATE-NOTE: `emulator counts for orders: None`.

**Observations**
- `delivery_count` on peek: 0 for never-delivered messages, 1 for the message dead-lettered after one delivery (cloud). The emulator reported 0 for the dead-lettered message as well.
- The Service Bus Emulator never returns `CountDetails`; counts must come from peeking on the emulator (M1, labelled approximate).
- Microsoft's `mssql/server:2022-latest` (amd64) segfaults under emulation on Apple Silicon; `testing/emulator/docker-compose.arm64.yml` (Azure SQL Edge) is selected automatically on arm64. CI uses the default amd64 file.
- Live test resources: resource group `rg-dispatched-m0-test` (Basic namespace, `orders` queue, `listen-only` / `send-only` rules, Data Owner role for the signed-in user) in the Horizon Tech subscription — created for this check, to be deleted after it.

## Decision

Approach A′ (own Service Bus layer on `fe2o3-amqp` + ATOM REST + ARM): **GO.** The same core code peeks active and dead-letter queues against both the emulator (plain AMQP, SAS) and a real namespace (TLS, Entra JWT and SAS), with correlated `$management` request/response, correct CBS audiences for sub-queues, real counts from the management API, and least-privilege errors that name the missing right. No protocol fallback (raw `fe2o3-amqp-management`, alternate reply addressing) was needed.

## Follow-ups for M1
- Token re-put and link refresh for sessions longer than one hour; reuse links per entity instead of per address where possible.
- Counts by peeking on the emulator (flagged "approximate"); honour `EntityPath` for entity-scoped keys.
- Entra sign-in, profiles, multi-tenant and sovereign clouds in CLI and desktop.
- Design system (impeccable + taste) and the real desktop shell; updater once the signing key exists.
- Remaining deferred minors from the M0 ledger (see the final review triage).
