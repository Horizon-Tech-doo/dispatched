# Dispatched

**A fast, cross-platform explorer for Azure Service Bus. Built for humans and agents.**

Dispatched is an open-source alternative to [ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer) that runs on Windows, macOS and Linux, signs in with Microsoft Entra ID, works with the Service Bus Emulator, and (soon) exposes an MCP server so agents can inspect and operate your messaging safely.

> **Status:** M0 (spike and skeleton). Browse-only. Not ready for production use.
> Auto-update: pending (signing key not yet configured).

## Try it (CLI)

```bash
cargo install --path crates/dispatched-cli   # from a checkout with ../horizon-azure-kit beside it
dispatched login                              # browser sign-in (or --device-code)
dispatched namespaces
dispatched --namespace myns ls --topics
dispatched --namespace myns peek orders --dlq
```

With a connection string (SAS or emulator). Prefer the `DISPATCHED_CONNECTION_STRING` environment variable over `--connection-string`: command-line arguments are visible in shell history and process lists.

```bash
export DISPATCHED_CONNECTION_STRING='Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;'
dispatched peek orders
```

## Develop

- Clone `horizon-azure-kit` next to this repo (shared Entra / keychain / ARM code).
- `cargo test` runs the core and CLI tests.
- Emulator tests: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0`.
  On Apple Silicon, `emulator.sh` automatically adds `docker-compose.arm64.yml` (Azure SQL Edge instead of the amd64-only SQL Server image).
- Desktop: `cd app && npm install && npm run tauri dev`.

Design: [`docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`](docs/superpowers/specs/2026-10-13-dispatched-v1-design.md).

## License

Apache-2.0. See [NOTICE](NOTICE). Not affiliated with Microsoft.
