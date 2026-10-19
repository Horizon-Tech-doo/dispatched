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
| **M4 — Agents + v1.0** | Full CLI and MCP surfaces, impeccable audit of every screen, installers for all targets, store builds (a Microsoft Store MSIX that the Store signs, and a sandboxed Mac App Store build with the updater off and no Azure CLI sign-in), docs, README with "Migrating from ServiceBusExplorer". |

## 12. Open items (not blocking the plan)

- Code signing: Windows Authenticode certificate and Apple Developer ID. If they aren't available, M0 ships unsigned builds, as Arkived did early on.
- Whether to post in incumbent issues #286, #607 and #863 at the v1.0 launch (marketing decision, after M4).
- Event Hubs v2 scoping, after v1.0.
- Trademark: copy Arkived's `docs/trademark-compliance.md` stance ("not affiliated with Microsoft"; "Azure Service Bus" used only descriptively). Use Azure-*style* glyphs drawn by us, not Microsoft's icon set, unless its terms allow use inside an app.

## 13. M1c decisions (2026-10-16)

Decided while planning M1c (`docs/superpowers/plans/2026-10-16-m1c-browse.md`) and confirmed in the live check (`docs/live-verification-m1c.md`):

1. Per-session peek is a broker-side `peek-message` with a session id and takes no session lock. If the source cannot do it (`Unsupported`, which covers every refusal except 401, 403, 404, 429 and 503), the app peeks the whole entity and keeps that session's messages client-side: always correct, only slower.
2. Session listing merges the broker's `get-message-sessions` with a peek tally of the main queue (capped at 10,000 messages). Either source may fail alone; both failing is an error. Session state needs a lock, so it moves to M2.
3. "From the end" finds the highest sequence number with one-message peeks (gallop, then bisect, at most 200 probes), then scans back in windows that grow x4 until it has Top N matches, reaches sequence 0, or has examined 100,000 messages. It returns the last N oldest first, has no "Peek more", and caps Top N at 5,000.
4. Grid rows are summaries with 160-character body previews. The detail pane fetches the full message by sequence number. Bodies over 4 MiB are truncated for display and copy is disabled for them; export writes the whole body.
5. Filters are a property expression, one body condition (contains, regex or JSONPath with an optional value) and an optional session id, all ANDed, and they run in the Rust scan.
6. Node colouring uses the five label swatches as an 8px marker on the tree row, from per-connection highlight rules (name regex, or a count at or above a threshold). Amber stays reserved for dead letters, PROD and errors.
7. The counts the tree shows, the highlight rules, the tree filter and the sub-tab an entity opens on are saved per connection.
8. Dashboard rows with dead letters are marked by the amber dead-letter number; there is no row tint. Unknown counts are blank cells in "Copy as TSV".
9. Entity URLs are `sb://{host}/{entity path}` and `.../$DeadLetterQueue`. The entity connection string is the profile's string with `EntityPath` set to the queue or topic (a subscription's string names its topic, as the portal and the Azure SDKs expect), offered for connection-string profiles only and refused when the stored string is scoped to another queue or topic; a key scoped to a topic copies that topic's subscriptions.
10. The export path comes from the native save dialog, opened by Rust, which handles "replace?". "Export all" uses the view's current filter and session; "Export selected" reads the selected sequence numbers again and reports any that have gone.
11. Concurrent Entra sign-ins were mostly built in M1b. M1c adds a Settings list of every stored sign-in (whose it is, which profiles use it) with sign-out. "Sign in again" uses M1b's "Sign in to {tenant}".
12. Overview live counts refresh every 10 s while the Overview is visible, and on F5.
13. Deviation from §4.2: there is no transfer count kind in the tree's count choice or highlight rules. Transfer messages are in flight and not browsable in M1c. Transfer counts show on Overview and the Dashboard.
14. Deviation from §4.2: a subscription's rules are a Rules sub-tab, not tree nodes. The tree keeps entities only.

Recorded after the build and the live check:

