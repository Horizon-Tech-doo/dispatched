# Dispatched v1 — Design

**Status:** Approved (revised 2026-10-13: shared kit, fe2o3-amqp) · **Date:** 2026-10-13 · **Owner:** Hamza Abdagić (Horizon Tech d.o.o.)
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
- Client layer: our own Service Bus layer built directly on `fe2o3-amqp`, plus ATOM REST for management and ARM for discovery (approach A′, section 3). This revises approach A: research showed `azure_core_amqp` 1.1.0 cannot carry the Service Bus management protocol, receive-and-delete, dead-letter reasons or WebSockets.
- No ACP host in v1.
- Shared code goes in a separate kit repo, **`Horizon-Tech-doo/horizon-azure-kit`** (crate `horizon-azure-kit`, published to crates.io). It holds the cross-product plumbing (auth, secrets, ARM, SAS, safety and jobs). Dispatched's M0 creates the kit by extracting Arkived's code. Arkived moves onto the kit in its own later plan. A shared UI package comes after Dispatched's M1 (section 2.1).
- The load-test harness is deferred until after v1.
- UI: ServiceBusExplorer is the reference for *features and workflows*, not for UI design. The UI is designed fresh, may improve on both the incumbent and Arkived, and explicitly rejects Arkived's generic "AI-slop" look. The **impeccable** and **taste** skills govern UI work (section 6).

## 2. Architecture

### 2.1 Shared kit

```
horizon-azure-kit/               Apache-2.0, Horizon-Tech-doo/horizon-azure-kit, published to crates.io
├── crates/horizon-azure-kit/    one crate, modules gated by cargo features
│   ├── auth/      Entra device code + browser (PKCE loopback), refresh-token cache, JWT claims,
│   │              cloud endpoints, `token_for(scope)` exchange (one sign-in → ARM / Storage / Service Bus)
│   ├── secrets/   CredentialStore trait, chunked OS keychain, in-memory fake for tests
│   ├── arm/       paged ARM client: subscriptions, list resources by provider type
│   ├── sas/       SAS token signing (HMAC-SHA256 over the URL-encoded audience)
│   └── safety/    Policy + ConfirmRequest + production check, CancellationToken,
│                  Job / JobEvent progress model, Audit trait + SQLite audit log
└── .github/workflows/tauri-release.yml   reusable release workflow, called by each app
```

- The kit contains only code that **both** products need. No Service Bus or Storage domain logic goes in it.
- Errors are a `KitError` enum. Each product converts it into its own error type with `From`.
- **During development**, products point at the local kit checkout through `[patch.crates-io]` (a path override, never a git dependency), so their own crates.io publishing still works.
- The kit is published as `0.1.0` before Dispatched's first crates.io release.
- **UI sharing:** once Dispatched's new design system settles (end of M1), its tokens and components are extracted into an npm package `@horizon-tech/explorer-ui`. Arkived's restyle onto it is separate work.

### 2.2 Dispatched

```
dispatched/                      Apache-2.0, Horizon-Tech-doo/dispatched
├── crates/
│   ├── dispatched-core/         all logic, no UI
│   │   ├── connstr/             Service Bus connection strings (namespace, EntityPath, emulator)
│   │   ├── profiles/            saved connections: SQLite metadata, secrets via kit `secrets`
│   │   ├── discovery/           Entra: subscriptions → Service Bus namespaces (via kit `arm`)
│   │   ├── mgmt/                ATOM REST management: entity CRUD, runtime counts, XML import/export
│   │   ├── messaging/           MessagingBackend trait + AMQP implementation
│   │   ├── ops/                 multi-step jobs (DLQ resubmit/move, purge, bulk delete, export/import, listen)
│   │   └── safety.rs            Service Bus operation kinds mapped onto kit `safety` (Policy, Job, Audit)
│   ├── dispatched-cli/          `dispatched` binary (clap)
│   └── dispatched-mcp/          MCP server library, launched by `dispatched mcp`
└── app/                         Tauri 2 + React/TypeScript + Vite
    └── src-tauri/src/commands/  one module per area; each command only translates and calls core
```

`app/src-tauri` is a **member of the root Cargo workspace**, so the repo has one `target/` (Arkived keeps two).

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

## 3. Service Bus client layer (approach A′)

