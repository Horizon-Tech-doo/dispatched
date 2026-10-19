# Dispatched

**A fast, cross-platform explorer for Azure Service Bus. Built for humans and agents.**

Dispatched is an open-source alternative to [ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer) that runs on Windows, macOS and Linux, signs in with Microsoft Entra ID, works with the Service Bus Emulator, and (soon) exposes an MCP server so agents can inspect and operate your messaging safely.

> **Status:** M2a, operate core and CLI. The CLI can now send, receive, settle, delete, move, resubmit, purge and manage sessions, with confirmations and an activity log. The desktop app is still read-only (M1c browse); its operate screens are M2b. Not ready for production use.
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
- `export --out <file|-> [--top N] [--format json|ndjson] [--force]` writes peeked messages using the `dispatched.message/v1` schema (JSON array by default, or one object per line). Custom property values carry a type tag: `null`, `bool`, `byte` (8-bit), `short` (16-bit), `int` (32-bit), `long` (64-bit), `ubyte`, `ushort`, `uint` (32-bit), `ulong` (64-bit), `float`, `double`, `string`, `symbol`, `timestamp`, `timespan` (an ISO 8601 duration such as `PT1H`, from a .NET `TimeSpan`), `uuid`, `binary` (base64, in full) or `other` (a value Dispatched cannot represent exactly, such as a char, a decimal, datetime-offset or uri). `import` reads the same tags. Copies (move, resubmit, import) keep every value as it is, null, binary and integer width included, except `other`, which cannot be sent again: a move, or a resubmit with `--remove-original`, fails that message and keeps its original unless `--drop-uncopyable-properties` is given, and a job whose copies lost properties notes "k copies lost properties: …". Files are written to a temp file and renamed into place, and an existing file needs `--force`; devices and pipes (`/dev/stdout`, FIFOs) are written directly. An export whose scan stalled before confirming the end of the entity is written but exits non-zero.
- `peek --from-end [--top N]` shows the last N matching messages (oldest first) instead of the first; it has no continuation, `--top` is limited to 5000, and it examines at most 100,000 messages (or `--scan-limit` if larger) and says so when that limit, not the start of the entity, ended the search. `--session ID` on `peek` and `export` keeps only the messages of that session.
- `show <entity>` prints an entity's status, counts and properties (`--format json` for the full description); `session ls <entity>` lists a session-enabled entity's sessions with message counts (`~N+` is a lower bound); `rule ls <topic>/subscriptions/<sub>` lists a subscription's rules with their filters and actions. All are read-only.

### Operating on messages

Everything below changes a queue, a topic or a message. Each command counts or finds what it will touch, shows the exact effect, and asks before it acts. Nothing the broker does here can be undone: `export` first if you want a copy (see "Copies" below).

