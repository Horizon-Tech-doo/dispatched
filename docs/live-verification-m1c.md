# M1c live verification — 2026-10-16

The emulator column comes from the gate `emulator_m1c` and the local test runs. The Cloud + Entra column comes from a hands-on check in the real window with a signed dev build, against two read-only namespaces in two tenants. Only the checks listed as exercised were run. Everything else says "not run", "emulator only" or "mock + e2e".

| Check | Emulator (SAS) | Cloud + Entra | Cloud + SAS |
|---|---|---|---|
| Connect, tenant and namespace picker, tree | emulator gates `emulator_m0`/`emulator_m1` (connect, list) | ✅ both namespaces | not run |
| Views (active, scheduled, deferred, DLQ, transfer DLQ) | ✅ gate `emulator_m1`; browse flows in mock + e2e | not run (peek was refused, see L1) | not run |
| From the end | ✅ gate `emulator_m1c` (tail peek) | not run | not run |
| From a sequence number | ✅ gate `emulator_m1c` (selected sequences); mock + e2e | not run | not run |
| Peek more | mock + e2e | not run | not run |
| Property, body and JSONPath filters | ✅ gate `emulator_m1` | not run | not run |
| Detail pane and body formats | mock + e2e | not run | not run |
| Truncation of large bodies | mock + e2e | not run | not run |
| Export JSON / NDJSON / selected / cancel | ✅ gate `emulator_m1` (NDJSON); selected and cancel in mock + e2e | not run | not run |
| Export save dialog attached as a sheet | n/a | not run (not verified in the real window) | not run |
| Peeks never settle (nothing is received or removed) | unit tests (only `peek-message` is used) | not run | not run |
| Sessions listing source | ✅ gate `emulator_m1c` (broker listing empty, fallback to peek, see GATE-NOTE) | not run | not run |
| Whole-entity peek on a session entity | emulator only | not run | not run |
| Session-scoped peek | ✅ gate `emulator_m1c` | not run | not run |
| Rules | gate `emulator_m1c` logs the result but does not assert it; mock + e2e | not run | not run |
| Overview counts and properties | ✅ gate `emulator_m1c` (entity details; counts approximate) | ✅ Overview opened on both namespaces | not run |
| Dashboard | mock + e2e | not run | not run |
| Copy URL and connection string | mock + e2e | not run | not run |
| Draft test (test an unsaved connection) | mock + e2e | not run | not run |
| Sign-ins in Settings | mock + e2e | not run | not run |
| Keyboard (j/k, F5, Shift+J/K) | mock + e2e | not run | not run |
| 12,000-row grid | mock + e2e, and a 100,000-row unit test | not run | not run |
| From-the-end timing on a large or partitioned entity | not run | not run | not run |
| Forbidden peek state and advice | unit + e2e | ✅ shown for a missing Listen claim (L1) | not run |
| VoiceOver on detail property rows | n/a | not run | n/a |
| Keychain prompt count in the M1c flows | n/a | not run | not run |

**Emulator gates (local):** `emulator_m0`, `emulator_m1` and `emulator_m1c` each pass against the local Service Bus emulator.
- GATE-NOTE: the emulator answers the session listing with an empty list (`broker=Empty`). The app then falls back to sessions found by peeking and says: "The service listed no sessions; showing sessions found in the first N messages."
- GATE-NOTE: emulator message counts are approximate.

**Local checks:** fmt and clippy are clean. Tests: core 304, CLI 71, app 104 (Rust); vitest 775; Playwright e2e 72.

**Live setup:** two namespaces, one in each of two tenants, signed in with Entra in the real window. All calls were read-only peeks and listings.

**Observations**
- L1: without a data-plane role, peek fails with `forbidden`, while the control-plane listing still works (tree and Overview load). The service text has this shape: `Unauthorized access. 'Listen' claim(s) are required to perform this operation. Resource: 'sb://<ns>.servicebus.windows.net/<entity>/$management'`.
  - Fix shipped (commits 115cebd to eac7e74): the advice now depends on the sign-in. Entra without Listen points to the "Azure Service Bus Data Receiver" role. Entra without Manage points to "Azure Service Bus Data Owner". SAS points to a key whose policy includes Listen or Manage. An unknown sign-in gets neutral wording. The action is "Edit connection" (no retry) in messages, Overview, Rules, Sessions and Dashboard.
  - The "Azure Service Bus Data Receiver" role was then assigned at namespace scope on one of the two namespaces, at the user's request. Browsing needs a data-plane role, not only a reader role on the control plane.
- Toolbar no longer clips at 640 px.
- The browser sign-in prompt callback now requires `https`.
- On Windows, "Show in Explorer" runs `explorer.exe` from `%WINDIR%` with `/select,"<path>"`. Not verified on a Windows machine.
- The user continued after the forbidden-peek fix and reported no further defects.

## Decision

M1c desktop browse: **GO.** Browsing, filters, detail, export, Overview, Sessions, Rules and the Dashboard pass on the emulator and in tests. A real Entra connection lists, shows the Overview, and explains a missing role. The items marked "not run" are listed below as follow-ups.

## Follow-ups
- Run the full browse flow on a cloud namespace with a data-plane role, with Entra and with SAS.
- Not verified live: the export save dialog as a sheet, VoiceOver on the detail property rows, the Keychain prompt count with the signed dev build in the M1c flows, the 12,000-row grid and from-the-end timing on a large or partitioned cloud entity, a whole-entity peek on a cloud session entity.
- R23 (M2): resolve the entity kind in Rust for pinned and `EntityPath` entities. The limited view shows them as queue rows and the palette labels them "queue".
- Session state viewing and editing (M2, needs a session lock).
- TTL is stored as `u32` in core, which caps very long TTLs (M2).
- Carry-over from M1a, still open: case-insensitive matching of user property names in filters.
