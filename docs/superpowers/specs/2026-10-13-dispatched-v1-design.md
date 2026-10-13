# Dispatched v1 — Design

**Status:** Approved design, pending spec review · **Date:** 2026-10-13 · **Owner:** Hamza Abdagić (Horizon Tech d.o.o.)
**Source idea:** Notion → Ideas → "Service Bus Explorer — Rust/Tauri" (and its sub-pages)
**Reference product:** [paolosalvatori/ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer), release 6.3.1

## 1. Intent

Dispatched is a fast, cross-platform, open-source replacement for ServiceBusExplorer. It does for Azure Service Bus what Arkived does for Azure Storage.

The incumbent is a good tool built on a dead stack:
- WinForms on .NET Framework 4.7.2, so Windows only (#286 has been open for 8 years).
- The UI thread blocks (#754).
- Every setting goes through modal dialogs.
- It doesn't work with the Service Bus Emulator (#814).
- It needs Manage rights to connect (#615).
- There is no story for agents or scripting.

Dispatched keeps what users like (the explorer workflow, DLQ repair and resubmit, the composer, the dashboard) and rebuilds it on Rust and Tauri, with an MCP surface so agents can operate Service Bus safely.

**Success for v1.0:**
- Practical parity with the incumbent's *must-have* features (section 4).
- The open issues listed in section 4 are solved.
- Signed installers ship for Windows (x64 and ARM64), macOS (universal) and Linux (x64 and aarch64).
- The CLI is on crates.io and `dispatched mcp` works as an MCP server.

**Decided during brainstorming:**
- Standalone sibling to Arkived (its own repo, name and release cycle), not a module inside Arkived.
- Service Bus only in v1. Event Hubs is the first v2 candidate.
- Name: **Dispatched**. The crates `dispatched`, `dispatched-core`, `dispatched-mcp` and `dispatched-cli` were all free when checked.
- Client layer: our own Service Bus layer on Microsoft's official `azure_core_amqp` (approach A, section 3).
- No ACP host in v1.
- No code shared with Arkived for now. Patterns and code are copied and adapted. A shared `horizon-azure-*` crate is reconsidered once two real users of it exist.
- The load-test harness is deferred until after v1.
- The UI takes Arkived's *structure* and ServiceBusExplorer's *layout*, with Arkived's generic "AI-slop" look explicitly rejected. The **impeccable** and **taste** skills govern UI work (section 6).

## 2. Architecture

```
dispatched/                      Apache-2.0, Horizon-Tech-doo/dispatched
├── crates/
│   ├── dispatched-core/         all logic, no UI
│   │   ├── auth/                Entra (browser, device code, Azure CLI, env/service principal),
│   │   │                        SAS connection strings (namespace and entity level), emulator
│   │   ├── profiles/            saved connections: SQLite metadata, secrets in the OS keychain
│   │   ├── arm/                 Entra discovery: tenants → subscriptions → namespaces
│   │   ├── mgmt/                ATOM REST management: entity CRUD, runtime counts, XML import/export
│   │   ├── messaging/           MessagingBackend trait + AMQP implementation
│   │   ├── ops/                 multi-step jobs (DLQ resubmit/move, purge, bulk delete, export/import, listen)
│   │   ├── policy/              Policy trait: destructive-op confirmation, production checks
│   │   └── audit/               activity log tagged with origin (gui | cli | mcp)
│   ├── dispatched-cli/          `dispatched` binary (clap)
│   └── dispatched-mcp/          MCP server library, launched by `dispatched mcp`
└── app/                         Tauri 2 + React/TypeScript + Vite
    └── src-tauri/src/commands/  one module per area; each command only translates and calls core
```

Principles:

1. **The core decides and the surfaces only translate.**
   - Every behaviour lives in `dispatched-core` (one `dlq::resubmit`, one `purge`, and so on).
   - The GUI, CLI and MCP surfaces differ only in how they parse input, render output, and implement the `Policy` trait (modal or sheet, terminal prompt, MCP elicitation).
2. **Long-running work is a job.**
   - A job has an ID, a `Stream<JobEvent>` of progress, partial results and a final summary, and a `CancellationToken`.
   - Jobs never stop at the first failure. They record outcomes per message.
3. **Peek is the default.** No surface makes a destructive receive the implicit default, and the GUI never remembers it.
4. **No god files.** Arkived's Tauri layer grew into one 7k-line `commands.rs`. In Dispatched, the Tauri commands are split by area and contain no business logic. Core modules stay small and focused.
5. **One read/write gate.** Core labels every operation as `Read` or `Mutating(kind)`. Mutating ops call `Policy::confirm(ConfirmRequest)` before touching the broker. Production profiles add a second, typed confirmation.

## 3. Service Bus client layer (approach A)

No Service Bus crate is both published and maintained:
- Microsoft's new `azure_messaging_servicebus` lives in `Azure/azure-sdk-for-rust` with `publish = false` and the status "0.1.0 Unreleased".
- The 0.21 on crates.io is the deprecated 2024 crate.
- The community `azservicebus` crate has not had a release since 2025-07.

So:

| Plane | Mechanism | Covers |
|---|---|---|
| Message plane | AMQP 1.0 via **`azure_core_amqp` 1.x** (official, stable). Service Bus semantics (management link `$management`, CBS auth, settlement, sessions) implemented in `messaging/`. Code is ported from Microsoft's unpublished MIT-licensed crate where useful, with attribution in `NOTICE`. | send / batch send, schedule / cancel, peek (from a sequence number), receive (peek-lock / receive-and-delete), complete / abandon / defer / dead-letter, receive deferred by sequence number, renew lock, sessions (accept, state get/set), DLQ and transfer-DLQ sub-queues, AMQP over WebSockets + proxy |
| Management plane | Service Bus **ATOM REST API** (`https://{ns}/{entity}?api-version=…`). Works with SAS, Entra and the emulator. | create, read, update and delete of queues, topics, subscriptions, rules (SQL filter, correlation filter, SQL action), entity SAS policies; runtime counts (active, DLQ, scheduled, transfer, transfer-DLQ, size) |
| Discovery | **ARM** (copied from Arkived's `arm.rs`) | Entra: list tenants, subscriptions, namespaces and their tier |

- `MessagingBackend` is a crate-internal trait. When Microsoft publishes the official crate, we evaluate swapping it in behind that trait. The public API stays the same.
- **Least privilege:**
  - If a key or role lacks Manage rights, the tree falls back to entities named in the connection string or added by hand.
  - Message operations still work if the key has Listen and/or Send rights. This solves #615, #707 and #723.
  - `Forbidden` errors name the right that is missing.
- **Emulator:**
  - `UseDevelopmentEmulator=true` connection strings are supported first-class: plain AMQP on localhost and emulator management endpoints (#814).
  - The emulator's own limits (no management API on some versions) show up as clear capability flags, not as errors.

**M0 gate (go/no-go for approach A).** Before M1 starts, a spike must work against **both** the emulator and a real namespace:
- Entra sign-in → list queues → peek messages → peek DLQ with reasons.
- The same flow with a SAS connection string.

If `azure_core_amqp` cannot carry the `$management` request/response pattern with map bodies, fall back to `fe2o3-amqp` directly for that link only.

## 4. Feature scope (v1.0)

Incumbent issue numbers are in parentheses.

### 4.1 Connections and auth
- Entra:
  - Ways to sign in: interactive browser, device code, Azure CLI credential, environment or service principal.
  - Multiple tenants, plus cloud selection: public, China, US Gov.
- SAS connection strings at namespace level and entity level (`EntityPath`), or built from endpoint, key name and key.
- Service Bus Emulator.
- Transport: AMQP or AMQP over WebSockets. HTTP/HTTPS proxy settings.
- Saved profiles:
  - name, colour, **production flag**, per-profile tree filters (#706), default entity view
  - secrets only in the OS keychain
  - "Test connection" reports which auth path was used and which rights are effective.

### 4.2 Explorer
- Lazy tree: Namespace → Queues / Topics → Subscriptions → Rules. Children load on expand (#482).
- Live tree search with Ctrl+F and debounce (#861).
- Counts on nodes (choose which counts to show).
- Node colouring by count thresholds (active, DLQ, scheduled, transfer, transfer-DLQ) and by name regex.
- **Dashboard:**
  - Active, DLQ, scheduled and transfer-DLQ counts per queue and subscription.
  - Sortable, auto-refresh (off / 30 s / 1 min / 5 min), rows with DLQ > 0 highlighted.
  - Copy as TSV (#862).
- Copy an entity's URL, its DLQ URL, or an entity-scoped connection string. Refresh with F5.

### 4.3 Entity management
- Create, read, update and delete with **every** property of:
  - **queues:** max size, lock duration, TTL, auto-delete-on-idle, duplicate-detection window, max delivery count, forwarding, flags, user metadata
  - **topics**
  - **subscriptions:** including the initial rule at create time
  - **rules:** SQL filter, correlation filter (all system properties + user properties), SQL action
  - **entity SAS policies**
- Status changes: Active, Disabled, SendDisabled, ReceiveDisabled.
- Forwarding targets chosen from a picker over the tree.
- Duplicate (queue, subscription). Rename (queue, topic) is done as create-copy then delete-original, as a job.
- Bulk delete, with a typed confirmation.
- XML import and export of a namespace or selected entities, **compatible with the incumbent's export format** so users can migrate. Rules include correlation-filter properties (#812).
- Validation catches known traps, for example warning when a default correlation filter matches nothing (#794).

### 4.4 Message browsing
- Views per entity: **Active**, **Dead-letter**, **Transfer DLQ**, **Scheduled**, **Deferred** (#662).
- Peek options:
  - Top N
  - from a sequence number
  - from the end (#656)
  - per session
  - "peek more" pages
- Receive-and-delete (Top N): an explicit, mutating, confirmed action. It can never consume more than the number of messages shown (#590).
- Filters run on fetched messages:
  - on system and custom properties (SQL-like expressions)
  - on **body content**: substring, regex, JSON path (#659, #870)
- Virtualised grid (must stay smooth at 10,000+ rows) with configurable, sortable columns and row numbers.
- Detail pane:
  - Body viewer with auto-detection and pretty-printing for JSON, XML and text, plus hex.
  - All system properties, plus custom properties with their types.
  - Copy body, copy as JSON.
- Export selected messages, or every message in a view, to JSON (body + properties) or NDJSON. Import messages from file to send or replay them.
- Delete individual active messages (#872), as a confirmed job.

### 4.5 Dead-letter queue and repair
- **Resubmit:**
  - One or many messages, to the source entity or any chosen queue or topic.
  - Optional editing before sending: body, system properties, custom properties with their types (#646), new MessageId.
  - Choice to remove the original from the source, including for non-DLQ entities (#771).
- **Move** and **delete** selected messages by sequence number. This works in the DLQ and the transfer DLQ (#768). It uses a lock-and-match strategy with clear timeouts and results for each message.
- **Purge:** active, DLQ, or all, including session-enabled entities. Bulk purge across selected entities (#703).
- **Cancel scheduled messages** (#746).
- **Deferred messages:** list, receive and settle, or delete (#662).
- **DLQ summary:** dead-letter reasons and descriptions grouped and counted.
- All of the above run as cancellable jobs with progress and a per-message result report that can be exported.
- **Move is at-least-once:** send to the target, then complete the source. This is documented in the UI. Transactional moves (#725) are a v1.x stretch goal.

### 4.6 Sessions
- List sessions, including empty sessions where the broker allows it (#636).
- Peek or receive within one session.
- **View, edit or clear session state** (#854).

### 4.7 Composer
- Body: text, file(s) sent round-robin, or a JSON or XML template in the incumbent's `BrokeredMessageTemplate` format, for compatibility.
- System properties: MessageId (with optional auto-increment), SessionId, CorrelationId, Subject (Label), To, ReplyTo, ReplyToSessionId, TTL, ScheduledEnqueueTime, and a free-text ContentType with presets (#884).
- Typed custom properties: string, int, long, double, bool, datetime, timespan, guid.
- Send, schedule, send N copies, batch send.
- **Clone & resend** from any message, in any view.

### 4.8 Listener
- Live receive stream for a queue or subscription, with a session-handler variant.
- Peek-lock with an auto-complete toggle, max concurrent calls, prefetch.
- Rolling grid, messages/sec and bytes/sec chart.
- Save or resubmit from the grid.

### 4.9 Operations panel and settings
- **Jobs drawer:** running and finished jobs with progress, cancel and result report.
- **Activity log:**
  - Every action, tagged GUI, CLI or MCP, with profile, entity, counts and outcome.
  - Persisted in SQLite, filterable, exportable.
  - Actions taken by the CLI or MCP refresh the views they affected.
- **Settings:**
  - theme (dark and light, #299)
  - default peek size, timeouts, retry policy
  - body encoding defaults
  - log verbosity
- The app is sharp on high-DPI screens (#440). Tauri handles this natively.

### 4.10 Out of scope for v1
- **Deferred (v1.x / v2):**
  - load-test harness (multiple senders and receivers, think time, latency charts)
  - generator and inspector plugins (WASM or scripts)
  - Event Hubs, Event Grid namespaces
  - in-app AI assistant
  - MCP live message streaming
  - ACP host
  - transactional moves
  - Homebrew and winget packages
- **Dropped:** Notification Hubs, Relay/WCF, the IoT Hub listener, NetMessaging transport, management-certificate Metrics, Windows Server Service Bus / ACS, SDI/MDI windows.

## 5. Safety model

- Operations are classified in core as `Read`, `Mutating::Send`, `Mutating::Settle`, `Mutating::Delete`, `Mutating::Purge`, `Mutating::EntityChange` or `Mutating::SessionState`.
- `Policy::confirm(ConfirmRequest { op, profile, entity, count_estimate, is_production })` is called before any mutating op touches the broker.
- Confirmation text names the exact effect, for example "Remove 1,284 messages from orders/$DeadLetterQueue on prod-weu".
- Production profiles (see the sketch after this list):
  - **GUI:** type the entity name.
  - **CLI:** `--yes` plus `--confirm-prod <entity>`.
  - **MCP:** an elicitation per call, and never auto-approved.
- Receive-and-delete is never remembered as a default. Purge is never one click.
- Every mutating op writes an audit entry before it starts and another when it finishes.

## 6. Desktop app

**Layout** (taken from ServiceBusExplorer, structure from Arkived):
- **Top bar:**
  - connection selector with a status indicator
  - **PROD** badge on production profiles
  - command palette on Ctrl/⌘+K
- **Left:** namespace tree with live filter, count badges and a context menu. The menu has the same actions as the palette.
- **Centre:** tabs, one per opened entity, plus the Dashboard. Entity sub-tabs:
  - Overview (properties + live counts)
  - Messages · Dead-letter · Transfer DLQ · Scheduled · Deferred
  - Sessions
  - Rules (subscriptions only)
  - Listener
- **Messages view:**
  - toolbar: Peek more · Receive ▾ · Resubmit · Move · Delete · Export · Filter
  - virtualised grid + detail pane
- **Side sheets, not modals,** for the Composer, Resubmit/Edit, entity create and edit, and filters. The list stays visible. This replaces the incumbent's modal dialogs and SDI/MDI windows.
- **Bottom drawer:** Jobs and Activity log.
- **Keyboard first:**
  - palette commands for every action
  - j/k and arrow navigation in grids
  - F5 to refresh, Ctrl+F for tree search
  - Delete opens a confirmation, never deletes directly

**Visual direction.** These are requirements; Arkived's look was rejected as "AI slop":
- No gradients, glows, glassmorphism, decorative cards, pill-shaped everything, decorative emoji or icon confetti.
- One restrained accent colour. Colour carries meaning only: DLQ warning, PROD, errors, selection.
- High information density, tabular numerals for counts and sequence numbers, aligned grids.
- A system UI font plus a monospace font for bodies, IDs and sequence numbers.
- Azure-style entity glyphs for namespace, queue, topic, subscription and rule (#749).
- First-class dark **and** light themes.
- **Process:** every UI task in the implementation plan runs the **impeccable** skill and the **taste** (design-taste-frontend) skill. A screen is only done after an impeccable review pass.

## 7. CLI and MCP

**CLI (`dispatched`):**
- Commands:
  - `login`, `connect`, `profile ls|add|rm|set-prod`
  - `ls [entity]`, `show <entity>`, `counts`
  - `peek <entity> [--dlq|--transfer-dlq|--deferred|--scheduled] [--top N] [--from-seq N] [--from-end] [--session ID] [--filter EXPR] [--body-filter EXPR]`
  - `send <entity> [--body|--file|--template] [--prop k=v:type] [--schedule TIME] [--count N]`
  - `dlq summary|resubmit|move|delete|purge`
  - `scheduled cancel`, `deferred ls|delete`
  - `entity create|update|delete|export|import`, `rule ls|add|rm`
  - `session ls|state get|set|clear`
  - `listen <entity>`, `mcp [--read-only]`
- Output: `--format table|json|ndjson`. JSON is stable and documented.
- Destructive commands prompt for confirmation unless given `--yes`. Production profiles also need `--confirm-prod`.

**MCP (`dispatched mcp`, stdio):**
- **Read tools, always allowed:**
  - `list_profiles`, `list_entities`, `get_entity`, `get_counts`
  - `peek_messages`, `search_messages` (property and body filters)
  - `get_dlq_summary`
  - `list_sessions`, `get_session_state`
  - `diagnose_subscription`: checks rules, forwarding, status, lock duration and delivery counts, and explains why messages aren't arriving
- **Write tools:**
  - `send_message`, `schedule_message`, `cancel_scheduled`
  - `resubmit_dlq`, `move_messages`, `delete_messages`, `purge`
  - `entity_create`, `entity_update`, `entity_delete`
  - `set_session_state`
  - Every write call goes through `Policy` via MCP elicitation.
- `--read-only` removes the write tools from the tool list.
- Every call is written to the shared Activity log.

## 8. Errors

- Core error enum:
  - `Auth`, `Forbidden { needed: Right }`
  - `NotFound`, `Conflict`
  - `Throttled { retry_after }`, `LockLost`, `Timeout`, `Cancelled`
  - `Unsupported { capability }` (for example, emulator limits)
  - `Protocol`
- Surfaces map these to user-facing messages that say *what to do*: which right is missing, which property to change.
- AMQP connections:
  - one cached connection per profile
  - automatic reconnect with exponential backoff and jitter
  - links are re-established transparently for idempotent reads
- Throttling follows `retry_after`. Retry settings are configurable.
- A bulk job reports `{succeeded, failed: [{seq, reason}], skipped}`. The failures can be exported and retried.

## 9. Testing

- **Unit tests (core):**
  - a fake `MessagingBackend`
  - recorded ATOM XML fixtures for the management API
  - SAS signing and connection-string parsing tests (ported from Arkived)
- **Integration tests:**
  - against the official **Service Bus Emulator** in Docker
  - run in CI on Linux, with a seeded config file listing the test entities
  - cover peek, receive, settle, DLQ flows, sessions, scheduled and deferred messages, and purge
- **Live checklist (manual, per milestone):**
  - a real namespace with Entra and with SAS
  - Premium, partitioned entities and sessions
  - a key with only Listen or only Send rights
  - written up in `docs/live-verification-*.md`, like Arkived's
- **UI:** Playwright smoke tests against `vite dev` with a mocked Tauri invoke layer, covering the main flows (connect, browse, peek, resubmit, purge confirmation).
- **Disk hygiene (hard rule, from the user's CLAUDE.md):**
  - builds only in the repo's own `target/`
  - check free space before heavy builds (stop if under 30 GB)
  - clean up Docker images and caches created by a session
  - this rule is repeated in every plan task that builds

## 10. CI and release

- **CI** (`ci.yml`):
  - fmt, clippy (`-D warnings`), unit tests on ubuntu, macos and windows
  - emulator integration tests on ubuntu
  - frontend typecheck and build
- **Release** (`release.yml`, adapted from Arkived, triggered by an `app-v*` tag) builds:
  - Windows x64 and ARM64: NSIS + MSI
  - macOS universal: DMG, signed and notarized when secrets are present
  - Linux x86_64 and aarch64: AppImage, deb, rpm
  - Tauri updater manifest and signatures
- The CLI is published to crates.io on a `cli-v*` tag.

## 11. Milestones

Every milestone ends with a tagged release and a live-verification note.

| Milestone | Delivers |
|---|---|
| **M0 — Spike + skeleton** | Repo, workspace, CI, release workflow wired up. Entra + SAS + emulator connect → list queues → peek → DLQ peek. **Go/no-go for approach A.** |
| **M1 — Browse** | Profiles, lazy tree, search, counts and colouring, dashboard, entity Overview, all peek views, filters (property and body), viewer, export. App shell and visual system set (impeccable and taste). |
| **M2 — Operate** | Receive and settle, composer, clone & resend, DLQ resubmit/move/delete, purge, scheduled cancel, deferred, sessions and session state, Jobs drawer, Activity log, production checks. |
| **M3 — Manage** | Full entity, rule and SAS-policy CRUD; status changes; duplicate and rename; XML import/export (incumbent-compatible); listener. |
| **M4 — Agents + v1.0** | Full CLI and MCP surfaces, impeccable audit of every screen, installers for all targets, docs, README with "Migrating from ServiceBusExplorer". |

## 12. Open items (not blocking the plan)

- Code signing: Windows Authenticode certificate and Apple Developer ID. If they aren't available, M0 ships unsigned builds, as Arkived did early on.
- Whether to post in incumbent issues #286, #607 and #863 at the v1.0 launch (marketing decision, after M4).
- Event Hubs v2 scoping, after v1.0.
- Trademark: copy Arkived's `docs/trademark-compliance.md` stance ("not affiliated with Microsoft"; "Azure Service Bus" used only descriptively). Use Azure-*style* glyphs drawn by us, not Microsoft's icon set, unless its terms allow use inside an app.