- `send <entity>` sends to a queue or topic. The body is `--body`, or `--file` (several files are used round-robin), or comes from `--template` (a ServiceBusExplorer BrokeredMessageTemplate file, JSON or XML; flags override its fields). Message flags: `--message-id` (with `--increment-id`, `order-0009` becomes `order-0010`), `--session-id`, `--correlation-id`, `--subject`, `--to`, `--reply-to`, `--reply-to-session-id`, `--content-type`, `--partition-key`, `--ttl` (`90s`, `1h30m`, ISO 8601 or .NET `TimeSpan` text). `--prop KEY=VALUE[:TYPE]` adds a custom property; the type is `string` (default), `int`, `long`, `double`, `bool`, `datetime`, `timespan` or `guid`. `--count N` (1 to 100,000) repeats the message and `--batch` sends them pipelined over one link (not atomic). `--schedule +10m` or an RFC 3339 time schedules instead of sending and prints the sequence numbers that `scheduled cancel` takes.
- `scheduled cancel <entity> --seq N[,N...] | --all` cancels scheduled messages.
- `import <entity> --file <export file>` sends the messages of a `dispatched.message/v1` file (JSON or NDJSON) again. `--new-message-id`, `--no-sessions` and `--keep-dead-letter-properties` shape the copies.
- `receive <entity> --then complete|abandon|defer|dead-letter | --delete` receives and settles. `--then` peek-locks each message and settles it that way (`--reason` and `--description` for `dead-letter`); `--delete` removes each message as it is delivered. `--top N` (1 to 5,000, default 1), `--dlq`, `--transfer-dlq`, `--session ID`. `--wait SECS` (default 5) is an idle timeout: the receive stops after that long without a message, so it can run longer while messages keep coming. A receive-and-delete that stops short returns every message it removed, including any that arrived while its link was closing; only when the link could not be closed cleanly does it add the note "up to n more messages may have been removed after the wait and are not in this result".
- `deferred ls <entity>`, `deferred receive <entity> --seq N --then ...` and `deferred delete <entity> --seq N` list, settle and delete deferred messages. `deferred receive` also takes `--session` and `--dlq`.
- `delete`, `move` and `resubmit` take `<entity>` and either `--seq N[,N...]` or `--all` with filters (`--filter`, `--body-contains`, `--body-regex`, `--body-json` with `--body-json-eq`, `--session`, `--limit` up to 100,000, which is also the peek the confirmation counts from). The view is `--view active|scheduled|deferred|dlq|transfer-dlq`, or `--dlq` / `--transfer-dlq`.
  - `delete` removes messages (scheduled ones are cancelled).
  - `move --to <target>` sends each message to another queue or topic, then removes the original. It is at-least-once: if the removal fails after the send, the item says the message may be delivered twice. A message with a custom property whose type cannot be copied (`other`) fails and stays, unless `--drop-uncopyable-properties` (also on `resubmit --remove-original`).
  - `resubmit` sends edited copies (to `--to`, by default the queue itself or a subscription's topic). `--remove-original` removes each original after its copy is sent, like `move` with edits. Edits: `--set-body`, `--set-body-file`, `--set-prop KEY=VALUE[:TYPE]`, `--remove-prop`, `--set-subject`, `--set-content-type`, `--set-session-id` (a PartitionKey equal to the old session id follows it), `--set-reply-to-session-id`, `--set-partition-key`, `--set-correlation-id`, `--set-ttl`, `--new-message-id`. From a dead-letter queue, `DeadLetterReason` and `DeadLetterErrorDescription` are dropped unless `--keep-dead-letter-properties`.
  - **Locking:** delete, move and resubmit with `--remove-original` find messages by locking them. While the search runs, the messages it has examined are locked, so consumers of a live queue do not get them until the search ends. The confirmation says so. The search ends after 5 s without a message, 10,000 messages examined or 2,000 held; whatever it did not reach is reported `not-reached` with the reason, and a selection that is cut off says "matched n in the first m examined; run again for the rest".
- `purge <entity>... [--scope active|dlq|transfer-dlq|all]` removes messages from one or more queues or subscriptions. Each entity is counted, then confirmed and purged on its own. It removes at most the confirmed number and says "more remain; run purge again" if there are more. Scheduled messages are never purged (`scheduled cancel --all`); `all` is active, deferred, dead-letter and transfer dead-letter.
- `dlq summary <entity> [--transfer-dlq]` groups dead-lettered messages by reason and description. `dlq resubmit`, `dlq move` and `dlq delete` are `resubmit`, `move` and `delete` on the dead-letter queue (same flags, `--transfer-dlq` for the transfer queue); `dlq purge` is `purge --scope dlq`.
- `session state get|set|clear <entity> <session>` reads, replaces or clears a session's state. `get --out <file|->` writes the raw bytes. `set` takes `--value` or `--file`, and refuses state over 256 KiB unless `--allow-large` (Premium). Changing state needs the session's lock: if a consumer holds the session, the command fails with `conflict` and changes nothing. `set` and `clear` take `--yes` and `--confirm-prod` but no `--report` (there are no per-message results).
- The `ls` listing of a limited identity now shows each pinned entity's resolved kind (queue or topic) instead of assuming a queue.

#### Confirmation, production and exit codes

- Every changing command (including `send`) shows its exact effect and asks first (`Continue? [y/N]`). `--yes` skips the question and prints what was confirmed.
- Without `--yes` and without a terminal (stdin and stderr), the command is refused before it connects, counts or peeks anything.
- Ctrl-C stops a running job between messages and reports what it did ("cancelled; n not attempted"); a second Ctrl-C ends the process at once. During `receive --delete` the second Ctrl-C first writes the `--report` file with the messages removed so far (or says how many were removed and not kept). An activity entry left pending this way (no finished entry for over a day) is finished as `cancelled` ("interrupted before it finished") when the log is next opened.
- On a profile marked production, or a connection string or `--namespace` whose host matches a production profile, `--yes` alone is not enough: name each entity with `--confirm-prod <entity>` (repeat for several), the target of a `move` or `resubmit` included. There is no prompt on production. The flags are checked before any connection is made, and a refusal is written to the activity log as `denied`.
- `--report <file>` writes the per-message results (JSON, or NDJSON for `.ndjson` and `.jsonl`); `--force-report` replaces an existing file. The path is checked before anything runs, so a job never changes the broker and then fails on its output file, and the file is written before anything is printed, so a closed pipe (`| head`) cannot lose it. A mutating command whose stdout is closed warns on stderr and exits `2`. `--format json` prints the result as JSON.
- Exit codes: `0` when every item succeeded, or was skipped as the command promised (scheduled messages that `move` and `resubmit` leave alone); `1` for an error or a refusal before the job started (bad input, denied or unconfirmed, production flags missing, connection failure); `2` when the job ran but is incomplete (an item failed, was not reached, was missing or was skipped, such as a sequence number already gone or a session locked by another receiver; or the job was cancelled; or its result or report could not be written). A purge in which every entity is denied exits `1`. Two kinds of bounded work still exit `0`, so check their notes: a purge that removed what it confirmed but found more ("more remain, run purge again"), and a receive-and-delete that carries the note "up to n more messages may have been removed". The table output lists every item that did not succeed with its reason, and the summary line counts skipped and missing items.
- A progress line goes to stderr only when stderr is a terminal.

#### Copies

- `receive --report` keeps the full messages (the same shape as `peek --format json`), because a receive-and-delete is a destructive read. In table mode, `receive --delete` warns on stderr that the messages are not shown in full.
- `delete`, `move` and `purge` keep no copy of the messages they remove. Run `export` first (`export --view dlq --out backup.json`).

### Activity log

Every changing command writes to a local activity log (`activity.db`, next to `dispatched.db`): when it started, when it finished, who (`cli`, `gui` or `mcp`), which profile and entity, the effect text, the outcome and per-job notes. It holds no connection strings, tokens or message bodies. The CLI logs changes only, not reads. The log keeps the newest 50,000 entries and 90 days.

- `activity ls [--limit N] [--origin cli|gui|mcp]` lists recent entries, newest first. The global `--profile NAME` shows only that profile's entries.
- `activity export --out <file> [--force]` writes every retained entry. The format is JSON by default, or `--format ndjson`.

### Connectivity

- AMQP over TCP 5671 is the default. Profiles can use AMQP over WebSockets (443) with `profile add ... --transport websockets`.
- `HTTPS_PROXY` routes a connection through an HTTP proxy and forces WebSockets. A profile can store a proxy as `--proxy http://host:port`, without credentials; it takes precedence over `HTTPS_PROXY`. For an authenticated proxy, set `HTTPS_PROXY=http://user:pass@host:port` and leave the profile's proxy unset.
- The emulator supports neither WebSockets nor a proxy.
- TLS trust: AMQP over TCP and WebSockets use the bundled webpki root store, while the HTTPS management calls (listing, counts) use the OS store. A TLS-inspecting proxy whose CA is only in the OS store may need follow-up work.

### Desktop app

The app browses and operates on the namespaces you connect. Each queue or subscription has sub-tabs for Messages, Dead-letter, Transfer DLQ, Scheduled, Deferred, Sessions (session-enabled entities) and Rules (subscriptions). Views can start at the first messages, from a sequence number, or from the end, with "Peek more" to continue. Filter by property expression, body text, regex or JSONPath, and by session. A detail pane shows the body (JSON, XML, text or hex) and every system and custom property. Export the current view or the selected rows to JSON or NDJSON; it runs as a job and writes only to the file you pick in the save dialog. The Overview shows an entity's properties and live counts. The Dashboard lists counts for every queue and subscription and can refresh on a timer. Peeks never lock, receive or settle a message.

Operating on messages works as in the CLI, through the same safety gate and the same activity log:

- **Send** from the composer: text, files sent round-robin, or an incumbent `BrokeredMessageTemplate`. You can set every system property and typed custom properties, an auto-increment MessageId, a schedule, N copies, or a batch. **Clone & resend** opens the composer from any message, and **Import** replays an export file.
- From a view's toolbar:
  - **Receive**: peek-lock with complete, abandon, defer or dead-letter, or receive-and-delete.
  - **Delete**, **Move** and **Resubmit**, the last with edits, a new MessageId and an option to remove the original.
  - **Cancel scheduled** in the Scheduled view and **Settle** in the Deferred view.
  - A dead-letter **Summary**, grouped by reason, in the dead-letter views.

  Each acts on the selected rows, or on every message matching the view's filter.
- **Purge** one entity from the tree, or several from the Dashboard. It counts first, shows what each entity will lose, and confirms each entity.
- **Session state**: view it as text or hex, copy it, save it to a file, replace it, or clear it.
- Every change is confirmed in the sheet that started it, with the exact effect. On a production profile, you type each entity's name, including a move's target. The Delete key opens a confirmation and never deletes directly. Receive-and-delete is never pre-selected.
- Each operation runs as a job in the Jobs drawer, with progress, cancel and a per-message report you can save. The Activity tab shows the shared `activity.db`, including changes made by the CLI, and refreshes the views those changes touched.

## Develop

- Clone `horizon-azure-kit` next to this repo (shared Entra / keychain / ARM code).
- `cargo test --workspace` runs the core and CLI tests.
- Emulator gate: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1 --test emulator_m1c --test emulator_m2a`.
  On Apple Silicon, `emulator.sh` automatically adds `docker-compose.arm64.yml` (Azure SQL Edge instead of the amd64-only SQL Server image).
- Desktop: `cd app && npm install && npm run tauri:dev`. On macOS this signs the dev build with your Developer ID or Apple Development certificate, if one is installed, so the Keychain's "Always Allow" survives rebuilds.

Design: [`docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`](docs/superpowers/specs/2026-10-13-dispatched-v1-design.md).

## License

Apache-2.0. See [NOTICE](NOTICE). Not affiliated with Microsoft.