No Service Bus crate is both published and usable:
- Microsoft's `azure_messaging_servicebus` in `Azure/azure-sdk-for-rust` is `publish = false` and marked "not production ready". Its management, peek decoding, lock tokens, settlement and scheduling are placeholders, so **nothing is ported from it**.
- The `azure_messaging_servicebus` 0.21 on crates.io is the deprecated 2024 REST crate.
- The community `azservicebus` crate (built on fe2o3) has not had a release since 2025-07. It is a *reference* for the protocol, not a dependency.
- `azure_core_amqp` 1.1.0 can't do enough for this tool:
  - It can't set the target of the `$management` response link.
  - Its receiver settle mode is hard-coded, so there is no receive-and-delete.
  - Dead-letter can't carry a reason, and abandon doesn't increment the delivery count.
  - No WebSockets in any published release.

So:

| Plane | Mechanism | Covers |
|---|---|---|
| Message plane | AMQP 1.0 via **`fe2o3-amqp` 0.18** with `fe2o3-amqp-cbs` (token put), `fe2o3-amqp-management` or our own request/response link pair on `{entity}/$management`, and `fe2o3-amqp-ws` (WebSockets). The Service Bus semantics live in `messaging/`. | send / batch send, schedule / cancel, peek (from a sequence number), receive (peek-lock, and receive-and-delete via a sender-settled link), complete / abandon / defer / dead-letter with reason, receive deferred by sequence number, renew lock, session receivers (`com.microsoft:session-filter`) and session state, DLQ and transfer-DLQ sub-queues, AMQP over WebSockets |
| Management plane | Service Bus **ATOM REST API**, `api-version=2024-05`. Cloud: `https://{ns}/…`. Emulator: `http://{host}:5300/…`. Auth: SAS header or Entra bearer token (scope `https://servicebus.azure.net/.default`). | create, read, update and delete of queues, topics, subscriptions, rules (SQL filter, correlation filter, SQL action), entity SAS policies; counts from `CountDetails` (active, DLQ, scheduled, transfer, transfer-DLQ) and `SizeInBytes` |
| Discovery | **ARM** via kit `arm`, `Microsoft.ServiceBus/namespaces`, `api-version=2024-01-01` | Entra: subscriptions → namespaces, with SKU tier, endpoint and `disableLocalAuth` (when it's true, the SAS option is hidden) |

- **Swappable backend.** `MessagingBackend` is a crate-internal trait. If Microsoft ships a real Service Bus crate, we evaluate swapping it in behind that trait. The public API stays the same.
- **Protocol facts the implementation must respect** (from research, 2026-10-13):
  - SAS signing uses the **raw UTF-8 key bytes**, not base64-decoded, over the lowercased, percent-encoded audience followed by `"\n"` and the expiry.
  - CBS token types: `jwt` (Entra) and `servicebus.windows.net:sastoken` (SAS). Tokens are re-put before they expire, because the broker drops links when a token expires.
  - Management requests carry an `operation=com.microsoft:<op>` application property and an AMQP-value **map** body. Responses are matched by correlation-id, with `statusCode` in the application properties. Peek returns a list *or* array of `{message: binary}` maps. Each one is decoded as a full AMQP message.
  - A message's sequence number, enqueued time, lock token and locked-until come from the `x-opt-*` annotations. Dead-letter reason and description are the application properties `DeadLetterReason` and `DeadLetterErrorDescription`.
  - ATOM: a GET for a missing entity returns **200 with an empty `<feed>`**, which must map to `NotFound`. Pages hold at most `$top=100`. Parse on local XML names, leniently, with defaults for every field.
- **Least privilege:**
  - If a key or role lacks Manage rights, the tree falls back to entities named in the connection string or added by hand.
  - Message operations still work if the key has Listen and/or Send rights. This solves #615, #707 and #723.
  - `Forbidden` errors name the right that is missing, parsed from the ATOM `<Detail>` text, for example "Manage,EntityRead claims required".
- **Emulator** (#814):
  - `UseDevelopmentEmulator=true` connection strings are supported first-class.
  - Connections use **plain AMQP** `amqp://{host}:5672`, with no TLS and no WebSockets.
  - The management API is at `http://{host}:{port or 5300}`.
  - The well-known key is `SAS_KEY_VALUE`.
  - The emulator's own limits show up as capability flags, not errors: a 1 h max TTL, at most 10 connections, configuration only through Config.json, and possibly unreliable `CountDetails`. When counts are unreliable, Dispatched counts by peeking and labels the number as "approximate".

**M0 gate (go/no-go for approach A′).** Before M1 starts, a spike must pass the same flow against **both** the emulator and a real namespace, once with Entra and once with a SAS connection string:
- list queues (ATOM) → peek messages (`$management` peek) → peek the DLQ, showing the dead-letter reason.

If `fe2o3-amqp-management` cannot carry Service Bus map bodies, use a hand-rolled sender and receiver link pair with source and target set to the reply-to address. That pattern is proven in `azservicebus`.

## 4. Feature scope (v1.0)

Incumbent issue numbers are in parentheses.

### 4.1 Connections and auth
- Entra:
  - Ways to sign in: interactive browser, device code, Azure CLI credential, environment or service principal.
  - Multiple tenants, plus cloud selection: public, China, US Gov.
- SAS connection strings at namespace level and entity level (`EntityPath`), or built from endpoint, key name and key.
- Service Bus Emulator.
- Transport: AMQP, or AMQP over WebSockets (cloud only). HTTP/HTTPS proxy settings, which apply to the WebSocket transport and to HTTPS management/ARM calls.
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

**Layout.** ServiceBusExplorer defines *which workflows* must be covered, not how the UI looks. We are free to design a better UI. The baseline below is a starting point, and the impeccable and taste process may change it wherever that gives a clearer, faster tool:
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
  - against the official **Service Bus Emulator** in Docker:
    - pinned image `mcr.microsoft.com/azure-messaging/servicebus-emulator:2.0.1` + `mcr.microsoft.com/mssql/server:2022-latest`
    - wait on `GET /health` (up to 3 min)
    - SA password without semicolons
  - run in CI on Linux, with a seeded `Config.json` (namespace `sbemulatorns`) listing the test entities
  - the mssql image is amd64-only; on Apple Silicon it runs under Rosetta and is best-effort for local runs
  - gated behind `DISPATCHED_EMULATOR=1`, so a plain `cargo test` stays hermetic
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
- **Release** (`release.yml`, triggered by an `app-v*` tag) calls the kit's reusable `tauri-release.yml` workflow, which builds:
  - Windows x64 and ARM64: NSIS + MSI
  - macOS universal: DMG, signed and notarized when secrets are present
  - Linux x86_64 and aarch64: AppImage, deb, rpm
  - Tauri updater manifest and signatures
- The CLI is published to crates.io on a `cli-v*` tag.

## 11. Milestones

Every milestone ends with a tagged release and a live-verification note.

| Milestone | Delivers |
|---|---|
| **M0 — Kit + spike + skeleton** | `horizon-azure-kit` repo created by extracting Arkived's auth, secrets, ARM, SAS and safety code (with tests). Dispatched repo, workspace, CI, emulator harness, release workflow wired up. Entra + SAS + emulator connect → list queues → peek → DLQ peek, in the CLI and in a minimal desktop window. **Go/no-go for approach A′.** |
| **M1 — Browse** | Profiles, lazy tree, search, counts and colouring, dashboard, entity Overview, all peek views, filters (property and body), viewer, export. App shell and visual system set (impeccable and taste). |
| **M2 — Operate** | Receive and settle, composer, clone & resend, DLQ resubmit/move/delete, purge, scheduled cancel, deferred, sessions and session state, Jobs drawer, Activity log, production checks. |
| **M3 — Manage** | Full entity, rule and SAS-policy CRUD; status changes; duplicate and rename; XML import/export (incumbent-compatible); listener. |
| **M4 — Agents + v1.0** | Full CLI and MCP surfaces, impeccable audit of every screen, installers for all targets, docs, README with "Migrating from ServiceBusExplorer". |

## 12. Open items (not blocking the plan)

- Code signing: Windows Authenticode certificate and Apple Developer ID. If they aren't available, M0 ships unsigned builds, as Arkived did early on.
- Whether to post in incumbent issues #286, #607 and #863 at the v1.0 launch (marketing decision, after M4).
- Event Hubs v2 scoping, after v1.0.
- Trademark: copy Arkived's `docs/trademark-compliance.md` stance ("not affiliated with Microsoft"; "Azure Service Bus" used only descriptively). Use Azure-*style* glyphs drawn by us, not Microsoft's icon set, unless its terms allow use inside an app.