15. The "matches nothing" filter state is neutral, not amber (R22).
16. Entity kind resolution in Rust for pinned and `EntityPath` entities is deferred to M2 (R23). Until then the limited view shows them as queue rows.
17. The grid row limit is 100,000 (R14).
18. An export writes only to the path picked in the Rust-opened save dialog, through a one-time pick id. The webview never supplies a path (R8).
19. Without a data-plane role a peek fails `forbidden` while the control-plane listing works. The error advice depends on the sign-in: Entra without Listen points to "Azure Service Bus Data Receiver", without Manage to "Azure Service Bus Data Owner"; SAS points to a key whose policy includes Listen or Manage; an unknown sign-in gets neutral wording. The action is "Edit connection", not retry.
20. The emulator answers session listing with an empty list. The app then shows sessions found by peeking and says so. Whole-entity peeks on session entities were not run on a cloud namespace.

## 14. M2a decisions (2026-10-17)

Decided while planning M2a, the operate core and CLI (`docs/superpowers/plans/2026-10-17-m2a-operate-core-cli.md`), and while building it. The emulator results are in `docs/live-verification-m2a.md`. The desktop half of M2 (Tauri commands, Jobs drawer, Activity log UI, composer and resubmit sheets) is M2b.

### Safety, audit and jobs

1. Safety types live in `dispatched-core`, not the kit. The kit's `safety` has `Policy`, `ConfirmRequest`, `Decision`, `Origin`, `require` and `CancellationToken` but no operation kinds, jobs or audit log. Spec §2.1 puts the job model and the SQLite audit log in the kit, but the kit holds only code both products need and Arkived is not on it yet. M2a builds `safety.rs`, `jobs.rs` and `activity.rs` in core and maps onto the kit's `ConfirmRequest` (`verb`, `target` = the entity base, `summary` = the exact effect text, `item_count`, `reversible`, `production`, `origin`). Extraction into the kit happens when Arkived adopts them.
2. The activity log is a separate SQLite file, `activity.db`, next to `dispatched.db`, with its own `user_version` schema, WAL mode and a 5 s busy timeout, so the CLI and the desktop app can write at once. An entry left pending by a process that ended (a second Ctrl-C, a kill, a crash) is finished as `cancelled` ("interrupted before it finished") when the log is opened once it has had no finished entry for a day; younger ones are left alone, since another process may still be running them. Schema 3 records the writing process id for diagnosis only. It is pruned on open to the newest 50,000 rows and 90 days. In M2a the CLI records mutating operations only; reads are not logged. The desktop keeps its in-memory log until M2b moves it onto this file.
3. Every mutating operation goes through one gate: confirm, audit before, run, audit after. An operation that is denied is logged once, as `denied`. A run that is dropped before it finishes is logged as `cancelled` ("interrupted before it finished"). On production the gate asks the policy once per entity the operation changes (`Operation::entities()`: a move or resubmit target included), each request naming that entity, so the typed confirmation covers every one; elsewhere it asks once. A finished entry the CLI cannot write is a warning on stderr. Activity entries hold no secret.
4. Exit codes for jobs: `0` when every item succeeded, or was skipped as the command promised (`Skipped { expected: true }`: only the scheduled messages a move or resubmit leaves alone); `2` when the job ran but is incomplete (`JobExit::Incomplete`: an item failed, was not reached, was missing or was skipped otherwise, every item failed, or the job was cancelled); a purge that ran to its confirmed bound with messages left, and a receive-and-delete with the uncertainty note, exit `0` and say so in their notes; `1` for an error before the job started (bad input, a denied or refused confirmation, a production refusal, a connection failure). A purge in which every entity was denied exits 1. A report that cannot be written after the job ran is a warning and exit `2`.
5. The CLI refuses before any connection, in this order: the `--report` path (an existing file without `--force-report`, a directory, or a missing directory), the production flags, and, when there is neither `--yes` nor a terminal, the confirmation itself (`precheck_confirmable`, for every mutating command). A job must never change the broker and then fail on its output file, and an unattended run must not count, peek or lock anything it can never confirm. The refusal of a production target is logged as `denied`, under the activity kind of the command, never a default.
6. On a production profile the CLI needs `--yes` and `--confirm-prod <entity>` naming each entity; there is no prompt. A connection string or `--namespace` whose host matches a saved production profile counts as production. Reads and the MCP policy (M4) are separate: `Origin::Mcp` is logged as `mcp`, and an MCP policy can be plugged into the gate. No `dispatched-mcp` crate exists yet.
7. Every confirmation says exactly what will happen, with the count and, for specific messages, the sequence numbers (the first 20, then "and k more"). Delete, move and resubmit-with-remove add that the messages examined during the search are locked while it runs, so consumers of a live queue do not get them meanwhile. Nothing the broker does here is reversible; no confirmation offers an undo.

