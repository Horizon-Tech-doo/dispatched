# M3b live verification — 2026-10-20

M3b is the desktop half of M3: the manage screens (entities, status, duplicate, rename, bulk delete, rules, SAS policies, ServiceBusExplorer XML export and import) and the Listener tab.

- **The emulator column** comes from the desktop gate `emulator_m3b`. It runs the app's own manage and listener paths (the functions behind the Tauri commands, the GUI policy answering typed production confirmations, and the activity log) against the local emulator, on a profile marked production.
- **The screens** are covered by Vitest against the mock engine, and by two Playwright end-to-end tests in Chromium and WebKit.
- **No cloud namespace was used.** Neither was the native window: the desktop walkthrough below is unchecked.

| Check | Emulator (SAS, production profile) | Cloud |
|---|---|---|
| Create a queue with lock 45 s, max delivery 7, TTL 30 min; the edit form reads back `45s`, `7`, `30m` | ✅ gate step 1 | not run |
| Edit one setting (max delivery 3); the others unchanged | ✅ gate step 2 | not run |
| Status to SendDisabled and back to Active | ✅ gate step 3 | not run |
| Duplicate, then rename the empty copy; confirmations name the copy, then both rename names, in order | ✅ gate step 4 | not run |
| Topic, plus a subscription created with a SQL rule; add a correlation rule, update it to SQL, remove it | ✅ gate step 5 (the rule list after each) | not run |
| Add a SAS policy | ⚠️ the job and its activity entries only (GATE-NOTE: the emulator drops policies) | not run |
| Export a queue and a topic with its subscription (3 entities), bulk-delete them in one job (3 typed confirmations), preview and import the file, read back | ✅ gate step 6 | not run |
| Listener Watch: 5 messages shown and left, not gated | ✅ gate step 7 | not run |
| Listener Complete, 2 receivers, stopped after 5: the queue is empty, and each removed message is read back with `job_message` | ✅ gate step 7 | not run |
| A started and a finished `gui` entry for every gated step, each succeeded | ✅ gate step 8 | not run |
| An edit keeps settings someone else changed after the form opened | unit test (core `update_settings_reads_inside_the_gate_and_keeps_other_settings`) | not run |
| A removing listener stops at its held cap without dropping a message | unit test (`destructive_listener_stops_at_the_held_cap`) | not run |
| Policy keys never reach the webview | unit tests (`policy_views_have_no_keys`, `policies_never_render_keys`) | not run |

**Gate runs (local):**
- `DISPATCHED_EMULATOR=1 cargo test --locked -p dispatched-app --lib emulator_m3b -- --nocapture` printed `emulator_m3b: all steps passed` on four runs (8.8 s, 7.8 s, 9.1 s, 7.8 s). Its one GATE-NOTE: `the emulator drops SAS policies; the policy read-back and removal are not checked here`. Afterwards the emulator held no `m3b-` entity.
- Every gate as CI runs them, after the last commit: the core gates `emulator_m0`, `m1`, `m1c`, `m2a` and `m3a` (four `1 passed`, one `2 passed`, no `skipped:`, `m3a_manage: all steps passed`), then `emulator_m2b: all steps passed` and `emulator_m3b: all steps passed`.

**Local checks:**
- Rust: fmt and clippy (`--workspace --all-targets --features dispatched-core/test-support -D warnings`) clean. Tests: core 684, CLI 178, app 162.
- Frontend: `tsc` and ESLint clean; Vitest 85 files and 922 tests; Playwright 76 tests (38 per browser, Chromium and WebKit).

**Design review:** done against spec §6 from 26 screenshots of the mock build (13 screens, dark and light), not with the impeccable skill (not available in the session). Fixed:
- Empty rule fields showed errors before anything was typed.
- Segmented controls stretched across the sheets.
- The duration hint was cut off.

Deferred (P3):
- The rate chart's bars sit at the right of an empty baseline for the first minute.
- In the mock, a watched message's detail says it is gone, because watched mock messages are not on the entity. This is mock only.

## Decision

M3b desktop manage and listener: **GO for the emulator.** Every manage operation and both listener kinds run through the app's own path with typed production confirmations and their activity entries. Policies beyond the add, a real ServiceBusExplorer file and the cloud are not proven here.

## Desktop walkthrough (not run; run on the emulator before the v0.x release)

Run `npm run tauri dev` in `app/`, with a connection-string profile for the emulator marked production:

- [ ] New queue, topic and subscription from the palette, from the tree's menu and from a topic's tab. Each confirmation asks for the name, and the tree shows the new entity.
- [ ] Edit settings from the Overview: only the changed settings are saved, and fixed settings are read-only. Change status from the Overview.
- [ ] Duplicate and rename from the palette: the rename's open tab follows the new name. Rename of a queue holding messages is refused with the core's text.
- [ ] Delete two entities from the Dashboard's selection, typing both names. Their tabs show "wasn't found".
- [ ] Rules tab: add a correlation rule, edit it, remove it. The empty correlation filter is pointed out (#794).
- [ ] Export the namespace to XML, delete an entity, import the file with and without "Replace existing entities".
- [ ] Listener: watch while sending from the CLI, then stop. Receive-and-delete with prefetch, then Send again from a removed message. Close the tab while listening, and the job ends.

## Cloud checklist

The M3a cloud checklist (`docs/live-verification-m3a.md`) carries over through the desktop. In addition:

- [ ] Policies: add, regenerate, remove; copy a key and a connection string, and the copied connection string sends.
- [ ] Forwarding targets picked in the form, and a cleared target, with supplementary authorization (Entra and SAS).
- [ ] Premium: max message size set on create, and shown on edit.
- [ ] A long listener under load: rates match the producer, and the held cap stops a removing listener with its note.
