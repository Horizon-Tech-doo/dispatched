# Dispatched

**A fast, cross-platform explorer for Azure Service Bus. Built for humans and agents.**

Dispatched is an open-source alternative to [ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer) that runs on Windows, macOS and Linux, signs in with Microsoft Entra ID, works with the Service Bus Emulator, and (soon) exposes an MCP server so agents can inspect and operate your messaging safely.

> **Status:** M1c, desktop browse. Read-only: sign-in, profiles, a lazy entity tree, peek views, filters, detail, export, Overview, Sessions, Rules and a Dashboard, in the desktop app and (for browsing, filtering and export) the CLI. Nothing here changes or removes messages. Not ready for production use.
> Auto-update: release builds are signed and publish an updater manifest (the in-app update prompt arrives with the M1 shell).

## Try it (CLI)

```bash
cargo install --path crates/dispatched-cli   # from a checkout with ../horizon-azure-kit beside it
dispatched login                              # browser sign-in (or --device-code)
dispatched namespaces                         # --tenant <id|domain> or --all-tenants to look further
dispatched --namespace myns ls --topics
dispatched --namespace myns peek orders --dlq
```

With a connection string (SAS or emulator). Prefer the `DISPATCHED_CONNECTION_STRING` environment variable over `--connection-string`: command-line arguments are visible in shell history and process lists.

```bash
export DISPATCHED_CONNECTION_STRING='Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;'
dispatched peek orders
```

### Sign-in, profiles and selection

- `login`, `logout`, `whoami`: Entra ID sign-in. Refresh tokens live in the OS keychain.
- `namespaces [--tenant <id|domain> | --all-tenants]`: Service Bus namespaces the account can see.
- `profile ls | show | add | rm | default | pin | unpin | test`: saved connections. Metadata is in SQLite, connection strings are in the keychain. `profile add <name>` takes `--namespace <ns> [--tenant <id>]` (Entra) or `--connection-string-env VAR` / `--connection-string-stdin`, plus `--production`, `--color`, `--transport` and `--proxy`. `pin` remembers an entity to show when the identity cannot list the namespace.
- `--profile <name>` picks a saved profile. Without a flag the environment is consulted in this order: `DISPATCHED_CONNECTION_STRING`, `DISPATCHED_NAMESPACE`, `DISPATCHED_PROFILE`, then the default profile (`profile default <name>`). An explicit `--connection-string`, `--namespace` or `--profile` beats all of them.
- `--namespace` uses the signed-in account's home tenant. For a namespace in another tenant, save a profile with `--tenant`.
- Profiles and sign-in are stored in `DISPATCHED_CONFIG_DIR`, by default the platform config directory (`dispatched/dispatched.db`). A connection-string selection never reads or creates it.

### Browsing

- `ls [--topics]` lists queues (and with `--topics`, topics and subscriptions) with active, scheduled, dead-letter and transfer-dead-letter counts. `~N` is a count obtained by peeking because the service reported none, `~N+` means the peek stopped before the end, and `?` is unknown. An identity with only Listen rights cannot list the namespace; `ls` then shows the pinned and connection-string entities with `?` counts and says so.
- `peek [entity] [--top N]` reads without removing. `--view active|scheduled|deferred|dlq|transfer-dlq`, or `--dlq` / `--transfer-dlq`. The entity is `queue` or `topic/subscriptions/sub`; it defaults to the connection string's `EntityPath`, then to a profile's single pinned entity.
- `--filter` takes a property expression over `sys.` and `user.` fields: `=`, `<>`, `<`, `>`, `<=`, `>=`, `LIKE`, `IN (...)`, `IS [NOT] NULL`, combined with `AND`, `OR`, `NOT` and parentheses, e.g. `"sys.DeliveryCount > 1 AND user.kind = 'x'"`.
- Body filters: `--body-contains <text>` (case-insensitive), `--body-regex <re>`, and `--body-json <jsonpath>` with an optional `--body-json-eq <value>`.
- `--scan-limit N` caps how many messages one peek call examines, matching or not (per page for `export`). When it stops a scan early, the command prints `--from-seq <n>` to continue from. `--format json` gives machine-readable output.
- `export --out <file|-> [--top N] [--format json|ndjson] [--force]` writes peeked messages using the `dispatched.message/v1` schema (JSON array by default, or one object per line). Files are written to a temp file and renamed into place, and an existing file needs `--force`; devices and pipes (`/dev/stdout`, FIFOs) are written directly. An export whose scan stalled before confirming the end of the entity is written but exits non-zero.
- `peek --from-end [--top N]` shows the last N matching messages (oldest first) instead of the first; it has no continuation, `--top` is limited to 5000, and it examines at most 100,000 messages (or `--scan-limit` if larger) and says so when that limit, not the start of the entity, ended the search. `--session ID` on `peek` and `export` keeps only the messages of that session.
- `show <entity>` prints an entity's status, counts and properties (`--format json` for the full description); `session ls <entity>` lists a session-enabled entity's sessions with message counts (`~N+` is a lower bound); `rule ls <topic>/subscriptions/<sub>` lists a subscription's rules with their filters and actions. All are read-only.

### Connectivity

- AMQP over TCP 5671 is the default. Profiles can use AMQP over WebSockets (443) with `profile add ... --transport websockets`.
- `HTTPS_PROXY` routes a connection through an HTTP proxy and forces WebSockets. A profile can store a proxy as `--proxy http://host:port`, without credentials; it takes precedence over `HTTPS_PROXY`. For an authenticated proxy, set `HTTPS_PROXY=http://user:pass@host:port` and leave the profile's proxy unset.
- The emulator supports neither WebSockets nor a proxy.
- TLS trust: AMQP over TCP and WebSockets use the bundled webpki root store, while the HTTPS management calls (listing, counts) use the OS store. A TLS-inspecting proxy whose CA is only in the OS store may need follow-up work.

### Desktop app

The app browses the namespaces and profiles you connect, read-only. Each queue or subscription has sub-tabs for Messages, Dead-letter, Transfer DLQ, Scheduled, Deferred, Sessions (session-enabled entities) and Rules (subscriptions). Views can start at the first messages, from a sequence number, or from the end, with "Peek more" to continue. Filter by property expression, body text, regex or JSONPath, and by session. A detail pane shows the body (JSON, XML, text or hex) and every system and custom property. Export the current view or the selected rows to JSON or NDJSON; it runs as a job and writes only to the file you pick in the save dialog. The Overview shows an entity's properties and live counts. The Dashboard lists counts for every queue and subscription and can refresh on a timer. Peeks never lock, receive or settle a message.

## Develop

- Clone `horizon-azure-kit` next to this repo (shared Entra / keychain / ARM code).
- `cargo test --workspace` runs the core and CLI tests.
- Emulator gate: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1`.
  On Apple Silicon, `emulator.sh` automatically adds `docker-compose.arm64.yml` (Azure SQL Edge instead of the amd64-only SQL Server image).
- Desktop: `cd app && npm install && npm run tauri:dev`. On macOS this signs the dev build with your Developer ID or Apple Development certificate, if one is installed, so the Keychain's "Always Allow" survives rebuilds.

Design: [`docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`](docs/superpowers/specs/2026-10-13-dispatched-v1-design.md).

## License

Apache-2.0. See [NOTICE](NOTICE). Not affiliated with Microsoft.