### Receiving and settling

8. Lock tokens of link deliveries are the 16-byte delivery tag read as a .NET GUID (`Uuid::from_bytes_le`). Tokens from `receive-by-sequence-number` are AMQP uuids taken as they are. Management requests made for a receiver (renew lock, update disposition, session state) carry `associated-link-name` and, for session receivers, `session-id`.
9. Settlements that must report an outcome (complete, abandon, defer, dead-letter) go through `com.microsoft:update-disposition` on the receiver's private management link, for link-received and sequence-number-received messages alike, because its status (410 or `message-lock-lost` becomes `lock_lost`) is the only settlement answer fe2o3 0.18 surfaces. Only `release` stays on the link; its failure is harmless because the lock expires. The emulator settled a link-received lock token this way (probe (c)).
10. Credit is bounded. Receive-and-delete never grants more credit than the remaining allowance. Peek-lock uses credit 1: one hung settlement then costs about one lock duration, and the price is about two round trips per message.
11. `--wait` is an idle timeout, in both modes: a receive ends after that long without a delivery, so it can run longer than `wait` while messages keep coming, bounded by `--top`.
12. When the wait ends the receiver drains, then reads until its credit is spent or `DRAIN_GRACE` (2 s) passes with nothing. fe2o3 never refreshes `credit()` from the broker's drain answer, so on the real link every short receive-and-delete ends on the grace with its unspent credit unconfirmed. The close settles it: it sets the link credit to 0 (a plain flow every sender must respect, so stopping does not depend on drain support), sweeps until the link is quiet (the grace time when the last receive did not wait it out), and detaches; what the sweep found is returned (Decision 13). This is not airtight: fe2o3 0.18 drops any transfer that arrives between the end of the sweep and the detach reply, so a broker that answers the drain, or the credit of 0, more than about 2 s late could still remove a delivery that nobody sees and that is not charged to a purge's allowance. The emulator stays quiet (`late_arrival_kept`); Azure is checked by the live-verification checklist, including a run with a concurrent producer. Only a close whose drain, sweep or detach did not complete adds the note "up to {n} more messages may have been removed after the wait and are not in this result", with `n` the credit still out. Purge charges its allowance with the messages received (those the close returned too) plus that uncertain credit, so a session-queue purge visits every session in one run and still never removes more than confirmed. The emulator completed the close and the link stayed quiet (probe (d), `close_uncertain: 0`). The fake broker's drain is unconfirmed by default, as on the real link.
13. A receive is cancel-safe by buffering. Deliveries are held on the receiver and returned first on the next call, so a dropped receive loses nothing between the link and the caller. An early return sets the shortfall to the outstanding credit. Closing the receiver drains, sweeps late deliveries, releases what is held and returns the receive-and-delete messages that were removed but not returned (buffered or swept); the receive records them as items and keeps them in `--report`, and purge counts them. A receive-and-delete delivery that cannot be decoded, in a receive or in the sweep, was removed all the same: it uses its credit, the receive goes on, and the report notes "n messages were removed but could not be decoded" (its sequence number is not readable). Spec §4.4 forbids an unreported removal.
14. A link error that arrives after deliveries is held until everything buffered has been handed out, then surfaced as a failure with the items kept and a non-zero exit. It is never a normal-looking end.
15. Receipts by sequence number (deferred messages) are not held. There is no link delivery to release, and abandoning through management would raise the delivery count, so callers settle deferred receipts explicitly.
16. Peek-lock receives from a dead-letter queue of a session entity use a plain receiver, not a session receiver.
17. `receive --report` keeps the full messages (the `peek --format json` shape), because a receive-and-delete is a destructive read and must offer a full copy. `receive --delete` in table mode warns on stderr. Delete, move and purge keep no copies: they remove messages the user chose and confirmed, and `export` exists to take a copy first.

### Lock-and-match: delete, move, resubmit

