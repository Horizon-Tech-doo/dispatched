# Dispatched — Azure Service Bus Explorer

**Explore, troubleshoot and manage Azure Service Bus on Windows, macOS and Linux.**

[![Latest release](https://img.shields.io/github/v/release/Horizon-Tech-doo/dispatched)](https://github.com/Horizon-Tech-doo/dispatched/releases/latest)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE-APACHE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-blue)](https://github.com/Horizon-Tech-doo/dispatched/releases/latest)

Dispatched is a free, open-source **Azure Service Bus explorer** with a native desktop app, a scriptable CLI and an MCP server for AI agents. Browse queues, topics and subscriptions; inspect messages; troubleshoot dead-letter queues; and replay messages with confirmations and a shared activity log.

[**Website**](https://dispatchedapp.com) · [**Download the preview**](https://dispatchedapp.com/download/) · [Mac App Store](https://apps.apple.com/app/id6826620230) · [Microsoft Store](https://apps.microsoft.com/detail/9NNTH1L8LCM4) · [Guides](https://dispatchedapp.com/guides/) · [Usage guide](docs/usage.md) · [MCP setup](docs/mcp.md) · [Report an issue](https://github.com/Horizon-Tech-doo/dispatched/issues)

> **Public preview (0.1.0).** Desktop and CLI support browsing, message operations and entity management. Not ready for production use; settings and storage formats may change before 1.0. See the [release notes](https://github.com/Horizon-Tech-doo/dispatched/releases/latest) for platform requirements and known limitations.

## What you can do

- **Inspect messages without consuming them.** Peek active, scheduled, deferred, dead-letter and transfer dead-letter messages. Read bodies as JSON, XML, text or hex.
- **Find the messages that matter.** Filter by properties, body text, regular expressions, JSONPath or session. Group dead letters by reason and export messages as JSON or NDJSON.
- **Replay and repair.** Send, schedule, receive, settle, resubmit with edits, move, delete and purge messages. Manage session state and cancel scheduled messages.
- **Manage your namespace.** Create and edit queues, topics, subscriptions, SQL and correlation rules, and shared access policies. Import and export ServiceBusExplorer entity XML.
- **Watch live traffic.** Listen for messages and inspect message and byte rates, or explicitly choose a receiving mode.
- **Use your existing connection.** Sign in with Microsoft Entra ID, discover namespaces across tenants, or use a connection string. Work locally with the Azure Service Bus Emulator.
- **Automate through CLI or MCP.** Use JSON output in scripts or connect an AI assistant. MCP supports a read-only mode and requires client confirmation for changes.

Changes show their intended effect before running. Profiles marked as production require entity-name confirmation. Desktop, CLI and MCP share a local activity log; credentials are stored in the system keychain.

## Download and connect

Get desktop installers and CLI archives from the [latest release](https://github.com/Horizon-Tech-doo/dispatched/releases/latest). The desktop app is also on the [Mac App Store](https://apps.apple.com/app/id6826620230) and the [Microsoft Store](https://apps.microsoft.com/detail/9NNTH1L8LCM4) (as Dispatched Explorer); see [dispatchedapp.com/download](https://dispatchedapp.com/download/).

| Platform | Desktop | CLI |
| --- | --- | --- |
| macOS (Apple silicon and Intel) | Universal `.dmg` | Universal `.zip` |
| Windows (x64 and ARM64) | `.exe` installer or `.msi` | `.zip` |
| Linux (x64 and ARM64) | `.AppImage`, `.deb` or `.rpm` | `.tar.gz` |

Open the desktop app and connect with Microsoft Entra ID or a Service Bus connection string. For the CLI, extract the archive, put `dispatched` on your PATH, then:

```bash
dispatched login
dispatched namespaces
dispatched --namespace myns ls --topics
dispatched --namespace myns peek orders --dlq
```

See the [usage guide](docs/usage.md) for connection profiles, emulator setup, filters and the complete operation reference.

## Looking for a ServiceBusExplorer alternative?

[ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer) is an established Windows tool for Azure messaging. Dispatched focuses on Azure Service Bus and brings that workflow to **macOS and Linux as well as Windows**, with a CLI and an MCP interface alongside the desktop app.

You can reuse ServiceBusExplorer entity XML exports and `BrokeredMessageTemplate` files. Dispatched does not replace ServiceBusExplorer's Event Hubs, Notification Hubs or Relay tooling.

## Connect an AI assistant

Register the MCP server with Claude Code using a saved connection profile:

```bash
claude mcp add dispatched -- dispatched mcp --profile <name>
```

Agents can inspect entities, search messages and summarize dead-letter queues. Write operations require confirmation in a compatible MCP client; clients without confirmation support cannot make changes. Use `--read-only` to omit write tools. See the [MCP reference](docs/mcp.md) for client configuration and the confirmation model.

## Develop

- Clone `horizon-azure-kit` next to this repo (shared Entra / keychain / ARM code).
- `cargo test --workspace` runs the core and CLI tests.
- Emulator gate: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1 --test emulator_m1c --test emulator_m2a`.
  On Apple Silicon, `emulator.sh` automatically adds `docker-compose.arm64.yml` (Azure SQL Edge instead of the amd64-only SQL Server image).
- Desktop: `cd app && npm install && npm run tauri:dev`. On macOS this signs the dev build with your Developer ID or Apple Development certificate, if one is installed, so the Keychain's "Always Allow" survives rebuilds.

Design: [`docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`](docs/superpowers/specs/2026-10-13-dispatched-v1-design.md).

## License

Apache-2.0. See [NOTICE](NOTICE). Not affiliated with Microsoft.
