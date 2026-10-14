# M1a live verification — 2026-10-14

| Check | Emulator (SAS) | Cloud + Entra | Cloud + SAS |
|---|---|---|---|
| WebSockets (AMQP over `wss://…/$servicebus/websocket/`) | n/a (unsupported, rejected before saving) | ✅ `live-ws` profile peeks active and DLQ; Service Bus accepts the `amqp` subprotocol | not run (same transport path) |
| HTTP proxy | n/a | not run (no proxy available) | not run |
| Multi-tenant discovery (`namespaces --all-tenants`) | n/a | ✅ home tenant listed with TENANT column; 3 guest tenants skipped with a warning (AADSTS50076, MFA required), exit 0 | n/a |
| Profiles (add / test / ls / rm, keychain entry removed) | ✅ unit + CLI tests | ✅ `profile test`: Manage ok, Listen ok | ✅ `profile test`: Manage ok, Listen ok; `rm` leaves no `dispatched:profile` keychain entry |
| Views | ✅ active, scheduled, deferred, DLQ (gate `emulator_m1`) | ✅ active (3), scheduled (2, state `scheduled`), DLQ (`MaxDeliveryCountExceeded`) | — |
| Property filters | ✅ gate | ✅ `=`, `LIKE`, `AND`, `NOT`, `IN`, `IS NULL`, `>=` on user and system properties | — |
| Body filters | ✅ gate | ✅ contains, regex, JSONPath equality each select the expected messages | — |
| Export | ✅ gate (NDJSON parsed) | ✅ NDJSON to stdout (schema `dispatched.message/v1`), `--out /dev/null` | — |
| Approximate counts | ✅ `~N` via peek (emulator sends no CountDetails) | exact counts from the service (3 / 1 / 2) | — |
| Token refresh and link re-attach | unit-tested | not run live (needs > 1 h idle) | — |

**Emulator gates (local, Apple Silicon):** `emulator_m0` and `emulator_m1` each `test result: ok. 1 passed`, run twice in a row against the same emulator to prove robustness to leftover state.

**Observations**
- Property names are matched case-sensitively. Messages sent through the REST API with Python's HTTP client arrived with capitalised names (`Tenant`, `Priority`), so `user.tenant = 'acme'` matched nothing while `user.Tenant = 'acme'` matched as expected.
- Guest tenants that require MFA cannot be reached with the home-tenant refresh token; they need an interactive sign-in for that tenant.
- An unsigned debug build triggers a macOS Keychain access prompt the first time it reads an item stored by a different build.
- Live test resources: resource group `rg-dispatched-m1a-test` (Basic namespace, `orders` queue with max delivery count 1, Data Owner role for the signed-in user) in the Horizon Tech subscription, created for this check and deleted after it (role assignment first).

## Decision

M1a core and CLI browse: **GO.** The same code browses, filters and exports against the emulator and a real namespace over AMQP and AMQP-over-WebSockets, with Entra and SAS, and saved profiles keep secrets only in the OS keychain.

## Follow-ups
- Interactive per-tenant sign-in for guest tenants that require MFA (and several concurrent sign-ins, M1c).
- Consider case-insensitive matching of user property names, or a hint when a filter names a property that exists with different casing.
- `profile rm` says "and its stored secret" for Entra profiles, which have none.
- Proxy path still to be verified against a real proxy; WSS trusts webpki roots while HTTPS management uses the OS store (TLS-inspecting proxies).
- Peek-lock receivers must grant one credit at a time (M2).