18. Delete, move and resubmit-with-remove find messages by lock-and-match. The wanted sequence numbers are first classified by peek: missing ones are reported `missing` at once, wanted ones beyond the examined budget are `not-reached` (`budget`), deferred ones are taken by `receive-by-sequence-number`, scheduled ones (delete only) are cancelled, and active ones are found with a peek-lock link at credit 1, acted on at once and, when they do not match, held locked until the end.
19. Held messages are released explicitly, before the link closes. AMQP `released` is a plain unlock that does not raise the delivery count, while `modified{delivery-failed}`, abandon, lock expiry and closing the link with messages still locked all raise it. This was unproven when planned, so the probe checked it first on the emulator (delivery count stays 0 after `released`, rises to 1 after abandon), and gate step 8 asserts it on the job path.
20. Held locks are renewed in `renew-lock` calls of at most 100 tokens when the earliest `locked_until` is within 10 s. Short locks are renewed at half their span with a 1 s floor. If renewing fails, everything held is released and the search stops (`not-reached`, `lock-time`). Session receivers renew the session lock. A final release that fails becomes a report note ("{n} locks could not be released; their delivery count rises by 1 when they expire"), never a silent rise.
21. Limits: 5 s without a delivery ends the search, at most 10,000 messages are examined and 2,000 held. Anything not reached is reported `not-reached` with its reason (`timeout`, `budget`, `held-limit`, `lock-time`, `cancelled`). On session entities the search runs inside the message's session. A lost lock fails only that item: "the lock expired; run again for the failed sequence numbers".
22. A matching selection (`--all` with filters) is bounded by `max_examined` as well as by `--limit`, and the confirmation says "matched n in the first m examined; run again for the rest" rather than claiming the view holds only n.
23. Move is at-least-once (§4.5): lock the source, send the copy, then complete the source. If the send fails the lock is released and the item fails. If the complete fails after the send, the item is `failed` with "sent to {target}, but the original could not be removed ({error}); it may be delivered twice". Resubmit with remove-original is the same path with edits allowed. Resubmit without it peeks the source and sends copies. The locating step returns its notes along with its items.

### Purge and the dead-letter summary

24. Purge is bounded by what was confirmed. The count comes from management `CountDetails` when available (exact), otherwise from a peek (`about N`, or `at least 10,000` when capped). Purge removes at most that many per sub-queue by receive-and-delete and says "N removed, more remain; run purge again" when messages are left. Management's active count includes deferred messages, so the `Active` bound is the active count minus the deferred count, and `All` lists deferred as its own line.
25. Deferred messages are purged by sequence number (receive, then complete). When the deferred peek hit its cap, the plan says "at least n" and the run adds "more deferred may remain; run purge again", even at an allowance of 0. Scheduled messages are never purged; the confirmation says so and points to `scheduled cancel --all`. `all` is active, deferred, dead-letter and transfer dead-letter.
26. A purge counts every entity before it purges any; a failed count stops the command with nothing changed. Each entity is then confirmed on its own, an unknown count is refused before the prompt, and the report notes each entity's activity op id. A session entity whose sessions are locked by a running consumer skips those sessions (`session-cannot-be-locked`): each is a skipped item, listed with its reason in the table output, the summary counts them, the report notes "sessions locked by another receiver were skipped; their messages remain", and the job exits `2`.
27. The dead-letter summary groups by reason and description from a peek capped at 10,000 messages, with the count and the first and last sequence number of each group.

### Sending

