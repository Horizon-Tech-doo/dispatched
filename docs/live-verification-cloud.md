# Cloud verification — 2026-10-20

These checks were run against a temporary Standard namespace, `sb-ponte-dispatched-test` (North Europe, ponte tenant, resource group `rg-dispatched-test`). Access was SAS (RootManageSharedAccessKey) and the transport AMQP over TLS. The namespace is deleted after the pass. Decisions are in spec §18.

## Gates on the cloud

Every gate ran against the namespace with `DISPATCHED_GATE_CONNECTION_STRING`. The fixture entities of `testing/emulator/Config.json` were created first with the CLI.

| Gate | Result | Notes |
|---|---|---|
| `emulator_m0` | ✅ | |
| `emulator_m1` | ✅ after a gate fix | Counts are exact (management API), not peek-approximate. |
| `emulator_m1c` | ✅ | |
| `emulator_m2a` | ✅ (2 tests) | |
| `emulator_m3a` | ✅ 17 gated steps | Metadata reads back on queue, topic and subscription. A SAS policy is added, read back and removed. The rename refusal count is exact ("holds 1 message"). |
| `emulator_m2b` (desktop) | ✅ | |
| `emulator_m3b` (desktop) | ✅ after the TLS fix | The policy reads back with Listen and Send, and is removed. |

## CLI checks

| Check | Result |
|---|---|
| TTL `2d` and auto-delete `max` on create; TTL updated to `3d`, `never` and `max` | ✅ (a read right after a create can be stale) |
| Partitioned queue: `--partitioning` refused on update; another setting updated; duplicate; rename of the copy; partitioned topic rename | ✅ after fix (spec §18.3) |
| Forwarding and forwarding of dead letters: 3 messages reach the target; `--forward-to ""` clears; a later update keeps the DLQ forward | ✅ after fix (spec §18.2) |
| Delete of a missing entity maps to `not found` | ✅ |
| `listen --delete --prefetch 20 --concurrency 2` under 5 × 20 sends, Ctrl-C: 100 printed, 100 removed, the queue empty | ✅ |
| `listen --complete --each-session`: sessions a, b and a later c taken in full (8 messages) | ✅ |
| AMQP over WebSockets (443): send, peek, receive-and-delete | ✅ |

## Bugs found and fixed

1. **The desktop panicked on its first TLS connection.** Two rustls crypto providers were linked, so neither was picked by default. The fix is in spec §18.1; the `emulator_m3b` gate failed before it and passes after.
2. **Forwarding could not be set.** The service needs absolute URLs. Fix: spec §18.2, with tests `forwarding_targets_are_written_as_absolute_urls` and `forwarding_targets_read_back_as_names`.
3. **Duplicate, rename and import of partitioned entities failed.** The size read back is 16 times the configured size. Fix: spec §18.3, with tests `create_of_a_partitioned_copy_sends_the_configured_size` and `configured_size_leaves_other_sizes_alone`.

## Not run yet

- Entra ID sign-in. It needs a data-plane role on the test namespace and an interactive `dispatched login` by the user.
- The native desktop window against the cloud (the walkthroughs in the M2b and M3b notes).
- Premium (message size, namespace partitions), a proxy, and a real ServiceBusExplorer export file.
