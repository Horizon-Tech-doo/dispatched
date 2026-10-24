# Dispatched for agents (MCP)

`dispatched mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server over stdin and stdout. An agent can browse a Service Bus namespace, find out why messages are not arriving, and, when you confirm it, change messages and entities. Every change goes through the same safety gate and activity log as the CLI and the desktop app.

```bash
dispatched mcp                       # read and write tools
dispatched mcp --read-only           # read tools only
dispatched --profile dev mcp         # the namespace of a saved profile
```

## Which namespace

The server picks its namespace once, at start, the way every other command does: `--profile`, `--namespace` or `--connection-string`, then `DISPATCHED_CONNECTION_STRING`, `DISPATCHED_NAMESPACE`, `DISPATCHED_PROFILE`, then the default profile (`dispatched profile default <name>`). If none applies, the server still starts, and each tool call needs a `profile`.

Every tool also takes an optional `profile` argument, the name of another saved profile (see `list_profiles`). A tool never takes a connection string, so the agent can only reach namespaces you have saved, and secrets stay in your OS keychain. Prefer a saved profile over a connection string in a client's config file, where it would be stored as plain text.

## Tools

Entities are named `queue`, `topic` or `topic/subscriptions/subscription`. Results are JSON; where the CLI has a `--format json` shape, the tool returns the same one.

### Read tools (always listed)

| Tool | What it returns |
| --- | --- |
| `list_profiles` | Saved profiles: name, host, sign-in kind, emulator, production, default. No secrets. |
| `list_entities` | Queues with counts; with `topics: true`, topics and their subscriptions (as `dispatched ls --topics`). |
| `get_entity` | An entity's properties and counts (as `dispatched show`). |
| `get_counts` | Counts of a queue or subscription, or of each subscription of a topic. |
| `peek_messages` | Messages of `sub_queue` (`active`, `dlq`, `transfer_dlq`, `scheduled`, `deferred`): `top` (default 50, at most 500), `from_sequence`, `session`, and the filters `filter`, `body_contains`, `body_regex`, `body_json`, `body_json_eq` of `dispatched peek`. |
| `search_messages` | The same with at least one filter required, examining up to `scan_limit` messages (default 10,000, at most 100,000). |
| `get_dlq_summary` | Dead-lettered messages grouped by reason (as `dispatched dlq summary`). |
| `list_sessions` | Sessions with message counts. |
| `get_session_state` | One session's state. |
| `diagnose_subscription` | Plain-sentence findings (`problem`, `warning`, `info`) on why messages may not arrive at a subscription, and the facts they rest on (see below). |

Peeks never lock, receive or settle a message. Bodies come back as `{kind, size, text, truncated}`: the text as the CLI decodes it, or `base64` for binary that is not UTF-8. A body longer than 64 KiB is cut there and marked `truncated: true`; session state is returned the same way.

`diagnose_subscription` looks at the topic's and the subscription's status, forwarding (`ForwardTo`, `ForwardDeadLetteredMessagesTo`), the rules (none, a filter that is never true such as `1=0`, an empty correlation filter, and every other filter, listed), active and dead-letter counts with the dead-letter reasons, `MaxDeliveryCount` and `LockDuration`, the default time to live and dead-lettering on expiry, and whether sessions are required.

### Write tools (left out with `--read-only`)

| Tool | What it does |
| --- | --- |
| `send_message` | Send 1 to 100 copies of a message (body, system properties, `KEY=VALUE[:TYPE]` properties, time to live). |
| `schedule_message` | The same, enqueued `at` a time (`+10m` or RFC 3339). The report holds the new sequence numbers. |
| `cancel_scheduled` | Cancel scheduled messages by sequence number (at most 1,000). |
| `resubmit_dlq` | Send copies of dead-lettered messages back, optionally removing the originals. |
| `move_messages` | Move messages to another queue or topic (sent, then removed). |
| `delete_messages` | Delete messages. |
| `purge` | Remove every message of one entity in a scope (`active`, `dlq`, `transfer_dlq`, `all`), counted first. |
| `entity_create`, `entity_update`, `entity_delete` | Create, change (only the given settings) or delete a queue, topic or subscription. |
| `set_session_state` | Replace or clear a session's state. |

Delete, move and resubmit act on `sequence_numbers` (at most 1,000), or on the messages matching filters, with a required `max` (at most 1,000). Each returns the job report of the CLI's `--format json`: `succeeded`, `failed` (`[{seq, reason}]`), `skipped` and one item per message or entity.

The listener, rules and SAS policies have no MCP tools yet; use the CLI or the app.

## Safety

- **Every change is confirmed by you, in your MCP client.** Before anything changes, the server asks through an [elicitation](https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation) form showing the exact effect, the same sentence the CLI and the app show, for example "Remove 1,284 messages from orders/$DeadLetterQueue on dev", with the count and "This can't be undone." Ticking the box lets it run; declining, cancelling or leaving it unticked changes nothing.
- **Production needs the name typed.** On a profile marked production (`dispatched profile add ... --production`, or in the app), or any connection to the host of one, the form asks you to type the entity name, once per entity the change touches (a move's target included). The agent cannot mark or unmark production, and it is read from your saved profiles at each call.
- **No form, no change.** If your client cannot show elicitation forms, every change is refused with "this MCP client cannot ask you to confirm changes; use the Dispatched app or the CLI". Nothing is ever allowed without you. Whether a client supports elicitation depends on the client and its version; check its documentation.
- **Read-only mode.** `--read-only` leaves every write tool out of the tool list, so the agent cannot even ask.
- **Activity log.** Every change, and every refused one, is written to the shared activity log as origin `mcp` (`dispatched activity ls --origin mcp`). The desktop app's Activity tab shows these entries and refreshes the views they touched.
- **Bounded.** Peeks return at most 500 messages, searches examine at most 100,000, bodies are cut at 64 KiB, and a change names or matches at most 1,000 messages (100 for a send).

Errors come back as tool results with `isError`, in the words the CLI uses (which right is missing, what was not found, what to do). Arguments the server cannot use are reported as invalid parameters.

## Setting it up

The examples run `dispatched` from your `PATH`; use the full path if it is elsewhere.

**Claude Code**

```bash
claude mcp add dispatched -- dispatched mcp --profile <name>
claude mcp add dispatched-ro -- dispatched mcp --read-only --profile <name>
```

**Claude Desktop** (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "dispatched": {
      "command": "dispatched",
      "args": ["mcp", "--read-only"]
    }
  }
}
```

**VS Code** (`.vscode/mcp.json`)

```json
{
  "servers": {
    "dispatched": {
      "type": "stdio",
      "command": "dispatched",
      "args": ["mcp", "--read-only"]
    }
  }
}
```

Add `"--profile", "<name>"` to `args` to pick a saved profile; without it the default profile is used. Drop `--read-only` once you know your client can show confirmation forms.

Notes and warnings go to stderr, which clients keep as the server's log; stdout carries only the protocol.