28. Batch send is pipelined, not atomic. `send --batch` sends over one link with up to 64 unsettled messages and reports each one's outcome. The atomic AMQP batch format (`0x80013700`) is not used in v1. This is a deviation from a strict batch.
29. Schedule goes through `$management` (`com.microsoft:schedule-message`), so the broker returns the sequence numbers that `scheduled cancel` takes. Schedule and cancel report per message, as send does: successful chunks are reported, the failing chunk and the rest carry the error, and a failure in a later chunk does not lose the sequence numbers already scheduled.
30. TTL is a duration. `PeekedMessage.time_to_live` is read from the AMQP header `ttl`, or else from `absolute-expiry-time - creation-time`, so TTLs above the 32-bit millisecond header cap survive; it is serialized as `time_to_live_ms` and the export schema is unchanged. On send, the header `ttl` is capped at `u32::MAX` and `creation-time` and `absolute-expiry-time` are set, as the Azure SDKs do.
31. Typed custom properties: `string`, `int` (32-bit), `long` (64-bit), `double`, `bool`, `datetime`, `timespan` (described `com.microsoft:timespan`, 100 ns ticks, as the .NET SDK writes it) and `guid`. The CLI syntax is `KEY=VALUE[:TYPE]`, `string` by default; a literal `:` in a string value is written `KEY=VALUE:string`. Every AMQP property type is kept end to end, with its own export tag: `bool`, `byte`, `short`, `int`, `long`, `ubyte`, `ushort`, `uint`, `ulong`, `float`, `double`, `string`, `symbol`, `timestamp`, `uuid`, `binary`, `null` and `timespan`, so a copied or replayed message keeps the types a .NET receiver casts. Nothing is widened: what has no faithful representation (AMQP `char` and the decimals, a timestamp outside years 0 to 9999, other described values such as datetime-offset and uri) decodes as `other`, which a copy cannot send (Decision 33).
32. Auto-increment MessageId counts the trailing digits up and keeps zero padding (`order-0009` becomes `order-0010`); a base without digits gets `-1`, `-2`, and so on. A fixed `--message-id` is sent N times, and the CLI notes that duplicate detection may drop the copies.
33. Clone and resend copy the body (Data sections and AMQP string values byte-exact; other AMQP-value or sequence bodies are re-encoded from JSON and the item's detail says so), every user-settable system property and the custom properties as they are: null, binary and every integer width included (exports hold binary in full as base64, and import reads it back). Only an `other` value cannot be re-encoded. A move, or a resubmit that removes the original, then fails that message and keeps its original, unless `--drop-uncopyable-properties` (core: `drop_uncopyable_properties`) sends the copy without it; a copy that keeps its original is sent without it. Either way the item's detail names the property and the job notes "k copies lost properties: …". Broker-owned values are not copied. From a dead-letter queue, `DeadLetterReason` and `DeadLetterErrorDescription` are dropped unless asked for. A new session id takes a matching partition key with it. An invalid message fails alone, by index.
34. BrokeredMessageTemplate: M2a reads one incumbent template file, JSON or XML (root `brokeredMessage` in `http://schemas.microsoft.com/servicebusexplorer`), with the incumbent's field names; property types take the .NET names and Dispatched's short names, and `timeToLive` takes .NET `TimeSpan` text. A time in the template becomes the schedule unless `--schedule` is given. The incumbent's multi-template collection files, and type names beyond these, are M2b, because the incumbent's own `MessagePropertyInfo.Type` is marked TODO and they need real sample files.
35. Importing an export file replays `dispatched.message/v1`, including the `timespan` and `long` property types, and drops broker-owned fields. `--new-message-id`, `--no-sessions` and `--keep-dead-letter-properties` control the copy.

### Sessions

36. Session state needs a session lock. Reading or changing it accepts that session with a session receiver at credit 0 (no message is locked), then closes it. If another receiver holds the session the broker refuses with `com.microsoft:session-cannot-be-locked`, which maps to `conflict` ("session X is locked by another receiver; stop that receiver or wait for its lock to expire"), and nothing changes. State above 256 KiB is refused unless `--allow-large`; on Premium the limit is then the entity's maximum message size.
37. `Explorer::session_state` is a read: no gate, no audit entry. Setting or clearing state is a gated operation.
38. The session filter is sent as a plain value, as the Azure SDKs send it: the session id under `com.microsoft:session-filter` as a plain string, or null for the next available session. The plan had sent a described value; the emulator's gateway refused the attach ("invalid filter type"), so this replaces it. The accepted session id is read back from the remote source filter. A "requires sessions" refusal on a plain receive maps to `invalid` ("{entity} requires sessions; open a session receiver").

### Carry-overs and scope

39. Entity kind resolution (R23): `Explorer::resolve_kind` returns the kind from the ATOM entry when the identity can read it. On `forbidden` it probes `peek-message` (count 1) on the entity's `$management`: success means a queue, a service refusal other than 401, 403, 404, 429 or 503 means a topic, and anything else is "unknown". Subscription paths are subscriptions without a call. The CLI's limited `ls` shows the resolved kind; the desktop half is M2b.
40. User property names in filters are matched case-insensitively (M1a carry-over): `user.X` matches the property named exactly `X` first, otherwise the first whose name equals `X` ignoring ASCII case.
41. MCP stays in M4 (§11). M2a only guarantees that an MCP policy can be plugged into the gate.
42. Parked for M2b: the next-available session receive holds the explorer lock during the accept wait; management has no single-subscription GET; lock-and-match items reach the progress sink only at the end, so the desktop has no live rows; an all-beyond-cap deferred estimate reads `AtLeast(0)` and the desktop copy should say "an unknown number of deferred messages".
