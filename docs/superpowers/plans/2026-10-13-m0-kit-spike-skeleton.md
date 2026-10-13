# M0 — Kit, Spike and Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the shared `horizon-azure-kit` crate and the Dispatched skeleton, and prove approach A′ end to end. The proof is: Entra or SAS (cloud) and the emulator → list queues over ATOM → peek messages over AMQP `$management` → peek the dead-letter queue with reasons, from the CLI and from a minimal desktop window.

**Architecture:**
- **Two repos side by side** under `/Users/hamzaabdagic/Development/Horizon-Tech-doo/`: `horizon-azure-kit/` (the shared crate) and `dispatched/` (core library, CLI, Tauri app).
- **`dispatched-core` owns all Service Bus logic:**
  - connection strings, namespace endpoints, token sources
  - ATOM management client
  - `fe2o3-amqp` connection + CBS + `$management` request/response
  - message decoding
  - an `Explorer` facade
- **The CLI and the Tauri app are thin layers** that call `Explorer`.

**Tech Stack:**
- Rust 2024 edition (MSRV 1.88), tokio
- `fe2o3-amqp` 0.18 (+ `-types`, `-cbs`), `serde_amqp` 0.18
- `reqwest` 0.13 (rustls), `quick-xml` 0.36, `keyring` 4, `secrecy` 0.10
- clap 4, Tauri 2, React 19 + Vite + TypeScript
- Docker Compose (Service Bus Emulator 2.0.1)

**Spec:** `dispatched/docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` (approved, revised 2026-10-13). Read sections 1–3, 8–11 before starting.

## Global Constraints

- **Repo locations:**
  - Kit: `/Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit`
  - Dispatched: `/Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched` (already `git init`-ed, spec committed)
- **Disk hygiene (hard rule, from the user's CLAUDE.md):**
  - Build only in each repo's own `target/`.
  - Never copy a repo or crate to a scratch dir to build it, and never set a new `CARGO_TARGET_DIR`.
  - Before any heavy build (`cargo build/test` of the whole workspace, `tauri build`, docker pulls), run `df -h /System/Volumes/Data`. If free space is under 30 GB, stop and tell the user.
  - Docker: after emulator runs, use `docker compose down -v`. At the end of the session, remove the images this session pulled (Task 20). Never run `docker system prune`.
- **Licensing and attribution:**
  - All code is Apache-2.0, copyright "Horizon Tech d.o.o.".
  - Nothing is ported from Microsoft's unpublished `azure_messaging_servicebus`.
  - Protocol knowledge taken from `azservicebus` (MIT, minghuaw, itself a port of Microsoft's MIT .NET SDK) is credited in `dispatched/NOTICE`.
- **Edition and MSRV:** `edition = "2024"`, `rust-version = "1.88"`. `fe2o3-amqp` 0.18 needs at least 1.85.
- **TLS:**
  - `reqwest` uses `default-features = false, features = ["rustls", "http2", "charset", "system-proxy", "json", "form", "query"]`.
  - `fe2o3-amqp` uses `features = ["rustls"]` only. Never enable `native-tls` alongside it: with both enabled, `amqps://` silently switches to native-tls.
- **Pinned versions** (use exactly these major/minor lines):
  - async-trait 0.1, base64 0.22, hmac 0.12, sha2 0.10, percent-encoding 2
  - secrecy 0.10, keyring 4 (default `v1` feature)
  - serde 1, serde_json 1, thiserror 2, time 0.3, uuid 1, url 2
  - tokio 1, tokio-util 0.7, tracing 0.1, quick-xml 0.36, mockito 1, clap 4.6
  - fe2o3-amqp 0.18.2, fe2o3-amqp-types 0.18, fe2o3-amqp-cbs 0.18, serde_amqp 0.18 (features `time`, `uuid`)
  - tauri 2
- **Keychain naming:**
  - Kit keys are `"{key_prefix}:entra:{sign_in_id}"`.
  - Dispatched uses keychain service `io.horizon-tech.dispatched` and key prefix `dispatched`.
  - The kit's chunk marker is `hzkit-chunked-v1:`. This deliberately breaks compatibility with Arkived's old format (the user chose long-term design over compatibility; Arkived users re-login once after migrating).
- **Entra:**
  - Public client id `04b07795-8ddb-461a-bbee-02f9e1bf7b46` (Azure CLI).
  - Sign-in scope: `https://management.azure.com/.default offline_access openid profile`.
  - Service Bus scope: `https://servicebus.azure.net/.default`.
- **Service Bus protocol constants:**
  - CBS token types: `jwt` and `servicebus.windows.net:sastoken`.
  - ATOM `api-version=2024-05`.
  - ARM namespaces `api-version=2024-01-01`.
  - Management node: `{entity_path}/$management`.
- **Emulator:**
  - Connection string `Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;`
  - AMQP `amqp://localhost:5672`, no TLS. Management `http://localhost:5300`.
  - Images `mcr.microsoft.com/azure-messaging/servicebus-emulator:2.0.1` and `mcr.microsoft.com/mssql/server:2022-latest`.
- **Outward-facing actions need the user's explicit yes, every time:** creating GitHub repos, pushing, creating Azure resources, publishing crates.
- **UI:**
  - The M0 window is plumbing proof only: semantic HTML with system defaults, no visual design work, no CSS framework.
  - The design system is M1's first task and runs the **impeccable** and **taste** skills. Do not add colours, gradients, cards or decorative styling in M0.
- **Deliberately out of M0** (each belongs to a later milestone plan):
  - token auto-refresh for long-lived AMQP links (M1)
  - profiles (M1)
  - browser-opened Tauri sign-in UI (M1)
  - WebSockets and proxy (M1)
  - send/receive/settle as product features (M2; M0 has test-only seeding helpers)
  - jobs and audit modules in the kit (M2)
  - MCP (M4)

## Review Focus

These five inputs are the most likely to hurt a real user, and no task covers them by default. Each one has a pinned test in the task that owns the code.

1. **Expired or revoked Entra session.** Covers refresh returning `invalid_grant` (90-day expiry, password reset) or `interaction_required` (MFA / Conditional Access). Users expect a clear "sign in again" error, `KitError::NotSignedIn`, and never a raw token-endpoint dump. → Task 3 test `refresh_invalid_grant_is_not_signed_in`, Task 5 test `token_for_after_revocation_reports_not_signed_in`.
2. **Peeking an empty entity or a sequence number past the end.** The broker answers `204` or an empty `messages` list. Users expect an empty result, not an error. → Task 15 test `peek_response_204_is_empty`.
3. **Message bodies that aren't a single UTF-8 data section.** Covers an AMQP-value string body (from some SDKs), several data sections, a sequence body, an empty body, and binary that isn't UTF-8. Users expect the message to still display, and never a panic. → Task 14 tests `decodes_amqp_value_string_body`, `decodes_multiple_data_sections`, `non_utf8_binary_has_no_text`.
4. **ATOM quirks.**
   - GET for a missing entity returns 200 with an empty `<feed>`.
   - 401/403 bodies carry `<Detail>` text naming the missing claims.
   - Users expect `NotFound`, and `Forbidden { needed: Some("Manage,EntityRead") }`.
   - → Task 12 tests `get_missing_queue_is_not_found`, `forbidden_names_missing_claims`.
5. **Connection-string variants pasted from the portal or other tools:**
   - trailing `;`
   - keys in any case
   - `sb://host/` with a trailing slash
   - an explicit port
   - `EntityPath`
   - a pre-signed `SharedAccessSignature=`
   - `UseDevelopmentEmulator=true`
   - Users expect each to parse, or to fail with a message naming the missing key. → Task 10 tests in `connstr.rs`.

---

## File Structure

### horizon-azure-kit
```
horizon-azure-kit/
├── Cargo.toml                          workspace
├── rust-toolchain.toml
├── LICENSE-APACHE, README.md, .gitignore
├── .github/workflows/ci.yml            fmt, clippy, test (3 OS), msrv
├── .github/workflows/tauri-release.yml reusable desktop release workflow
└── crates/horizon-azure-kit/
    ├── Cargo.toml
    └── src/
        ├── lib.rs                      module wiring + re-exports
        ├── error.rs                    KitError, KitResult
        ├── cloud.rs                    AzureCloud endpoints
        ├── secrets/mod.rs              CredentialStore trait, MemoryStore
        ├── secrets/keychain.rs         OsKeychain (chunked), feature "keychain"
        ├── auth/mod.rs                 re-exports
        ├── auth/oauth.rs               OAuthClient: device code, refresh, auth-code exchange, authorize URL
        ├── auth/browser.rs             PKCE + loopback redirect listener
        ├── auth/claims.rs              JWT identity claims (unverified decode)
        ├── auth/session.rs             EntraSession: login, token_for (cached), logout
        ├── sas.rs                      SAS token signing
        ├── arm.rs                      ArmClient: tenants, subscriptions, list_by_provider
        └── safety.rs                   Policy, ConfirmRequest, Decision, Origin, require(), CancellationToken
```

### dispatched
```
dispatched/
├── Cargo.toml                          workspace (default-members = crates/*), kit as path+version dep
├── rust-toolchain.toml, LICENSE-APACHE, NOTICE, README.md, .gitignore
├── .github/workflows/ci.yml            rust (3 OS), app (ubuntu), emulator (ubuntu)
├── .github/workflows/release.yml       calls kit tauri-release.yml
├── testing/emulator/                   docker-compose.yml, Config.json, .env.example, emulator.sh
├── crates/dispatched-core/
│   ├── Cargo.toml
│   ├── src/lib.rs
│   ├── src/error.rs                    Error enum + From<KitError>
│   ├── src/connstr.rs                  ConnectionString::parse
│   ├── src/namespace.rs                Namespace endpoints/audiences, EntityPath, SubQueue
│   ├── src/tokens.rs                   TokenSource trait, SAS / pre-signed / Entra sources
│   ├── src/mgmt/mod.rs                 ManagementClient (ATOM REST)
│   ├── src/mgmt/xml.rs                 lenient local-name XML tree
│   ├── src/mgmt/models.rs              QueueInfo, TopicInfo, SubscriptionInfo, EntityCounts + parsing
│   ├── src/messaging/mod.rs            re-exports
│   ├── src/messaging/connection.rs     ServiceBusConnection (fe2o3 conn + session + CBS)
│   ├── src/messaging/management.rs     ManagementLink: correlated request/response on $management
│   ├── src/messaging/message.rs        PeekedMessage, MessageBody, PropValue + decode
│   ├── src/messaging/peek.rs           peek request/response
│   ├── src/messaging/testing.rs        feature "test-support": send_text, dead_letter_next
│   ├── src/discovery.rs                Entra → ARM → Service Bus namespaces
│   ├── src/explorer.rs                 Explorer facade
│   └── tests/emulator_m0.rs            M0 gate test against the emulator (DISPATCHED_EMULATOR=1)
├── crates/dispatched-cli/
│   ├── Cargo.toml                      [[bin]] name = "dispatched"
│   └── src/{main.rs, state.rs, resolve.rs, output.rs}
└── app/
    ├── package.json, index.html, vite.config.ts, tsconfig.json
    ├── src/{main.tsx, App.tsx, api.ts}
    └── src-tauri/                      workspace member "dispatched-app"
        ├── Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/
        └── src/{main.rs, lib.rs, state.rs, error.rs, commands/{mod.rs, connection.rs, browse.rs}}
```

---
## Part A — horizon-azure-kit

### Task 1: Kit workspace, error type, cloud endpoints, CI

**Files:**
- Create: `horizon-azure-kit/Cargo.toml`, `horizon-azure-kit/rust-toolchain.toml`, `horizon-azure-kit/.gitignore`, `horizon-azure-kit/README.md`, `horizon-azure-kit/LICENSE-APACHE`
- Create: `horizon-azure-kit/crates/horizon-azure-kit/Cargo.toml`
- Create: `horizon-azure-kit/crates/horizon-azure-kit/src/lib.rs`, `src/error.rs`, `src/cloud.rs`
- Create: `horizon-azure-kit/.github/workflows/ci.yml`

**Interfaces:**
- Produces: `horizon_azure_kit::{KitError, KitResult, AzureCloud}`. `KitError` variants: `AuthFailed(String)`, `NotSignedIn(String)`, `NotFound(String)`, `Forbidden(String)`, `Network(String)`, `Remote { status: u16, body: String }`, `SecretStore(String)`, `PolicyDenied(String)`, `Cancelled`, `Invalid(String)`. `AzureCloud::{Public, China, UsGov}` with `authority_host() -> &'static str`, `arm_endpoint() -> &'static str`, `arm_scope() -> String`.

- [ ] **Step 1: Create the repo and workspace files**

```bash
mkdir -p /Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit/crates/horizon-azure-kit/src
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit
git init -b main
cp ../arkived/LICENSE-APACHE .
```

`horizon-azure-kit/Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/horizon-azure-kit"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
license = "Apache-2.0"
authors = ["Hamza Abdagić <hamza.abdagic@horizon-tech.io>", "Horizon Tech d.o.o."]
repository = "https://github.com/Horizon-Tech-doo/horizon-azure-kit"

[profile.release]
lto = "thin"
codegen-units = 1
```

`horizon-azure-kit/rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

`horizon-azure-kit/.gitignore`:
```
/target
.DS_Store
.idea/
.vscode/
/.claude/
```

`horizon-azure-kit/README.md`:
```markdown
# horizon-azure-kit

Shared Azure plumbing for Horizon Tech's desktop tools ([Arkived](https://github.com/Horizon-Tech-doo/arkived), [Dispatched](https://github.com/Horizon-Tech-doo/dispatched)):

- `auth` — Microsoft Entra sign-in (browser + PKCE, device code), refresh-token cache, per-scope token exchange
- `secrets` — OS keychain credential store (chunked for large tokens) + in-memory store
- `arm` — Azure Resource Manager discovery (tenants, subscriptions, resources by provider)
- `sas` — Shared Access Signature signing
- `safety` — confirmation policy for destructive operations, cancellation

Only code that more than one product needs lives here. Product domain logic does not.

Not affiliated with Microsoft. "Azure" and "Microsoft Entra" are trademarks of Microsoft Corporation.

Licensed under Apache-2.0.
```

`horizon-azure-kit/crates/horizon-azure-kit/Cargo.toml`:
```toml
[package]
name = "horizon-azure-kit"
description = "Shared Azure plumbing for Horizon Tech desktop tools: Entra sign-in, OS keychain secrets, ARM discovery, SAS signing, safety policy."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
readme = "../../README.md"
keywords = ["azure", "entra", "keychain", "sas"]
categories = ["authentication", "api-bindings"]

[features]
default = ["keychain"]
keychain = ["dep:keyring"]

[dependencies]
async-trait = "0.1"
base64 = "0.22"
hmac = "0.12"
keyring = { version = "4", optional = true }
percent-encoding = "2"
reqwest = { version = "0.13", default-features = false, features = ["rustls", "http2", "charset", "system-proxy", "json", "form", "query"] }
secrecy = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"
time = { version = "0.3", features = ["serde", "serde-well-known"] }
tokio = { version = "1", features = ["net", "io-util", "time", "macros", "rt", "sync"] }
tokio-util = "0.7"
tracing = "0.1"
url = "2"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
mockito = "1"
tokio = { version = "1", features = ["full"] }
```

- [ ] **Step 2: Write the failing tests for error display and cloud endpoints**

`src/error.rs`:
```rust
//! Error type shared by every kit module.

/// Result alias for kit operations.
pub type KitResult<T> = Result<T, KitError>;

/// Errors produced by the kit. Products convert these into their own error
/// types with `From`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KitError {
    /// Sign-in or token exchange failed.
    #[error("authentication failed: {0}")]
    AuthFailed(String),
    /// No usable session: never signed in, refresh token expired or revoked,
    /// or the tenant requires interactive sign-in (MFA / Conditional Access).
    #[error("not signed in: {0}")]
    NotSignedIn(String),
    /// A requested resource or secret does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The identity lacks permission.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// Transport-level failure (DNS, TLS, connection reset, timeout).
    #[error("network error: {0}")]
    Network(String),
    /// A remote service answered with an unexpected status.
    #[error("unexpected response ({status}): {body}")]
    Remote {
        /// HTTP status code.
        status: u16,
        /// Response body (may be truncated by the caller).
        body: String,
    },
    /// The OS keychain or another secret store failed.
    #[error("secret store error: {0}")]
    SecretStore(String),
    /// A destructive operation was refused by the confirmation policy.
    #[error("operation denied: {0}")]
    PolicyDenied(String),
    /// The operation was cancelled by the caller.
    #[error("cancelled")]
    Cancelled,
    /// The caller passed invalid input.
    #[error("invalid input: {0}")]
    Invalid(String),
}

impl KitError {
    pub(crate) fn network(e: reqwest::Error) -> Self {
        KitError::Network(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_are_actionable() {
        assert_eq!(
            KitError::NotSignedIn("session expired; sign in again".into()).to_string(),
            "not signed in: session expired; sign in again"
        );
        assert_eq!(
            KitError::Remote { status: 500, body: "boom".into() }.to_string(),
            "unexpected response (500): boom"
        );
    }
}
```

`src/cloud.rs`:
```rust
//! Azure cloud endpoints.

use serde::{Deserialize, Serialize};

/// The Azure cloud a sign-in or resource belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AzureCloud {
    /// Azure public cloud.
    #[default]
    Public,
    /// Azure operated by 21Vianet (China).
    China,
    /// Azure US Government.
    UsGov,
}

impl AzureCloud {
    /// Microsoft Entra authority host, without trailing slash.
    pub fn authority_host(self) -> &'static str {
        match self {
            AzureCloud::Public => "https://login.microsoftonline.com",
            AzureCloud::China => "https://login.chinacloudapi.cn",
            AzureCloud::UsGov => "https://login.microsoftonline.us",
        }
    }

    /// Azure Resource Manager endpoint, without trailing slash.
    pub fn arm_endpoint(self) -> &'static str {
        match self {
            AzureCloud::Public => "https://management.azure.com",
            AzureCloud::China => "https://management.chinacloudapi.cn",
            AzureCloud::UsGov => "https://management.usgovcloudapi.net",
        }
    }

    /// OAuth scope for ARM access tokens.
    pub fn arm_scope(self) -> String {
        format!("{}/.default", self.arm_endpoint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_cloud_endpoints() {
        let c = AzureCloud::Public;
        assert_eq!(c.authority_host(), "https://login.microsoftonline.com");
        assert_eq!(c.arm_scope(), "https://management.azure.com/.default");
    }

    #[test]
    fn sovereign_clouds_differ() {
        assert_ne!(AzureCloud::China.arm_endpoint(), AzureCloud::Public.arm_endpoint());
        assert_eq!(AzureCloud::UsGov.authority_host(), "https://login.microsoftonline.us");
    }

    #[test]
    fn serde_names_are_kebab_case() {
        assert_eq!(serde_json::to_string(&AzureCloud::UsGov).unwrap(), "\"us-gov\"");
    }
}
```

`src/lib.rs`:
```rust
//! Shared Azure plumbing for Horizon Tech desktop tools.
//!
//! See the crate README for the module overview. Only code needed by more
//! than one product lives here.

#![deny(rust_2018_idioms, unsafe_code, missing_docs)]
#![warn(clippy::all)]

pub mod cloud;
pub mod error;

pub use cloud::AzureCloud;
pub use error::{KitError, KitResult};
```

- [ ] **Step 3: Run the tests**

Run: `df -h /System/Volumes/Data` (stop if under 30 GB free), then `cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit && cargo test`
Expected: PASS. 4 tests: `displays_are_actionable`, `public_cloud_endpoints`, `sovereign_clouds_differ`, `serde_names_are_kebab_case`. If a warning says `KitError::network` is unused, that's expected until Task 3.

- [ ] **Step 4: Add CI**

`horizon-azure-kit/.github/workflows/ci.yml`:
```yaml
name: CI
on:
  push: { branches: [main] }
  pull_request: { branches: [main] }
env:
  CARGO_TERM_COLOR: always
jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt }
      - run: cargo fmt --all --check
  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy }
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --all-targets --all-features -- -D warnings
  test:
    strategy:
      fail-fast: false
      matrix: { os: [ubuntu-latest, macos-latest, windows-latest] }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-features
  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with: { toolchain: "1.88" }
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --all-features
```

- [ ] **Step 5: Verify MSRV locally if the toolchain is available**

Run: `rustup toolchain list | grep -q 1.88 && cargo +1.88 check --all-features || echo "1.88 not installed; CI msrv job covers it"`
Expected: success, or the echo. If `cargo +1.88` fails because a dependency needs a newer compiler, raise `rust-version` in `Cargo.toml` and the CI `toolchain` to the version cargo names, and note it in the commit message.

- [ ] **Step 6: Commit**

```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit
cargo fmt --all
git add -A
git commit -m "Kit: workspace, KitError, AzureCloud endpoints, CI"
```

---

### Task 2: Secrets — CredentialStore, MemoryStore, chunked OsKeychain

**Files:**
- Create: `crates/horizon-azure-kit/src/secrets/mod.rs`, `src/secrets/keychain.rs`
- Modify: `crates/horizon-azure-kit/src/lib.rs`

**Interfaces:**
- Consumes: `KitError`, `KitResult` (Task 1).
- Produces:
  - `horizon_azure_kit::secrets::{CredentialStore, MemoryStore}`. `OsKeychain` exists behind feature `keychain`.
  - `trait CredentialStore: Send + Sync { fn put(&self, key: &str, secret: &SecretString) -> KitResult<()>; fn get(&self, key: &str) -> KitResult<Option<SecretString>>; fn delete(&self, key: &str) -> KitResult<()>; }`
  - `MemoryStore::new()`, `OsKeychain::new(service: impl Into<String>)`.

- [ ] **Step 1: Write the trait, MemoryStore and failing tests**

`src/secrets/mod.rs`:
```rust
//! Secret storage. Secrets never touch plaintext config files.

#[cfg(feature = "keychain")]
mod keychain;

#[cfg(feature = "keychain")]
pub use keychain::OsKeychain;

use crate::KitResult;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::sync::Mutex;

/// A key/value store for secrets.
pub trait CredentialStore: Send + Sync {
    /// Store `secret` under `key`, replacing any existing value.
    fn put(&self, key: &str, secret: &SecretString) -> KitResult<()>;
    /// Read the secret stored under `key`. `Ok(None)` if absent.
    fn get(&self, key: &str) -> KitResult<Option<SecretString>>;
    /// Remove the secret under `key`. Removing a missing key is not an error.
    fn delete(&self, key: &str) -> KitResult<()>;
}

/// In-memory store for tests and for `--no-keychain` sessions.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for MemoryStore {
    fn put(&self, key: &str, secret: &SecretString) -> KitResult<()> {
        self.inner
            .lock()
            .expect("memory store poisoned")
            .insert(key.to_string(), secret.expose_secret().to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> KitResult<Option<SecretString>> {
        Ok(self
            .inner
            .lock()
            .expect("memory store poisoned")
            .get(key)
            .map(|v| SecretString::from(v.clone())))
    }

    fn delete(&self, key: &str) -> KitResult<()> {
        self.inner.lock().expect("memory store poisoned").remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip_and_idempotent_delete() {
        let store = MemoryStore::new();
        store.put("k", &SecretString::from("v")).unwrap();
        assert_eq!(store.get("k").unwrap().unwrap().expose_secret(), "v");
        store.delete("k").unwrap();
        store.delete("k").unwrap();
        assert!(store.get("k").unwrap().is_none());
    }

    #[test]
    fn trait_is_object_safe() {
        fn take(_: &dyn CredentialStore) {}
        take(&MemoryStore::new());
    }
}
```

- [ ] **Step 2: Write OsKeychain with chunking and its tests**

Some keychains cap entry size: Windows Credential Manager allows 2560 bytes, and Entra refresh tokens can exceed it. So long values are split into ≤900-byte chunks with a marker entry.

`src/secrets/keychain.rs`:
```rust
//! OS keychain store (macOS Keychain, Windows Credential Manager, Linux Secret Service).

use super::CredentialStore;
use crate::{KitError, KitResult};
use secrecy::{ExposeSecret, SecretString};

const CHUNK_MARKER_PREFIX: &str = "hzkit-chunked-v1:";
const MAX_CHUNK_BYTES: usize = 900;

/// Credential store backed by the operating system keychain.
pub struct OsKeychain {
    service: String,
}

impl OsKeychain {
    /// Create a store whose entries are grouped under `service`
    /// (for example `io.horizon-tech.dispatched`).
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self, key: &str) -> KitResult<keyring::Entry> {
        keyring::Entry::new(&self.service, key)
            .map_err(|e| KitError::SecretStore(format!("keychain entry: {e}")))
    }

    fn raw_put(&self, key: &str, value: &str) -> KitResult<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| KitError::SecretStore(format!("keychain put: {e}")))
    }

    fn raw_get(&self, key: &str) -> KitResult<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KitError::SecretStore(format!("keychain get: {e}"))),
        }
    }

    fn raw_delete(&self, key: &str) -> KitResult<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KitError::SecretStore(format!("keychain delete: {e}"))),
        }
    }

    fn delete_chunks(&self, key: &str) -> KitResult<()> {
        if let Some(count) = self.raw_get(key)?.as_deref().and_then(parse_marker) {
            for i in 0..count {
                self.raw_delete(&chunk_key(key, i))?;
            }
        }
        Ok(())
    }
}

impl CredentialStore for OsKeychain {
    fn put(&self, key: &str, secret: &SecretString) -> KitResult<()> {
        let value = secret.expose_secret();
        self.delete_chunks(key)?;
        if value.len() <= MAX_CHUNK_BYTES && !value.starts_with(CHUNK_MARKER_PREFIX) {
            return self.raw_put(key, value);
        }
        let chunks = split_chunks(value);
        for (i, chunk) in chunks.iter().enumerate() {
            self.raw_put(&chunk_key(key, i), chunk)?;
        }
        self.raw_put(key, &format!("{CHUNK_MARKER_PREFIX}{}", chunks.len()))
    }

    fn get(&self, key: &str) -> KitResult<Option<SecretString>> {
        let Some(head) = self.raw_get(key)? else {
            return Ok(None);
        };
        let Some(count) = parse_marker(&head) else {
            return Ok(Some(SecretString::from(head)));
        };
        let mut joined = String::new();
        for i in 0..count {
            let part = self.raw_get(&chunk_key(key, i))?.ok_or_else(|| {
                KitError::SecretStore(format!("keychain chunk {i} of `{key}` is missing"))
            })?;
            joined.push_str(&part);
        }
        Ok(Some(SecretString::from(joined)))
    }

    fn delete(&self, key: &str) -> KitResult<()> {
        self.delete_chunks(key)?;
        self.raw_delete(key)
    }
}

fn chunk_key(key: &str, index: usize) -> String {
    format!("{key}::chunk::{index}")
}

fn parse_marker(value: &str) -> Option<usize> {
    value.strip_prefix(CHUNK_MARKER_PREFIX)?.parse().ok()
}

fn split_chunks(value: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in value.chars() {
        if !current.is_empty() && current.len() + ch.len_utf8() > MAX_CHUNK_BYTES {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_bytes_and_respects_limit() {
        let source = "x".repeat(MAX_CHUNK_BYTES * 2 + 17);
        let chunks = split_chunks(&source);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.len() <= MAX_CHUNK_BYTES));
        assert_eq!(chunks.concat(), source);
    }

    #[test]
    fn split_never_breaks_utf8() {
        let source = format!("{}å", "x".repeat(MAX_CHUNK_BYTES));
        assert_eq!(split_chunks(&source).concat(), source);
    }

    #[test]
    fn marker_roundtrip() {
        assert_eq!(parse_marker(&format!("{CHUNK_MARKER_PREFIX}3")), Some(3));
        assert_eq!(parse_marker("plain"), None);
    }

    /// Hits the real OS keychain. Run locally:
    /// `cargo test -p horizon-azure-kit -- --ignored keychain_roundtrip`
    #[test]
    #[ignore]
    fn keychain_roundtrip_large_secret() {
        let store = OsKeychain::new("io.horizon-tech.kit-test");
        let big = "t".repeat(3000);
        store.put("roundtrip", &SecretString::from(big.clone())).unwrap();
        assert_eq!(store.get("roundtrip").unwrap().unwrap().expose_secret(), big);
        store.delete("roundtrip").unwrap();
        assert!(store.get("roundtrip").unwrap().is_none());
    }
}
```

Modify `src/lib.rs`. Add after `pub mod error;`:
```rust
pub mod secrets;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p horizon-azure-kit secrets`
Expected: PASS. 5 tests pass, 1 is ignored.

Then run: `cargo test -p horizon-azure-kit -- --ignored keychain_roundtrip`
Expected: PASS on macOS. macOS may show a keychain permission prompt; allow it.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: CredentialStore, MemoryStore, chunked OsKeychain"
```

---

### Task 3: OAuthClient — device code, refresh, authorization code, authorize URL

**Files:**
- Create: `crates/horizon-azure-kit/src/auth/mod.rs`, `src/auth/oauth.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `KitError`, `KitResult`.
- Produces: `horizon_azure_kit::auth::{OAuthClient, DeviceCodeResponse, TokenResponse}`:
  - `OAuthClient::new(authority_host: impl Into<String>, client_id: impl Into<String>) -> Self`
  - `client_id(&self) -> &str`
  - `async start_device_code(&self, tenant: &str, scope: &str) -> KitResult<DeviceCodeResponse>`
  - `async poll_device_code(&self, tenant: &str, dc: &DeviceCodeResponse) -> KitResult<TokenResponse>`
  - `async refresh(&self, tenant: &str, refresh_token: &str, scope: &str) -> KitResult<TokenResponse>`
  - `authorize_url(&self, tenant: &str, scope: &str, redirect_uri: &str, state: &str, code_challenge: &str) -> String`
  - `async exchange_code(&self, tenant: &str, code: &str, redirect_uri: &str, code_verifier: &str, scope: &str) -> KitResult<TokenResponse>`
- `TokenResponse { access_token: String, id_token: Option<String>, refresh_token: Option<String>, expires_in: u64, scope: Option<String> }`.
- Error mapping: refresh errors `invalid_grant` and `interaction_required` (or any `AADSTS50076`/`AADSTS50079`/`AADSTS53003` in the description) map to `KitError::NotSignedIn`. Other token errors map to `AuthFailed`.

- [ ] **Step 1: Write oauth.rs with tests against a mock authority**

`src/auth/oauth.rs`:
```rust
//! Microsoft Entra OAuth 2.0 endpoints (v2.0).

use crate::{KitError, KitResult};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Response of the `/devicecode` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    /// Code the user types at `verification_uri`.
    pub user_code: String,
    /// URL the user opens.
    pub verification_uri: String,
    /// Opaque code used to poll the token endpoint.
    pub device_code: String,
    /// Seconds until the codes expire.
    pub expires_in: u64,
    /// Minimum seconds between polls.
    pub interval: u64,
    /// Ready-to-show instruction text.
    pub message: String,
}

/// Successful token-endpoint response.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    /// Bearer access token.
    pub access_token: String,
    /// OpenID id_token when `openid` was requested.
    #[serde(default)]
    pub id_token: Option<String>,
    /// Refresh token when `offline_access` was requested.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Access-token lifetime in seconds.
    pub expires_in: u64,
    /// Granted scopes.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Client for one Entra authority host and one public client id.
#[derive(Debug, Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
    authority_host: String,
    client_id: String,
}

impl OAuthClient {
    /// Build a client. `authority_host` is e.g. `https://login.microsoftonline.com`.
    pub fn new(authority_host: impl Into<String>, client_id: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            authority_host: authority_host.into().trim_end_matches('/').to_string(),
            client_id: client_id.into(),
        }
    }

    /// The public client id in use.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    fn endpoint(&self, tenant: &str, path: &str) -> String {
        format!("{}/{tenant}/oauth2/v2.0/{path}", self.authority_host)
    }

    /// Start a device-code flow.
    pub async fn start_device_code(&self, tenant: &str, scope: &str) -> KitResult<DeviceCodeResponse> {
        let resp = self
            .http
            .post(self.endpoint(tenant, "devicecode"))
            .form(&[("client_id", self.client_id.as_str()), ("scope", scope)])
            .send()
            .await
            .map_err(KitError::network)?;
        let status = resp.status();
        let body = resp.text().await.map_err(KitError::network)?;
        if !status.is_success() {
            return Err(KitError::AuthFailed(format!("devicecode endpoint returned {status}: {body}")));
        }
        serde_json::from_str(&body).map_err(|e| KitError::AuthFailed(format!("devicecode parse: {e}")))
    }

    /// Poll until the user completes the device-code sign-in, it is declined, or it expires.
    pub async fn poll_device_code(&self, tenant: &str, dc: &DeviceCodeResponse) -> KitResult<TokenResponse> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in);
        let mut interval = Duration::from_secs(dc.interval.max(1));
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(KitError::AuthFailed("device-code sign-in timed out".into()));
            }
            let form = [
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", self.client_id.as_str()),
                ("device_code", dc.device_code.as_str()),
            ];
            match self.token_request(tenant, &form).await? {
                Ok(token) => return Ok(token),
                Err(err) => match err.error.as_str() {
                    "authorization_pending" => {}
                    "slow_down" => interval += Duration::from_secs(5),
                    _ => return Err(auth_error(err)),
                },
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// Exchange a refresh token for an access token for `scope`.
    pub async fn refresh(&self, tenant: &str, refresh_token: &str, scope: &str) -> KitResult<TokenResponse> {
        let form = [
            ("grant_type", "refresh_token"),
            ("client_id", self.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("scope", scope),
        ];
        self.token_request(tenant, &form).await?.map_err(auth_error)
    }

    /// Build the browser authorization URL (authorization code + PKCE S256).
    pub fn authorize_url(
        &self,
        tenant: &str,
        scope: &str,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> String {
        let mut url = url::Url::parse(&self.endpoint(tenant, "authorize"))
            .expect("authority host and tenant form a valid URL");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_mode", "query")
            .append_pair("scope", scope)
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("prompt", "select_account");
        url.into()
    }

    /// Redeem an authorization code obtained through [`OAuthClient::authorize_url`].
    pub async fn exchange_code(
        &self,
        tenant: &str,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
        scope: &str,
    ) -> KitResult<TokenResponse> {
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", self.client_id.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
            ("scope", scope),
        ];
        self.token_request(tenant, &form).await?.map_err(auth_error)
    }

    async fn token_request(
        &self,
        tenant: &str,
        form: &[(&str, &str)],
    ) -> KitResult<Result<TokenResponse, TokenError>> {
        let resp = self
            .http
            .post(self.endpoint(tenant, "token"))
            .form(form)
            .send()
            .await
            .map_err(KitError::network)?;
        let ok = resp.status().is_success();
        let body = resp.text().await.map_err(KitError::network)?;
        if ok {
            return serde_json::from_str(&body)
                .map(Ok)
                .map_err(|e| KitError::AuthFailed(format!("token parse: {e}")));
        }
        Ok(Err(serde_json::from_str(&body).unwrap_or(TokenError {
            error: "unknown_error".into(),
            error_description: Some(body),
        })))
    }
}

/// Map a token-endpoint error to a kit error. Expired, revoked or
/// MFA/Conditional-Access-blocked sessions become `NotSignedIn` so callers
/// can prompt for an interactive sign-in instead of showing a raw error.
fn auth_error(err: TokenError) -> KitError {
    let desc = err.error_description.unwrap_or_default();
    let needs_interaction = matches!(err.error.as_str(), "invalid_grant" | "interaction_required")
        || ["AADSTS50076", "AADSTS50079", "AADSTS53003", "AADSTS700082"]
            .iter()
            .any(|code| desc.contains(code));
    if needs_interaction {
        let first_line = desc.lines().next().unwrap_or_default().to_string();
        KitError::NotSignedIn(format!("sign in again ({}: {first_line})", err.error))
    } else {
        KitError::AuthFailed(format!("{}: {desc}", err.error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    fn token_json() -> &'static str {
        r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"token_type":"Bearer","scope":"s"}"#
    }

    #[tokio::test]
    async fn device_code_start_parses_response() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/organizations/oauth2/v2.0/devicecode")
            .match_body(Matcher::UrlEncoded("client_id".into(), "cid".into()))
            .with_status(200)
            .with_body(r#"{"user_code":"AB-CD","verification_uri":"https://microsoft.com/devicelogin","device_code":"dc","expires_in":900,"interval":5,"message":"go"}"#)
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        let dc = client.start_device_code("organizations", "scope").await.unwrap();
        assert_eq!(dc.user_code, "AB-CD");
        assert_eq!(dc.interval, 5);
    }

    #[tokio::test]
    async fn device_code_poll_returns_token_on_success() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/t/oauth2/v2.0/token")
            .match_body(Matcher::UrlEncoded("device_code".into(), "dc".into()))
            .with_status(200)
            .with_body(token_json())
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        let dc = DeviceCodeResponse {
            user_code: "u".into(),
            verification_uri: "v".into(),
            device_code: "dc".into(),
            expires_in: 30,
            interval: 1,
            message: "m".into(),
        };
        let token = client.poll_device_code("t", &dc).await.unwrap();
        assert_eq!(token.access_token, "at");
        assert_eq!(token.refresh_token.as_deref(), Some("rt"));
    }

    #[tokio::test]
    async fn device_code_declined_is_auth_failed() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/t/oauth2/v2.0/token")
            .with_status(400)
            .with_body(r#"{"error":"authorization_declined","error_description":"user said no"}"#)
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        let dc = DeviceCodeResponse {
            user_code: "u".into(),
            verification_uri: "v".into(),
            device_code: "dc".into(),
            expires_in: 30,
            interval: 1,
            message: "m".into(),
        };
        let err = client.poll_device_code("t", &dc).await.unwrap_err();
        assert!(matches!(err, KitError::AuthFailed(m) if m.contains("authorization_declined")));
    }

    #[tokio::test]
    async fn refresh_success() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/t/oauth2/v2.0/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("scope".into(), "https://servicebus.azure.net/.default offline_access".into()),
            ]))
            .with_status(200)
            .with_body(token_json())
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        let t = client
            .refresh("t", "rt", "https://servicebus.azure.net/.default offline_access")
            .await
            .unwrap();
        assert_eq!(t.access_token, "at");
    }

    #[tokio::test]
    async fn refresh_invalid_grant_is_not_signed_in() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/t/oauth2/v2.0/token")
            .with_status(400)
            .with_body(r#"{"error":"invalid_grant","error_description":"AADSTS700082: The refresh token has expired due to inactivity.\r\nTrace ID: x"}"#)
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        let err = client.refresh("t", "rt", "s").await.unwrap_err();
        assert!(matches!(&err, KitError::NotSignedIn(m) if m.contains("AADSTS700082")), "{err}");
    }

    #[tokio::test]
    async fn refresh_mfa_required_is_not_signed_in() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/t/oauth2/v2.0/token")
            .with_status(400)
            .with_body(r#"{"error":"interaction_required","error_description":"AADSTS50076: multi-factor authentication required"}"#)
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        assert!(matches!(client.refresh("t", "rt", "s").await.unwrap_err(), KitError::NotSignedIn(_)));
    }

    #[tokio::test]
    async fn exchange_code_sends_pkce_verifier() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("POST", "/t/oauth2/v2.0/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "authorization_code".into()),
                Matcher::UrlEncoded("code_verifier".into(), "verifier".into()),
                Matcher::UrlEncoded("redirect_uri".into(), "http://localhost:5555".into()),
            ]))
            .with_status(200)
            .with_body(token_json())
            .create_async()
            .await;
        let client = OAuthClient::new(server.url(), "cid");
        let t = client
            .exchange_code("t", "code", "http://localhost:5555", "verifier", "s")
            .await
            .unwrap();
        assert_eq!(t.access_token, "at");
    }

    #[test]
    fn authorize_url_carries_pkce_and_state() {
        let client = OAuthClient::new("https://login.microsoftonline.com/", "cid");
        let url = url::Url::parse(&client.authorize_url("organizations", "a b", "http://localhost:1", "st", "ch")).unwrap();
        assert_eq!(url.path(), "/organizations/oauth2/v2.0/authorize");
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], "ch");
        assert_eq!(q["state"], "st");
        assert_eq!(q["scope"], "a b");
    }
}
```

`src/auth/mod.rs`:
```rust
//! Microsoft Entra ID sign-in and token exchange.

mod oauth;

pub use oauth::{DeviceCodeResponse, OAuthClient, TokenResponse};
```

Modify `src/lib.rs`. Add `pub mod auth;` above `pub mod cloud;`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p horizon-azure-kit auth::oauth`
Expected: PASS, 8 tests.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: Entra OAuth client (device code, refresh, PKCE code exchange)"
```

---

### Task 4: Browser sign-in — PKCE and loopback redirect

**Files:**
- Create: `crates/horizon-azure-kit/src/auth/browser.rs`
- Modify: `src/auth/mod.rs`

**Interfaces:**
- Consumes: `KitError`, `KitResult`.
- Produces:
  - `auth::Pkce { verifier: String, challenge: String }` with `Pkce::generate()`.
  - `auth::LoopbackRedirect`:
    - `async bind() -> KitResult<Self>`
    - `redirect_uri(&self) -> String` (`http://localhost:{port}`)
    - `async wait_for_code(self, expected_state: &str, timeout: Duration) -> KitResult<String>`

- [ ] **Step 1: Write browser.rs with tests**

`src/auth/browser.rs`:
```rust
//! Interactive browser sign-in: authorization code + PKCE over a loopback redirect.

use crate::{KitError, KitResult};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SUCCESS_HTML: &str = "<!doctype html><title>Signed in</title><p>Signed in. You can close this window and return to the app.</p>";
const FAILURE_HTML: &str = "<!doctype html><title>Sign-in failed</title><p>Sign-in failed. Return to the app for details.</p>";

/// A PKCE verifier/challenge pair (RFC 7636, S256).
#[derive(Debug, Clone)]
pub struct Pkce {
    /// High-entropy secret sent with the code exchange.
    pub verifier: String,
    /// `BASE64URL(SHA256(verifier))`, sent with the authorize request.
    pub challenge: String,
}

impl Pkce {
    /// Generate a fresh pair (64-character verifier).
    pub fn generate() -> Self {
        let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

/// A one-shot HTTP listener on 127.0.0.1 that receives the OAuth redirect.
pub struct LoopbackRedirect {
    listener: TcpListener,
    port: u16,
}

impl LoopbackRedirect {
    /// Bind an ephemeral port on the loopback interface.
    pub async fn bind() -> KitResult<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| KitError::AuthFailed(format!("cannot open loopback listener: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| KitError::AuthFailed(format!("loopback address: {e}")))?
            .port();
        Ok(Self { listener, port })
    }

    /// Redirect URI to register in the authorize request.
    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Wait for the browser to hit the redirect URI and return the `code`.
    /// Requests without a `code` (e.g. `/favicon.ico`) are answered 404 and ignored.
    pub async fn wait_for_code(self, expected_state: &str, timeout: Duration) -> KitResult<String> {
        let wait = async {
            loop {
                let (mut stream, _) = self
                    .listener
                    .accept()
                    .await
                    .map_err(|e| KitError::AuthFailed(format!("loopback accept: {e}")))?;
                let request = read_request_head(&mut stream).await?;
                match parse_redirect(&request, expected_state) {
                    Ok(Some(code)) => {
                        respond(&mut stream, "200 OK", SUCCESS_HTML).await;
                        return Ok(code);
                    }
                    Ok(None) => respond(&mut stream, "404 Not Found", "").await,
                    Err(e) => {
                        respond(&mut stream, "400 Bad Request", FAILURE_HTML).await;
                        return Err(e);
                    }
                }
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| KitError::AuthFailed("browser sign-in timed out".into()))?
    }
}

async fn read_request_head(stream: &mut TcpStream) -> KitResult<String> {
    let mut buf = vec![0u8; 16 * 1024];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| KitError::AuthFailed(format!("loopback read: {e}")))?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Parse the request line of a redirect. `Ok(None)` means "not the redirect".
pub(crate) fn parse_redirect(request: &str, expected_state: &str) -> KitResult<Option<String>> {
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| KitError::AuthFailed("malformed redirect request".into()))?;
    let url = url::Url::parse(&format!("http://localhost{target}"))
        .map_err(|e| KitError::AuthFailed(format!("malformed redirect URL: {e}")))?;
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(error) = params.get("error") {
        let desc = params.get("error_description").cloned().unwrap_or_default();
        return Err(KitError::AuthFailed(format!("{error}: {desc}")));
    }
    let Some(code) = params.get("code") else {
        return Ok(None);
    };
    if params.get("state").map(String::as_str) != Some(expected_state) {
        return Err(KitError::AuthFailed("state mismatch in sign-in redirect".into()));
    }
    Ok(Some(code.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = Pkce::generate();
        assert_eq!(p.verifier.len(), 64);
        assert_eq!(p.challenge, URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes())));
    }

    #[test]
    fn parses_code_with_matching_state() {
        let req = "GET /?code=abc&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(parse_redirect(req, "xyz").unwrap().as_deref(), Some("abc"));
    }

    #[test]
    fn rejects_state_mismatch() {
        let req = "GET /?code=abc&state=evil HTTP/1.1\r\n\r\n";
        assert!(parse_redirect(req, "xyz").is_err());
    }

    #[test]
    fn surfaces_entra_error() {
        let req = "GET /?error=access_denied&error_description=User+cancelled HTTP/1.1\r\n\r\n";
        let err = parse_redirect(req, "xyz").unwrap_err();
        assert!(err.to_string().contains("User cancelled"));
    }

    #[test]
    fn ignores_favicon() {
        assert!(parse_redirect("GET /favicon.ico HTTP/1.1\r\n\r\n", "x").unwrap().is_none());
    }

    #[tokio::test]
    async fn listener_returns_code_from_real_request() {
        let redirect = LoopbackRedirect::bind().await.unwrap();
        let uri = redirect.redirect_uri();
        let port: u16 = uri.rsplit(':').next().unwrap().parse().unwrap();
        let waiter = tokio::spawn(redirect.wait_for_code("s1", Duration::from_secs(5)));
        let mut favicon = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        favicon.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").await.unwrap();
        let mut ok = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        ok.write_all(b"GET /?code=the-code&state=s1 HTTP/1.1\r\n\r\n").await.unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), "the-code");
    }
}
```

Modify `src/auth/mod.rs`:
```rust
//! Microsoft Entra ID sign-in and token exchange.

mod browser;
mod oauth;

pub use browser::{LoopbackRedirect, Pkce};
pub use oauth::{DeviceCodeResponse, OAuthClient, TokenResponse};
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p horizon-azure-kit auth::browser`
Expected: PASS, 6 tests.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: browser sign-in with PKCE and loopback redirect"
```

---

### Task 5: Claims and EntraSession — login, cached token_for, logout

**Files:**
- Create: `crates/horizon-azure-kit/src/auth/claims.rs`, `src/auth/session.rs`
- Modify: `src/auth/mod.rs`

**Interfaces:**
- Consumes:
  - `OAuthClient`, `TokenResponse`, `DeviceCodeResponse` (Task 3)
  - `Pkce`, `LoopbackRedirect` (Task 4)
  - `CredentialStore` (Task 2)
  - `AzureCloud` (Task 1)
- Produces:
  - `auth::AZURE_CLI_CLIENT_ID: &str`
  - `auth::SignIn { id: String, user_principal: String, tenant_id: String, display_name: Option<String>, cloud: AzureCloud }` (Serialize + Deserialize + Clone + PartialEq)
  - `auth::AccessToken { token: SecretString, expires_on: OffsetDateTime }` (Clone)
  - `auth::EntraSession`:
    - `new(cloud: AzureCloud, secrets: Arc<dyn CredentialStore>, key_prefix: impl Into<String>) -> Self`
    - `with_oauth(self, oauth: OAuthClient) -> Self`
    - `async login_device_code(&self, tenant: &str, on_prompt: impl FnOnce(&DeviceCodeResponse)) -> KitResult<SignIn>`
    - `async login_browser(&self, tenant: &str, open_url: impl FnOnce(&str)) -> KitResult<SignIn>`
    - `async token_for(&self, sign_in: &SignIn, tenant: Option<&str>, scope: &str) -> KitResult<AccessToken>`
    - `is_signed_in(&self, sign_in: &SignIn) -> KitResult<bool>`
    - `logout(&self, sign_in: &SignIn) -> KitResult<()>`
- Keychain key: `"{key_prefix}:entra:{sign_in.id}"`, value JSON `{"refresh_token","tenant_id","client_id"}`.

- [ ] **Step 1: Write claims.rs, ported from Arkived (`arkived/crates/arkived-core/src/auth/entra/claims.rs`)**

`src/auth/claims.rs`:
```rust
//! Identity claims from an Entra JWT. The signature is NOT verified: this only
//! labels a sign-in with names the authority just issued to us. Never use it
//! for authorization decisions.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

/// Best-effort identity fields.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IdentityClaims {
    /// `tid`
    pub tenant_id: Option<String>,
    /// `preferred_username`, else `upn`, `unique_name`, `email`.
    pub user_principal: Option<String>,
    /// `oid` — stable per user per tenant.
    pub object_id: Option<String>,
    /// `name`
    pub name: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawClaims {
    tid: Option<String>,
    oid: Option<String>,
    name: Option<String>,
    preferred_username: Option<String>,
    upn: Option<String>,
    unique_name: Option<String>,
    email: Option<String>,
}

/// Decode the payload segment of a JWT. Malformed input yields defaults.
pub fn parse_identity_claims(jwt: &str) -> IdentityClaims {
    let Some(payload) = jwt.split('.').nth(1) else {
        return IdentityClaims::default();
    };
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')) else {
        return IdentityClaims::default();
    };
    let Ok(raw) = serde_json::from_slice::<RawClaims>(&bytes) else {
        return IdentityClaims::default();
    };
    IdentityClaims {
        tenant_id: raw.tid,
        user_principal: raw.preferred_username.or(raw.upn).or(raw.unique_name).or(raw.email),
        object_id: raw.oid,
        name: raw.name,
    }
}

#[cfg(test)]
pub(crate) fn test_jwt(payload_json: &str) -> String {
    format!("h.{}.s", URL_SAFE_NO_PAD.encode(payload_json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_preferred_username_tenant_and_oid() {
        let c = parse_identity_claims(&test_jwt(r#"{"tid":"t1","oid":"o1","preferred_username":"a@x.io","name":"A"}"#));
        assert_eq!(c.tenant_id.as_deref(), Some("t1"));
        assert_eq!(c.object_id.as_deref(), Some("o1"));
        assert_eq!(c.user_principal.as_deref(), Some("a@x.io"));
    }

    #[test]
    fn falls_back_to_upn() {
        let c = parse_identity_claims(&test_jwt(r#"{"upn":"u@x.io"}"#));
        assert_eq!(c.user_principal.as_deref(), Some("u@x.io"));
    }

    #[test]
    fn malformed_yields_defaults() {
        assert_eq!(parse_identity_claims("nope"), IdentityClaims::default());
        assert_eq!(parse_identity_claims("a.!!!.c"), IdentityClaims::default());
    }
}
```

- [ ] **Step 2: Write session.rs with tests**

`src/auth/session.rs`:
```rust
//! A signed-in Entra identity: interactive login, cached per-scope access
//! tokens, and logout. Refresh tokens live in the credential store; access
//! tokens live only in memory.

use super::browser::{LoopbackRedirect, Pkce};
use super::claims::parse_identity_claims;
use super::oauth::{DeviceCodeResponse, OAuthClient, TokenResponse};
use crate::secrets::CredentialStore;
use crate::{AzureCloud, KitError, KitResult};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use time::OffsetDateTime;

/// Microsoft's public "Azure CLI" client id. Pre-consented in most tenants and
/// allows `http://localhost` loopback redirects.
pub const AZURE_CLI_CLIENT_ID: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";

const LOGIN_EXTRA_SCOPES: &str = "offline_access openid profile";
const REFRESH_MARGIN: time::Duration = time::Duration::minutes(5);

/// A remembered sign-in. Products persist this record themselves (it holds no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignIn {
    /// Stable id: the user's object id, or `tenant:upn` if absent.
    pub id: String,
    /// User principal name / e-mail.
    pub user_principal: String,
    /// Home tenant of the sign-in.
    pub tenant_id: String,
    /// Display name, if the token carried one.
    pub display_name: Option<String>,
    /// Cloud the sign-in belongs to.
    pub cloud: AzureCloud,
}

/// An access token for one scope.
#[derive(Debug, Clone)]
pub struct AccessToken {
    /// The bearer token.
    pub token: SecretString,
    /// Expiry instant (UTC).
    pub expires_on: OffsetDateTime,
}

#[derive(Serialize, Deserialize)]
struct StoredRefresh {
    refresh_token: String,
    tenant_id: String,
    client_id: String,
}

type CacheKey = (String, String, String); // (sign-in id, tenant, scope)

/// Entra sign-in manager for one cloud.
pub struct EntraSession {
    oauth: OAuthClient,
    cloud: AzureCloud,
    secrets: Arc<dyn CredentialStore>,
    key_prefix: String,
    cache: Mutex<HashMap<CacheKey, AccessToken>>,
}

impl EntraSession {
    /// Build a session manager. `key_prefix` namespaces keychain entries per product.
    pub fn new(cloud: AzureCloud, secrets: Arc<dyn CredentialStore>, key_prefix: impl Into<String>) -> Self {
        Self {
            oauth: OAuthClient::new(cloud.authority_host(), AZURE_CLI_CLIENT_ID),
            cloud,
            secrets,
            key_prefix: key_prefix.into(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Replace the OAuth client (tests, custom client ids).
    pub fn with_oauth(mut self, oauth: OAuthClient) -> Self {
        self.oauth = oauth;
        self
    }

    fn login_scope(&self) -> String {
        format!("{} {LOGIN_EXTRA_SCOPES}", self.cloud.arm_scope())
    }

    fn key(&self, sign_in_id: &str) -> String {
        format!("{}:entra:{sign_in_id}", self.key_prefix)
    }

    /// Device-code sign-in. `on_prompt` shows the code and URL to the user.
    pub async fn login_device_code(
        &self,
        tenant: &str,
        on_prompt: impl FnOnce(&DeviceCodeResponse),
    ) -> KitResult<SignIn> {
        let scope = self.login_scope();
        let dc = self.oauth.start_device_code(tenant, &scope).await?;
        on_prompt(&dc);
        let token = self.oauth.poll_device_code(tenant, &dc).await?;
        self.finish_login(tenant, token)
    }

    /// Browser sign-in (authorization code + PKCE). `open_url` must open the URL
    /// in the user's browser.
    pub async fn login_browser(&self, tenant: &str, open_url: impl FnOnce(&str)) -> KitResult<SignIn> {
        let redirect = LoopbackRedirect::bind().await?;
        let redirect_uri = redirect.redirect_uri();
        let pkce = Pkce::generate();
        let state = uuid::Uuid::new_v4().simple().to_string();
        let scope = self.login_scope();
        open_url(&self.oauth.authorize_url(tenant, &scope, &redirect_uri, &state, &pkce.challenge));
        let code = redirect.wait_for_code(&state, Duration::from_secs(300)).await?;
        let token = self
            .oauth
            .exchange_code(tenant, &code, &redirect_uri, &pkce.verifier, &scope)
            .await?;
        self.finish_login(tenant, token)
    }

    fn finish_login(&self, requested_tenant: &str, token: TokenResponse) -> KitResult<SignIn> {
        let mut claims = token.id_token.as_deref().map(parse_identity_claims).unwrap_or_default();
        if claims.user_principal.is_none() {
            claims = parse_identity_claims(&token.access_token);
        }
        let tenant_id = claims.tenant_id.unwrap_or_else(|| requested_tenant.to_string());
        let user_principal = claims.user_principal.unwrap_or_else(|| "unknown".to_string());
        let id = claims.object_id.unwrap_or_else(|| format!("{tenant_id}:{user_principal}"));
        let refresh_token = token
            .refresh_token
            .ok_or_else(|| KitError::AuthFailed("Entra returned no refresh token".into()))?;
        let stored = StoredRefresh {
            refresh_token,
            tenant_id: tenant_id.clone(),
            client_id: self.oauth.client_id().to_string(),
        };
        let json = serde_json::to_string(&stored).expect("StoredRefresh serializes");
        self.secrets.put(&self.key(&id), &SecretString::from(json))?;
        Ok(SignIn { id, user_principal, tenant_id, display_name: claims.name, cloud: self.cloud })
    }

    fn load(&self, sign_in: &SignIn) -> KitResult<StoredRefresh> {
        let secret = self.secrets.get(&self.key(&sign_in.id))?.ok_or_else(|| {
            KitError::NotSignedIn(format!("{} is not signed in", sign_in.user_principal))
        })?;
        serde_json::from_str(secret.expose_secret())
            .map_err(|e| KitError::SecretStore(format!("stored sign-in is unreadable: {e}")))
    }

    /// Access token for `scope` (e.g. `https://servicebus.azure.net/.default`),
    /// in `tenant` or the sign-in's home tenant. Served from memory until 5
    /// minutes before expiry.
    pub async fn token_for(&self, sign_in: &SignIn, tenant: Option<&str>, scope: &str) -> KitResult<AccessToken> {
        let stored = self.load(sign_in)?;
        let tenant = tenant.unwrap_or(&stored.tenant_id).to_string();
        let key = (sign_in.id.clone(), tenant.clone(), scope.to_string());
        if let Some(hit) = self.cache.lock().expect("token cache poisoned").get(&key) {
            if hit.expires_on - OffsetDateTime::now_utc() > REFRESH_MARGIN {
                return Ok(hit.clone());
            }
        }
        let resp = match self
            .oauth
            .refresh(&tenant, &stored.refresh_token, &format!("{scope} offline_access"))
            .await
        {
            Ok(resp) => resp,
            Err(KitError::NotSignedIn(msg)) => {
                return Err(KitError::NotSignedIn(format!("{}: {msg}", sign_in.user_principal)));
            }
            Err(e) => return Err(e),
        };
        if let Some(rotated) = resp.refresh_token.as_ref().filter(|rt| **rt != stored.refresh_token) {
            let updated = StoredRefresh { refresh_token: rotated.clone(), ..stored };
            let json = serde_json::to_string(&updated).expect("StoredRefresh serializes");
            self.secrets.put(&self.key(&sign_in.id), &SecretString::from(json))?;
        }
        let token = AccessToken {
            token: SecretString::from(resp.access_token),
            expires_on: OffsetDateTime::now_utc() + time::Duration::seconds(resp.expires_in as i64),
        };
        self.cache.lock().expect("token cache poisoned").insert(key, token.clone());
        Ok(token)
    }

    /// Whether a refresh token is stored for this sign-in.
    pub fn is_signed_in(&self, sign_in: &SignIn) -> KitResult<bool> {
        Ok(self.secrets.get(&self.key(&sign_in.id))?.is_some())
    }

    /// Forget the sign-in: delete its refresh token and cached access tokens.
    pub fn logout(&self, sign_in: &SignIn) -> KitResult<()> {
        self.secrets.delete(&self.key(&sign_in.id))?;
        self.cache.lock().expect("token cache poisoned").retain(|(id, _, _), _| id != &sign_in.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::claims::test_jwt;
    use crate::secrets::MemoryStore;
    use mockito::Matcher;

    fn session(server: &mockito::Server, store: Arc<MemoryStore>) -> EntraSession {
        EntraSession::new(AzureCloud::Public, store, "test")
            .with_oauth(OAuthClient::new(server.url(), "cid"))
    }

    async fn mock_device_login(server: &mut mockito::Server) -> (mockito::Mock, mockito::Mock) {
        let dc = server
            .mock("POST", "/organizations/oauth2/v2.0/devicecode")
            .with_status(200)
            .with_body(r#"{"user_code":"U","verification_uri":"V","device_code":"dc","expires_in":30,"interval":1,"message":"M"}"#)
            .create_async()
            .await;
        let id_token = test_jwt(r#"{"tid":"tenant-1","oid":"oid-1","preferred_username":"a@x.io","name":"A"}"#);
        let tok = server
            .mock("POST", "/organizations/oauth2/v2.0/token")
            .match_body(Matcher::UrlEncoded("device_code".into(), "dc".into()))
            .with_status(200)
            .with_body(format!(r#"{{"access_token":"arm-at","id_token":"{id_token}","refresh_token":"rt-1","expires_in":3600}}"#))
            .create_async()
            .await;
        (dc, tok)
    }

    #[tokio::test]
    async fn device_login_persists_refresh_token_and_returns_identity() {
        let mut server = mockito::Server::new_async().await;
        let _mocks = mock_device_login(&mut server).await;
        let store = Arc::new(MemoryStore::new());
        let s = session(&server, store.clone());
        let mut prompted = false;
        let sign_in = s.login_device_code("organizations", |_| prompted = true).await.unwrap();
        assert!(prompted);
        assert_eq!(sign_in.id, "oid-1");
        assert_eq!(sign_in.tenant_id, "tenant-1");
        assert_eq!(sign_in.user_principal, "a@x.io");
        assert!(store.get("test:entra:oid-1").unwrap().unwrap().expose_secret().contains("rt-1"));
        assert!(s.is_signed_in(&sign_in).unwrap());
    }

    #[tokio::test]
    async fn token_for_refreshes_once_then_serves_from_cache() {
        let mut server = mockito::Server::new_async().await;
        let _login = mock_device_login(&mut server).await;
        let refresh = server
            .mock("POST", "/tenant-1/oauth2/v2.0/token")
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
                Matcher::UrlEncoded("scope".into(), "https://servicebus.azure.net/.default offline_access".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"access_token":"sb-at","refresh_token":"rt-2","expires_in":3600}"#)
            .expect(1)
            .create_async()
            .await;
        let store = Arc::new(MemoryStore::new());
        let s = session(&server, store.clone());
        let sign_in = s.login_device_code("organizations", |_| {}).await.unwrap();
        let a = s.token_for(&sign_in, None, "https://servicebus.azure.net/.default").await.unwrap();
        let b = s.token_for(&sign_in, None, "https://servicebus.azure.net/.default").await.unwrap();
        assert_eq!(a.token.expose_secret(), "sb-at");
        assert_eq!(b.token.expose_secret(), "sb-at");
        refresh.assert_async().await;
        assert!(store.get("test:entra:oid-1").unwrap().unwrap().expose_secret().contains("rt-2"));
    }

    #[tokio::test]
    async fn token_for_after_revocation_reports_not_signed_in() {
        let mut server = mockito::Server::new_async().await;
        let _login = mock_device_login(&mut server).await;
        let _refresh = server
            .mock("POST", "/tenant-1/oauth2/v2.0/token")
            .with_status(400)
            .with_body(r#"{"error":"invalid_grant","error_description":"AADSTS50173: The provided grant has expired due to it being revoked."}"#)
            .create_async()
            .await;
        let s = session(&server, Arc::new(MemoryStore::new()));
        let sign_in = s.login_device_code("organizations", |_| {}).await.unwrap();
        let err = s.token_for(&sign_in, None, "scope").await.unwrap_err();
        assert!(matches!(&err, KitError::NotSignedIn(m) if m.contains("a@x.io")), "{err}");
    }

    #[tokio::test]
    async fn unknown_sign_in_is_not_signed_in() {
        let server = mockito::Server::new_async().await;
        let s = session(&server, Arc::new(MemoryStore::new()));
        let ghost = SignIn {
            id: "ghost".into(),
            user_principal: "g@x.io".into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        };
        assert!(matches!(s.token_for(&ghost, None, "s").await.unwrap_err(), KitError::NotSignedIn(_)));
    }

    #[tokio::test]
    async fn logout_forgets_refresh_token() {
        let mut server = mockito::Server::new_async().await;
        let _login = mock_device_login(&mut server).await;
        let store = Arc::new(MemoryStore::new());
        let s = session(&server, store.clone());
        let sign_in = s.login_device_code("organizations", |_| {}).await.unwrap();
        s.logout(&sign_in).unwrap();
        assert!(!s.is_signed_in(&sign_in).unwrap());
    }
}
```

Replace `src/auth/mod.rs`:
```rust
//! Microsoft Entra ID sign-in and token exchange.

mod browser;
mod claims;
mod oauth;
mod session;

pub use browser::{LoopbackRedirect, Pkce};
pub use claims::{IdentityClaims, parse_identity_claims};
pub use oauth::{DeviceCodeResponse, OAuthClient, TokenResponse};
pub use session::{AZURE_CLI_CLIENT_ID, AccessToken, EntraSession, SignIn};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p horizon-azure-kit auth`
Expected: PASS. All auth tests pass, including `token_for_refreshes_once_then_serves_from_cache` (the mock asserts exactly one refresh call) and `token_for_after_revocation_reports_not_signed_in`.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: EntraSession with device/browser login, cached per-scope tokens, logout"
```

---

### Task 6: SAS token signing

**Files:**
- Create: `crates/horizon-azure-kit/src/sas.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  - `horizon_azure_kit::sas::sign_sas(audience: &str, key_name: &str, key: &str, expiry_unix: i64) -> String`. It returns the full `SharedAccessSignature sr=…&sig=…&se=…&skn=…`.
  - `sas_token(audience: &str, key_name: &str, key: &str, ttl: std::time::Duration) -> (String, OffsetDateTime)`.

The test vector was computed independently with Python's `hmac`/`urllib.parse.quote(safe='')`: audience `https://myns.servicebus.windows.net/myqueue`, key name `RootManageSharedAccessKey`, key `abc123=`, expiry `1700000000`.

- [ ] **Step 1: Write sas.rs with the vector test**

`src/sas.rs`:
```rust
//! Shared Access Signature (SAS) tokens for Service Bus / Event Hubs style
//! resources. The key is used as raw UTF-8 bytes (NOT base64-decoded).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::Sha256;
use time::OffsetDateTime;

/// RFC 3986 unreserved characters stay literal; everything else is encoded.
const ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// Sign `audience` and return the complete `SharedAccessSignature ...` string.
pub fn sign_sas(audience: &str, key_name: &str, key: &str, expiry_unix: i64) -> String {
    let resource = utf8_percent_encode(audience, ENCODE_SET).to_string().to_lowercase();
    let string_to_sign = format!("{resource}\n{expiry_unix}");
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(string_to_sign.as_bytes());
    let signature = STANDARD.encode(mac.finalize().into_bytes());
    let signature = utf8_percent_encode(&signature, ENCODE_SET);
    let key_name = utf8_percent_encode(key_name, ENCODE_SET);
    format!("SharedAccessSignature sr={resource}&sig={signature}&se={expiry_unix}&skn={key_name}")
}

/// Sign `audience` valid for `ttl` from now. Returns the token and its expiry.
pub fn sas_token(audience: &str, key_name: &str, key: &str, ttl: std::time::Duration) -> (String, OffsetDateTime) {
    let expires = OffsetDateTime::now_utc() + ttl;
    (sign_sas(audience, key_name, key, expires.unix_timestamp()), expires)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_independent_vector() {
        let token = sign_sas(
            "https://myns.servicebus.windows.net/myqueue",
            "RootManageSharedAccessKey",
            "abc123=",
            1_700_000_000,
        );
        assert_eq!(
            token,
            "SharedAccessSignature sr=https%3a%2f%2fmyns.servicebus.windows.net%2fmyqueue&sig=6GlxVMpuwouj5YRodkN6TCe9lcUQs5VVsBX42Q0th90%3D&se=1700000000&skn=RootManageSharedAccessKey"
        );
    }

    #[test]
    fn sas_token_expiry_is_in_the_future() {
        let (token, expires) = sas_token("amqp://localhost/orders", "k", "SAS_KEY_VALUE", std::time::Duration::from_secs(3600));
        assert!(token.starts_with("SharedAccessSignature sr=amqp%3a%2f%2flocalhost%2forders&sig="));
        assert!(expires > OffsetDateTime::now_utc());
    }
}
```

Modify `src/lib.rs`. Add `pub mod sas;` after `pub mod error;`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p horizon-azure-kit sas`
Expected: PASS, 2 tests. If `matches_independent_vector` fails, the encoding set or lowercasing is wrong. Do not change the expected string.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: SAS token signing with independent test vector"
```

---

### Task 7: ARM client — tenants, subscriptions, resources by provider

**Files:**
- Create: `crates/horizon-azure-kit/src/arm.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `AzureCloud`, `KitError`.
- Produces:
  - `horizon_azure_kit::arm::{ArmClient, Tenant, Subscription}`.
  - `ArmClient`:
    - `new(cloud: AzureCloud, token: SecretString) -> Self`
    - `with_base(base: impl Into<String>, token: SecretString) -> Self`
    - `async list_tenants(&self) -> KitResult<Vec<Tenant>>`
    - `async list_subscriptions(&self) -> KitResult<Vec<Subscription>>`
    - `async list_by_provider<T: DeserializeOwned>(&self, subscription_id: &str, resource_type: &str, api_version: &str) -> KitResult<Vec<T>>`
  - `Tenant { tenant_id: String, display_name: Option<String>, default_domain: Option<String> }`
  - `Subscription { subscription_id: String, display_name: String, tenant_id: Option<String>, state: Option<String> }`
- HTTP mapping: 401 → `NotSignedIn`, 403 → `Forbidden`, 404 → `NotFound`, other non-2xx → `Remote`.

- [ ] **Step 1: Write arm.rs with tests**

`src/arm.rs`:
```rust
//! Minimal Azure Resource Manager client for discovery (read-only).

use crate::{AzureCloud, KitError, KitResult};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const TENANTS_API: &str = "2022-12-01";
const SUBSCRIPTIONS_API: &str = "2022-12-01";

/// A tenant the signed-in identity can access.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tenant {
    /// Tenant GUID.
    pub tenant_id: String,
    /// Display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Primary domain, e.g. `contoso.onmicrosoft.com`.
    #[serde(default)]
    pub default_domain: Option<String>,
}

/// A subscription visible to the token.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    /// Subscription GUID.
    pub subscription_id: String,
    /// Display name.
    pub display_name: String,
    /// Owning tenant.
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// `Enabled`, `Disabled`, `Warned`, ...
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    value: Vec<T>,
    #[serde(rename = "nextLink", default)]
    next_link: Option<String>,
}

/// Read-only ARM client bound to one access token.
pub struct ArmClient {
    http: reqwest::Client,
    base: String,
    token: SecretString,
}

impl ArmClient {
    /// Client for `cloud`'s ARM endpoint.
    pub fn new(cloud: AzureCloud, token: SecretString) -> Self {
        Self::with_base(cloud.arm_endpoint(), token)
    }

    /// Client for a custom base URL (tests, Azure Stack).
    pub fn with_base(base: impl Into<String>, token: SecretString) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.into().trim_end_matches('/').to_string(),
            token,
        }
    }

    /// Tenants the identity belongs to.
    pub async fn list_tenants(&self) -> KitResult<Vec<Tenant>> {
        self.get_paged(format!("{}/tenants?api-version={TENANTS_API}", self.base)).await
    }

    /// Subscriptions visible to the token.
    pub async fn list_subscriptions(&self) -> KitResult<Vec<Subscription>> {
        self.get_paged(format!("{}/subscriptions?api-version={SUBSCRIPTIONS_API}", self.base)).await
    }

    /// All resources of `resource_type` (e.g. `Microsoft.ServiceBus/namespaces`)
    /// in a subscription, deserialized as `T`.
    pub async fn list_by_provider<T: DeserializeOwned>(
        &self,
        subscription_id: &str,
        resource_type: &str,
        api_version: &str,
    ) -> KitResult<Vec<T>> {
        self.get_paged(format!(
            "{}/subscriptions/{subscription_id}/providers/{resource_type}?api-version={api_version}",
            self.base
        ))
        .await
    }

    async fn get_paged<T: DeserializeOwned>(&self, first: String) -> KitResult<Vec<T>> {
        let mut out = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next {
            let page: Page<T> = self.get_json(&url).await?;
            out.extend(page.value);
            next = page.next_link;
        }
        Ok(out)
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> KitResult<T> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(self.token.expose_secret())
            .send()
            .await
            .map_err(KitError::network)?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(KitError::network)?;
        match status {
            200..=299 => serde_json::from_str(&body)
                .map_err(|e| KitError::Remote { status, body: format!("ARM decode error: {e}") }),
            401 => Err(KitError::NotSignedIn(format!("ARM rejected the token: {body}"))),
            403 => Err(KitError::Forbidden(body)),
            404 => Err(KitError::NotFound(url.to_string())),
            _ => Err(KitError::Remote { status, body }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;

    #[tokio::test]
    async fn subscriptions_follow_next_link() {
        let mut server = mockito::Server::new_async().await;
        let next = format!("{}/subscriptions?api-version=2022-12-01&page=2", server.url());
        let _p1 = server
            .mock("GET", "/subscriptions")
            .match_query(Matcher::UrlEncoded("api-version".into(), "2022-12-01".into()))
            .match_header("authorization", "Bearer tok")
            .with_status(200)
            .with_body(format!(r#"{{"value":[{{"subscriptionId":"s1","displayName":"One","tenantId":"t"}}],"nextLink":"{next}"}}"#))
            .create_async()
            .await;
        let _p2 = server
            .mock("GET", "/subscriptions")
            .match_query(Matcher::UrlEncoded("page".into(), "2".into()))
            .with_status(200)
            .with_body(r#"{"value":[{"subscriptionId":"s2","displayName":"Two"}]}"#)
            .create_async()
            .await;
        let arm = ArmClient::with_base(server.url(), SecretString::from("tok"));
        let subs = arm.list_subscriptions().await.unwrap();
        assert_eq!(subs.iter().map(|s| s.subscription_id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
    }

    #[derive(Debug, Deserialize)]
    struct Ns {
        name: String,
    }

    #[tokio::test]
    async fn list_by_provider_builds_path() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/subscriptions/s1/providers/Microsoft.ServiceBus/namespaces")
            .match_query(Matcher::UrlEncoded("api-version".into(), "2024-01-01".into()))
            .with_status(200)
            .with_body(r#"{"value":[{"name":"sb1"}]}"#)
            .create_async()
            .await;
        let arm = ArmClient::with_base(server.url(), SecretString::from("tok"));
        let got: Vec<Ns> = arm.list_by_provider("s1", "Microsoft.ServiceBus/namespaces", "2024-01-01").await.unwrap();
        assert_eq!(got[0].name, "sb1");
    }

    #[tokio::test]
    async fn forbidden_maps_to_forbidden() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/tenants")
            .match_query(Matcher::Any)
            .with_status(403)
            .with_body("AuthorizationFailed")
            .create_async()
            .await;
        let arm = ArmClient::with_base(server.url(), SecretString::from("tok"));
        assert!(matches!(arm.list_tenants().await.unwrap_err(), KitError::Forbidden(_)));
    }
}
```

Modify `src/lib.rs`. Add `pub mod arm;` as the first module line.

- [ ] **Step 2: Run tests**

Run: `cargo test -p horizon-azure-kit arm`
Expected: PASS, 3 tests.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: ARM client for tenants, subscriptions, resources by provider"
```

---

### Task 8: Safety — confirmation policy and cancellation

**Files:**
- Create: `crates/horizon-azure-kit/src/safety.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  - `horizon_azure_kit::safety::{Origin, ConfirmRequest, Decision, Policy, AllowAll, DenyAll, require, CancellationToken}`.
  - `Origin::{Gui, Cli, Mcp}`.
  - `ConfirmRequest { verb: String, target: String, summary: String, item_count: Option<u64>, reversible: bool, production: bool, origin: Origin }`, plus `requires_typed_confirmation(&self) -> bool` (true when `production`).
  - `Decision::{Allow, Deny(String)}`.
  - `#[async_trait] trait Policy: Send + Sync { async fn confirm(&self, req: &ConfirmRequest) -> Decision; }`
  - `async fn require(policy: &dyn Policy, req: &ConfirmRequest) -> KitResult<()>`, which maps `Deny` to `PolicyDenied`.
  - `CancellationToken` re-exported from `tokio_util::sync`.
- Jobs and audit join this module in M2, when their first consumer exists.

- [ ] **Step 1: Write safety.rs with tests**

`src/safety.rs`:
```rust
//! Human-in-the-loop confirmation for destructive operations, and cancellation.
//!
//! Every surface (desktop, CLI, MCP) implements [`Policy`]. Core libraries call
//! [`require`] before any mutating broker or storage call.

use crate::{KitError, KitResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use tokio_util::sync::CancellationToken;

/// Which surface initiated an operation (shown in confirmations and audit logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Desktop app.
    Gui,
    /// Command line.
    Cli,
    /// An agent through MCP.
    Mcp,
}

/// A proposed destructive operation.
#[derive(Debug, Clone, Serialize)]
pub struct ConfirmRequest {
    /// Short verb, e.g. `purge`.
    pub verb: String,
    /// Exact target, e.g. `orders/$DeadLetterQueue`.
    pub target: String,
    /// Sentence naming the exact effect, shown to the human.
    pub summary: String,
    /// Number of items affected, if known.
    pub item_count: Option<u64>,
    /// Whether the effect can be undone.
    pub reversible: bool,
    /// The target belongs to a profile marked production.
    pub production: bool,
    /// Who asked.
    pub origin: Origin,
}

impl ConfirmRequest {
    /// Production targets require the human to type the target name
    /// (GUI), pass an explicit flag (CLI), or approve per call (MCP).
    pub fn requires_typed_confirmation(&self) -> bool {
        self.production
    }
}

/// A policy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Proceed.
    Allow,
    /// Refuse, with a reason shown to the caller.
    Deny(String),
}

/// Confirmation gate implemented by each surface.
#[async_trait]
pub trait Policy: Send + Sync {
    /// Ask whether `req` may proceed.
    async fn confirm(&self, req: &ConfirmRequest) -> Decision;
}

/// Refuses everything. The safe default.
pub struct DenyAll;

#[async_trait]
impl Policy for DenyAll {
    async fn confirm(&self, req: &ConfirmRequest) -> Decision {
        Decision::Deny(format!("{} on {} is not allowed in this context", req.verb, req.target))
    }
}

/// Allows everything. Tests and explicitly trusted automation only; never
/// for agent-facing surfaces.
pub struct AllowAll;

#[async_trait]
impl Policy for AllowAll {
    async fn confirm(&self, _req: &ConfirmRequest) -> Decision {
        Decision::Allow
    }
}

/// Ask `policy` and turn a denial into [`KitError::PolicyDenied`].
pub async fn require(policy: &dyn Policy, req: &ConfirmRequest) -> KitResult<()> {
    match policy.confirm(req).await {
        Decision::Allow => Ok(()),
        Decision::Deny(reason) => Err(KitError::PolicyDenied(reason)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(production: bool) -> ConfirmRequest {
        ConfirmRequest {
            verb: "purge".into(),
            target: "orders/$DeadLetterQueue".into(),
            summary: "Remove 12 messages from orders/$DeadLetterQueue".into(),
            item_count: Some(12),
            reversible: false,
            production,
            origin: Origin::Cli,
        }
    }

    #[tokio::test]
    async fn deny_all_maps_to_policy_denied() {
        let err = require(&DenyAll, &req(false)).await.unwrap_err();
        assert!(matches!(err, KitError::PolicyDenied(m) if m.contains("purge")));
    }

    #[tokio::test]
    async fn allow_all_allows() {
        require(&AllowAll, &req(false)).await.unwrap();
    }

    #[test]
    fn production_requires_typed_confirmation() {
        assert!(req(true).requires_typed_confirmation());
        assert!(!req(false).requires_typed_confirmation());
    }

    #[tokio::test]
    async fn cancellation_token_propagates_to_children() {
        let parent = CancellationToken::new();
        let child = parent.child_token();
        parent.cancel();
        child.cancelled().await;
        assert!(child.is_cancelled());
    }
}
```

Modify `src/lib.rs`. Add `pub mod safety;` after `pub mod sas;`.

- [ ] **Step 2: Run the full kit suite and clippy**

Run: `cargo test -p horizon-azure-kit && cargo clippy --all-targets --all-features -- -D warnings`
Expected: all tests PASS (keychain roundtrip ignored), and clippy is clean.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit: confirmation policy and cancellation"
```

---

### Task 9: Reusable desktop release workflow

**Files:**
- Create: `horizon-azure-kit/.github/workflows/tauri-release.yml`

**Interfaces:**
- Produces: a `workflow_call` workflow at `Horizon-Tech-doo/horizon-azure-kit/.github/workflows/tauri-release.yml@main`.
  - Inputs: `app-name` (string, required), `project-path` (default `app`), `cargo-workspace` (default `.`), `tag-prefix` (default `app-v`).
  - Secrets: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
  - It checks out the caller into `product/` and the kit into `horizon-azure-kit/` side by side, so path dependencies `../horizon-azure-kit/...` resolve.

- [ ] **Step 1: Write the workflow (adapted from `arkived/.github/workflows/release.yml`)**

`horizon-azure-kit/.github/workflows/tauri-release.yml`:
```yaml
name: Tauri desktop release (reusable)

on:
  workflow_call:
    inputs:
      app-name: { required: true, type: string }
      project-path: { required: false, type: string, default: app }
      cargo-workspace: { required: false, type: string, default: "." }
      tag-prefix: { required: false, type: string, default: app-v }
    secrets:
      TAURI_SIGNING_PRIVATE_KEY: { required: true }
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD: { required: true }

permissions:
  contents: write

jobs:
  build:
    name: ${{ matrix.label }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - { label: "macOS (universal)", platform: macos-latest, rust_targets: "aarch64-apple-darwin,x86_64-apple-darwin", args: "--target universal-apple-darwin" }
          - { label: "Linux (x86_64)", platform: ubuntu-22.04, rust_targets: x86_64-unknown-linux-gnu, args: "" }
          - { label: "Linux (aarch64)", platform: ubuntu-22.04-arm, rust_targets: aarch64-unknown-linux-gnu, args: "" }
          - { label: "Windows (x64)", platform: windows-latest, rust_targets: x86_64-pc-windows-msvc, args: "" }
          - { label: "Windows (ARM64)", platform: windows-latest, rust_targets: aarch64-pc-windows-msvc, args: "--target aarch64-pc-windows-msvc" }
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4
        with: { path: product }
      - uses: actions/checkout@v4
        with: { repository: Horizon-Tech-doo/horizon-azure-kit, path: horizon-azure-kit }
      - name: Install Linux dependencies
        if: startsWith(matrix.platform, 'ubuntu')
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf build-essential file libssl-dev libxdo-dev
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: "${{ matrix.rust_targets }}" }
      - uses: swatinem/rust-cache@v2
        with:
          workspaces: "product/${{ inputs.cargo-workspace }} -> target"
          key: ${{ matrix.label }}
      - name: Install frontend dependencies
        working-directory: product/${{ inputs.project-path }}
        run: npm ci
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          projectPath: product/${{ inputs.project-path }}
          tagName: ${{ inputs.tag-prefix }}__VERSION__
          releaseName: "${{ inputs.app-name }} v__VERSION__"
          releaseBody: |
            Desktop installers for ${{ inputs.app-name }}:
            - **Windows x64**: `*_x64-setup.exe` (NSIS) or `*_x64_en-US.msi`
            - **Windows ARM64**: `*_arm64-setup.exe`
            - **macOS (Intel + Apple Silicon)**: `*_universal.dmg`
            - **Linux x86_64**: `*_amd64.AppImage`, `*_amd64.deb`, `*.x86_64.rpm`
            - **Linux aarch64**: `*_aarch64.AppImage`, `*_arm64.deb`, `*.aarch64.rpm`
          releaseDraft: true
          prerelease: false
          includeUpdaterJson: true
          args: ${{ matrix.args }}
```

- [ ] **Step 2: Validate the YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/tauri-release.yml')); print('ok')"`
Expected: `ok`. If PyYAML is missing, run `ruby -ryaml -e "YAML.load_file('.github/workflows/tauri-release.yml'); puts 'ok'"` instead.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "Kit: reusable Tauri release workflow for all desktop targets"
```

---
## Part B — Dispatched

### Task 10: Dispatched workspace, error type, connection strings

**Files:**
- Create: `dispatched/Cargo.toml`, `dispatched/rust-toolchain.toml`, `dispatched/.gitignore`, `dispatched/LICENSE-APACHE`
- Create: `dispatched/crates/dispatched-core/Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/connstr.rs`

**Interfaces:**
- Consumes: `horizon_azure_kit::KitError`.
- Produces:
  - `dispatched_core::{Error, Result, ConnectionString}`. `Error` variants:
    - `Auth(String)`, `NotSignedIn(String)`, `Forbidden { needed: Option<String>, detail: String }`, `NotFound(String)`
    - `Throttled { retry_after: Option<Duration> }`, `Timeout(String)`, `Cancelled`, `Unsupported { capability: String }`
    - `Invalid(String)`, `Network(String)`, `Protocol(String)`
  - `impl From<KitError> for Error`.
  - `ConnectionString { host: String, port: Option<u16>, shared_access_key_name: Option<String>, shared_access_key: Option<SecretString>, shared_access_signature: Option<SecretString>, entity_path: Option<String>, use_development_emulator: bool }`, with `ConnectionString::parse(&str) -> Result<Self>`.

- [ ] **Step 1: Create the workspace**

`dispatched/Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/dispatched-core"]
default-members = ["crates/dispatched-core"]

[workspace.package]
version = "0.0.1"
edition = "2024"
rust-version = "1.88"
license = "Apache-2.0"
authors = ["Hamza Abdagić <hamza.abdagic@horizon-tech.io>", "Horizon Tech d.o.o."]
repository = "https://github.com/Horizon-Tech-doo/dispatched"
homepage = "https://github.com/Horizon-Tech-doo/dispatched"

[workspace.dependencies]
# Local checkout during development; the version is what crates.io publishing uses.
horizon-azure-kit = { version = "0.1", path = "../horizon-azure-kit/crates/horizon-azure-kit" }
dispatched-core = { version = "0.0.1", path = "crates/dispatched-core" }
anyhow = "1"
async-trait = "0.1"
base64 = "0.22"
clap = { version = "4.6", features = ["derive", "env"] }
dirs = "6"
fe2o3-amqp = { version = "0.18.2", features = ["rustls"] }
fe2o3-amqp-cbs = "0.18"
fe2o3-amqp-types = "0.18"
mockito = "1"
open = "5"
quick-xml = "0.36"
reqwest = { version = "0.13", default-features = false, features = ["rustls", "http2", "charset", "system-proxy", "json", "form", "query"] }
secrecy = "0.10"
serde = { version = "1", features = ["derive"] }
serde_amqp = { version = "0.18", features = ["time", "uuid"] }
serde_json = "1"
thiserror = "2"
time = { version = "0.3", features = ["serde", "serde-well-known", "formatting"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "sync"] }
tracing = "0.1"
url = "2"
uuid = { version = "1", features = ["v4", "serde"] }

[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```

`dispatched/rust-toolchain.toml`: the same content as the kit's (Task 1).

`dispatched/.gitignore`:
```
/target
node_modules/
/app/dist/
/app/src-tauri/gen/
/testing/emulator/.env
.DS_Store
.idea/
.vscode/
/.claude/
```

Run: `cp ../arkived/LICENSE-APACHE .`

`dispatched/crates/dispatched-core/Cargo.toml`:
```toml
[package]
name = "dispatched-core"
description = "Core library for Dispatched, a cross-platform Azure Service Bus explorer."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true

[features]
# Seeding helpers used by integration tests (send, dead-letter, drain).
test-support = []

[dependencies]
horizon-azure-kit = { workspace = true }
async-trait = { workspace = true }
base64 = { workspace = true }
fe2o3-amqp = { workspace = true }
fe2o3-amqp-cbs = { workspace = true }
fe2o3-amqp-types = { workspace = true }
quick-xml = { workspace = true }
reqwest = { workspace = true }
secrecy = { workspace = true }
serde = { workspace = true }
serde_amqp = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
time = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
url = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
mockito = { workspace = true }
tokio = { version = "1", features = ["full"] }
```

- [ ] **Step 2: Write error.rs**

`src/error.rs`:
```rust
//! Dispatched error type. Messages say what to do, not just what failed.

use horizon_azure_kit::KitError;
use std::time::Duration;

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors surfaced to every Dispatched surface (GUI, CLI, MCP).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Credentials were rejected.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// No usable Entra session; the user must sign in again.
    #[error("not signed in: {0}")]
    NotSignedIn(String),
    /// The identity lacks a right. `needed` names it when the service says so.
    #[error("access denied{}: {detail}", needed.as_deref().map(|n| format!(" (needs {n})")).unwrap_or_default())]
    Forbidden {
        /// Missing rights, e.g. `Manage,EntityRead` or `Listen`.
        needed: Option<String>,
        /// Service-provided detail.
        detail: String,
    },
    /// Entity or resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The service is throttling; retry later.
    #[error("the service is busy{}; try again", retry_after.map(|d| format!(" (retry after {}s)", d.as_secs())).unwrap_or_default())]
    Throttled {
        /// Server-suggested back-off.
        retry_after: Option<Duration>,
    },
    /// An operation exceeded its deadline.
    #[error("timed out: {0}")]
    Timeout(String),
    /// Cancelled by the user.
    #[error("cancelled")]
    Cancelled,
    /// The target (e.g. the emulator) does not support this capability.
    #[error("not supported here: {capability}")]
    Unsupported {
        /// What is unsupported.
        capability: String,
    },
    /// Invalid user input (connection string, entity path, ...).
    #[error("{0}")]
    Invalid(String),
    /// Network / transport failure.
    #[error("network error: {0}")]
    Network(String),
    /// Unexpected protocol response.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl From<KitError> for Error {
    fn from(e: KitError) -> Self {
        match e {
            KitError::AuthFailed(m) => Error::Auth(m),
            KitError::NotSignedIn(m) => Error::NotSignedIn(m),
            KitError::NotFound(m) => Error::NotFound(m),
            KitError::Forbidden(m) => Error::Forbidden { needed: None, detail: m },
            KitError::Network(m) => Error::Network(m),
            KitError::Remote { status, body } => Error::Protocol(format!("service returned {status}: {body}")),
            KitError::SecretStore(m) => Error::Auth(format!("secret store: {m}")),
            KitError::PolicyDenied(m) => Error::Invalid(format!("denied: {m}")),
            KitError::Cancelled => Error::Cancelled,
            KitError::Invalid(m) => Error::Invalid(m),
            other => Error::Protocol(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_names_the_missing_right() {
        let e = Error::Forbidden { needed: Some("Listen".into()), detail: "Unauthorized".into() };
        assert_eq!(e.to_string(), "access denied (needs Listen): Unauthorized");
    }

    #[test]
    fn kit_not_signed_in_is_preserved() {
        let e: Error = KitError::NotSignedIn("sign in again".into()).into();
        assert!(matches!(e, Error::NotSignedIn(_)));
    }
}
```

- [ ] **Step 3: Write connstr.rs with the Review Focus #5 tests**

`src/connstr.rs`:
```rust
//! Service Bus connection strings, as copied from the Azure portal, the
//! emulator docs, or other tools.

use crate::{Error, Result};
use secrecy::SecretString;

/// A parsed connection string. Unknown keys (e.g. `TransportType`) are ignored.
#[derive(Debug, Clone)]
pub struct ConnectionString {
    /// Namespace host, lowercased (`myns.servicebus.windows.net`, `localhost`).
    pub host: String,
    /// Explicit port in `Endpoint`, if any.
    pub port: Option<u16>,
    /// `SharedAccessKeyName`.
    pub shared_access_key_name: Option<String>,
    /// `SharedAccessKey`.
    pub shared_access_key: Option<SecretString>,
    /// Pre-signed `SharedAccessSignature`.
    pub shared_access_signature: Option<SecretString>,
    /// `EntityPath` (entity-scoped connection strings).
    pub entity_path: Option<String>,
    /// `UseDevelopmentEmulator=true`.
    pub use_development_emulator: bool,
}

impl ConnectionString {
    /// Parse `input`. Keys are case-insensitive; values keep their case.
    pub fn parse(input: &str) -> Result<Self> {
        let mut endpoint = None;
        let mut key_name = None;
        let mut key = None;
        let mut sas = None;
        let mut entity_path = None;
        let mut emulator = false;
        for segment in input.trim().split(';') {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            let Some((k, v)) = segment.split_once('=') else {
                return Err(Error::Invalid(format!(
                    "connection string segment `{}` has no `=`",
                    segment.chars().take(24).collect::<String>()
                )));
            };
            let v = v.trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "endpoint" => endpoint = Some(v.to_string()),
                "sharedaccesskeyname" => key_name = Some(v.to_string()),
                "sharedaccesskey" => key = Some(SecretString::from(v.to_string())),
                "sharedaccesssignature" => sas = Some(SecretString::from(v.to_string())),
                "entitypath" => entity_path = Some(v.to_string()).filter(|s| !s.is_empty()),
                "usedevelopmentemulator" => emulator = v.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
        let endpoint = endpoint.ok_or_else(|| Error::Invalid("connection string has no `Endpoint=`".into()))?;
        let url = url::Url::parse(&endpoint)
            .map_err(|e| Error::Invalid(format!("`Endpoint={endpoint}` is not a valid URL: {e}")))?;
        let host = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::Invalid(format!("`Endpoint={endpoint}` has no host")))?
            .to_ascii_lowercase();
        if sas.is_none() && (key_name.is_none() || key.is_none()) {
            return Err(Error::Invalid(
                "connection string needs `SharedAccessKeyName` and `SharedAccessKey`, or `SharedAccessSignature`".into(),
            ));
        }
        Ok(Self {
            host,
            port: url.port(),
            shared_access_key_name: key_name,
            shared_access_key: key,
            shared_access_signature: sas,
            entity_path,
            use_development_emulator: emulator,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn portal_namespace_string() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://MyNs.servicebus.windows.net/;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=abc+def/123=",
        )
        .unwrap();
        assert_eq!(cs.host, "myns.servicebus.windows.net");
        assert_eq!(cs.port, None);
        assert_eq!(cs.shared_access_key_name.as_deref(), Some("RootManageSharedAccessKey"));
        assert_eq!(cs.shared_access_key.unwrap().expose_secret(), "abc+def/123=");
        assert!(!cs.use_development_emulator);
    }

    #[test]
    fn trailing_semicolon_and_mixed_case_keys() {
        let cs = ConnectionString::parse(
            "endpoint=sb://ns.servicebus.windows.net;sharedaccesskeyname=k;SHAREDACCESSKEY=v;",
        )
        .unwrap();
        assert_eq!(cs.host, "ns.servicebus.windows.net");
    }

    #[test]
    fn entity_path_is_kept() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=orders",
        )
        .unwrap();
        assert_eq!(cs.entity_path.as_deref(), Some("orders"));
    }

    #[test]
    fn presigned_sas_is_enough() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessSignature=SharedAccessSignature sr=x&sig=y&se=1&skn=k",
        )
        .unwrap();
        assert!(cs.shared_access_signature.is_some());
        assert!(cs.shared_access_key.is_none());
    }

    #[test]
    fn emulator_with_port() {
        let cs = ConnectionString::parse(
            "Endpoint=sb://localhost:5300;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;",
        )
        .unwrap();
        assert_eq!(cs.host, "localhost");
        assert_eq!(cs.port, Some(5300));
        assert!(cs.use_development_emulator);
    }

    #[test]
    fn missing_endpoint_is_named() {
        let err = ConnectionString::parse("SharedAccessKeyName=k;SharedAccessKey=v").unwrap_err();
        assert!(err.to_string().contains("Endpoint="), "{err}");
    }

    #[test]
    fn missing_key_is_named() {
        let err = ConnectionString::parse("Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k").unwrap_err();
        assert!(err.to_string().contains("SharedAccessKey"), "{err}");
    }

    #[test]
    fn garbage_segment_is_rejected_without_echoing_secrets() {
        let err = ConnectionString::parse("Endpoint=sb://ns/;thisisnotakeyvaluepairbutlongsecretmaterial").unwrap_err();
        assert!(!err.to_string().contains("secretmaterial"), "{err}");
    }
}
```

`src/lib.rs`:
```rust
//! Core library for Dispatched, a cross-platform Azure Service Bus explorer.
//! All Service Bus behaviour lives here; the CLI, MCP server and desktop app
//! only translate input and render output.

#![deny(rust_2018_idioms, unsafe_code)]
#![warn(clippy::all)]

pub mod connstr;
pub mod error;

pub use connstr::ConnectionString;
pub use error::{Error, Result};
```

- [ ] **Step 4: Run tests**

Run: `df -h /System/Volumes/Data` (stop if under 30 GB free), then `cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && cargo test -p dispatched-core`
Expected: PASS, 10 tests. The first build compiles fe2o3 and the kit, which takes a while.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: workspace, Error, ConnectionString parsing"
```

---

### Task 11: Namespace endpoints, entity paths, token sources

**Files:**
- Create: `crates/dispatched-core/src/namespace.rs`, `src/tokens.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `ConnectionString`, `Error`/`Result` (Task 10); `horizon_azure_kit::{sas::sas_token, auth::{EntraSession, SignIn}}`.
- Produces:
  - `tokens::SERVICEBUS_SCOPE`.
  - `tokens::TokenKind::{Jwt, Sas}` with `cbs_type(self) -> &'static str`.
  - `tokens::ServiceBusToken { token: SecretString, kind: TokenKind, expires_on: OffsetDateTime }` with `http_authorization(&self) -> String`.
  - `#[async_trait] tokens::TokenSource: Send + Sync { async fn token(&self, audience: &str) -> Result<ServiceBusToken>; fn describe(&self) -> String; }`.
  - Implementations:
    - `SasKeySource::new(key_name, key: SecretString)`
    - `PresignedSasSource::parse(token: SecretString) -> Result<Self>`
    - `EntraSource::new(session: Arc<EntraSession>, sign_in: SignIn, tenant: Option<String>)`
  - `namespace::SubQueue::{Active, DeadLetter, TransferDeadLetter}`.
  - `namespace::EntityPath { entity: String, subscription: Option<String> }`, with `queue(&str)`, `subscription(&str, &str)`, `parse(&str) -> Result<Self>`, `base(&self) -> String`, `address(&self, SubQueue) -> String`.
  - `namespace::Namespace` (Clone):
    - constructors: `new(host, Arc<dyn TokenSource>)`, `emulator(host, EmulatorPorts, Arc<dyn TokenSource>)`, `from_connection_string(&ConnectionString) -> Result<Self>`
    - accessors: `host()`, `is_emulator()`, `tokens()`, `entity_path()`
    - endpoints: `amqp_url()`, `amqp_audience(&str)`, `management_base()`
  - `namespace::EmulatorPorts { amqp: u16, management: u16 }`.

- [ ] **Step 1: Write tokens.rs with tests**

`src/tokens.rs`:
```rust
//! Credentials for Service Bus, as tokens per audience (CBS for AMQP,
//! `Authorization` header for the management API).

use crate::{Error, Result};
use async_trait::async_trait;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;

/// Entra scope for Service Bus data and management-plane (ATOM) access.
pub const SERVICEBUS_SCOPE: &str = "https://servicebus.azure.net/.default";

const SAS_TTL: Duration = Duration::from_secs(60 * 60);

/// The kind of token, which decides the CBS token type and HTTP header format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Entra access token.
    Jwt,
    /// Shared Access Signature.
    Sas,
}

impl TokenKind {
    /// CBS `type` application property for `put-token`.
    pub fn cbs_type(self) -> &'static str {
        match self {
            TokenKind::Jwt => "jwt",
            TokenKind::Sas => "servicebus.windows.net:sastoken",
        }
    }
}

/// A token valid for one audience.
#[derive(Clone)]
pub struct ServiceBusToken {
    /// Raw token (`eyJ...` or `SharedAccessSignature sr=...`).
    pub token: SecretString,
    /// Token kind.
    pub kind: TokenKind,
    /// Expiry (UTC).
    pub expires_on: OffsetDateTime,
}

impl ServiceBusToken {
    /// Value for an HTTP `Authorization` header.
    pub fn http_authorization(&self) -> String {
        match self.kind {
            TokenKind::Jwt => format!("Bearer {}", self.token.expose_secret()),
            TokenKind::Sas => self.token.expose_secret().to_string(),
        }
    }
}

/// Produces tokens for Service Bus audiences.
#[async_trait]
pub trait TokenSource: Send + Sync {
    /// Token for `audience` (e.g. `amqps://ns.servicebus.windows.net/orders`).
    async fn token(&self, audience: &str) -> Result<ServiceBusToken>;
    /// Human description of the auth path, e.g. "SAS key RootManageSharedAccessKey".
    fn describe(&self) -> String;
}

/// Signs SAS tokens locally from a key.
pub struct SasKeySource {
    key_name: String,
    key: SecretString,
}

impl SasKeySource {
    /// Build from a key name and key.
    pub fn new(key_name: impl Into<String>, key: SecretString) -> Self {
        Self { key_name: key_name.into(), key }
    }
}

#[async_trait]
impl TokenSource for SasKeySource {
    async fn token(&self, audience: &str) -> Result<ServiceBusToken> {
        let (token, expires_on) =
            horizon_azure_kit::sas::sas_token(audience, &self.key_name, self.key.expose_secret(), SAS_TTL);
        Ok(ServiceBusToken { token: SecretString::from(token), kind: TokenKind::Sas, expires_on })
    }

    fn describe(&self) -> String {
        format!("SAS key {}", self.key_name)
    }
}

/// A pre-signed SAS from a connection string. Valid until its `se=` expiry.
pub struct PresignedSasSource {
    token: SecretString,
    expires_on: OffsetDateTime,
}

impl PresignedSasSource {
    /// Parse `SharedAccessSignature sr=..&sig=..&se=..&skn=..` (the prefix is optional).
    pub fn parse(token: SecretString) -> Result<Self> {
        let raw = token.expose_secret().trim();
        let full = if raw.starts_with("SharedAccessSignature ") {
            raw.to_string()
        } else {
            format!("SharedAccessSignature {raw}")
        };
        let se = full
            .trim_start_matches("SharedAccessSignature ")
            .split('&')
            .find_map(|kv| kv.strip_prefix("se="))
            .ok_or_else(|| Error::Invalid("SharedAccessSignature has no `se=` expiry".into()))?;
        let unix: i64 = se
            .parse()
            .map_err(|_| Error::Invalid(format!("SharedAccessSignature expiry `{se}` is not a number")))?;
        let expires_on = OffsetDateTime::from_unix_timestamp(unix)
            .map_err(|_| Error::Invalid(format!("SharedAccessSignature expiry `{se}` is out of range")))?;
        Ok(Self { token: SecretString::from(full), expires_on })
    }
}

#[async_trait]
impl TokenSource for PresignedSasSource {
    async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
        if self.expires_on <= OffsetDateTime::now_utc() {
            return Err(Error::Auth("the SharedAccessSignature in this connection string has expired".into()));
        }
        Ok(ServiceBusToken { token: self.token.clone(), kind: TokenKind::Sas, expires_on: self.expires_on })
    }

    fn describe(&self) -> String {
        "pre-signed SAS".into()
    }
}

/// Entra ID tokens from a kit `EntraSession`.
pub struct EntraSource {
    session: Arc<EntraSession>,
    sign_in: SignIn,
    tenant: Option<String>,
}

impl EntraSource {
    /// Use `sign_in`, optionally in a tenant other than its home tenant.
    pub fn new(session: Arc<EntraSession>, sign_in: SignIn, tenant: Option<String>) -> Self {
        Self { session, sign_in, tenant }
    }
}

#[async_trait]
impl TokenSource for EntraSource {
    async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
        let t = self
            .session
            .token_for(&self.sign_in, self.tenant.as_deref(), SERVICEBUS_SCOPE)
            .await?;
        Ok(ServiceBusToken { token: t.token, kind: TokenKind::Jwt, expires_on: t.expires_on })
    }

    fn describe(&self) -> String {
        format!("Entra ID {}", self.sign_in.user_principal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sas_key_source_signs_audience() {
        let src = SasKeySource::new("RootManageSharedAccessKey", SecretString::from("SAS_KEY_VALUE"));
        let t = src.token("amqp://localhost/orders").await.unwrap();
        assert_eq!(t.kind, TokenKind::Sas);
        assert!(t.http_authorization().starts_with("SharedAccessSignature sr=amqp%3a%2f%2flocalhost%2forders&"));
        assert_eq!(src.describe(), "SAS key RootManageSharedAccessKey");
    }

    #[tokio::test]
    async fn presigned_sas_parses_expiry_and_rejects_expired() {
        let future = OffsetDateTime::now_utc().unix_timestamp() + 600;
        let ok = PresignedSasSource::parse(SecretString::from(format!("sr=x&sig=y&se={future}&skn=k"))).unwrap();
        let t = ok.token("ignored").await.unwrap();
        assert!(t.http_authorization().starts_with("SharedAccessSignature sr=x"));
        let expired = PresignedSasSource::parse(SecretString::from("SharedAccessSignature sr=x&sig=y&se=1&skn=k")).unwrap();
        assert!(matches!(expired.token("a").await.unwrap_err(), Error::Auth(_)));
    }

    #[test]
    fn cbs_types() {
        assert_eq!(TokenKind::Jwt.cbs_type(), "jwt");
        assert_eq!(TokenKind::Sas.cbs_type(), "servicebus.windows.net:sastoken");
    }
}
```

- [ ] **Step 2: Write namespace.rs with tests**

`src/namespace.rs`:
```rust
//! A Service Bus namespace: endpoints, audiences and credentials.

use crate::connstr::ConnectionString;
use crate::tokens::{PresignedSasSource, SasKeySource, TokenSource};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Which queue of an entity to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubQueue {
    /// The entity itself.
    Active,
    /// `$DeadLetterQueue`.
    DeadLetter,
    /// `$Transfer/$DeadLetterQueue`.
    TransferDeadLetter,
}

/// A queue, or a topic subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityPath {
    /// Queue or topic name.
    pub entity: String,
    /// Subscription name when `entity` is a topic.
    pub subscription: Option<String>,
}

impl EntityPath {
    /// A queue.
    pub fn queue(name: &str) -> Self {
        Self { entity: name.to_string(), subscription: None }
    }

    /// A topic subscription.
    pub fn subscription(topic: &str, subscription: &str) -> Self {
        Self { entity: topic.to_string(), subscription: Some(subscription.to_string()) }
    }

    /// Parse `orders` or `events/subscriptions/all` (`Subscriptions` in any case).
    pub fn parse(input: &str) -> Result<Self> {
        let parts: Vec<&str> = input.trim().trim_matches('/').split('/').collect();
        match parts.as_slice() {
            [q] if !q.is_empty() => Ok(Self::queue(q)),
            [t, s, sub] if s.eq_ignore_ascii_case("subscriptions") && !t.is_empty() && !sub.is_empty() => {
                Ok(Self::subscription(t, sub))
            }
            _ => Err(Error::Invalid(format!(
                "`{input}` is not an entity; use `queue` or `topic/subscriptions/subscription`"
            ))),
        }
    }

    /// Canonical path without sub-queue.
    pub fn base(&self) -> String {
        match &self.subscription {
            None => self.entity.clone(),
            Some(sub) => format!("{}/Subscriptions/{sub}", self.entity),
        }
    }

    /// AMQP address including the sub-queue suffix.
    pub fn address(&self, sub_queue: SubQueue) -> String {
        match sub_queue {
            SubQueue::Active => self.base(),
            SubQueue::DeadLetter => format!("{}/$DeadLetterQueue", self.base()),
            SubQueue::TransferDeadLetter => format!("{}/$Transfer/$DeadLetterQueue", self.base()),
        }
    }
}

impl fmt::Display for EntityPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base())
    }
}

/// Emulator ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmulatorPorts {
    /// Plain AMQP port (5672).
    pub amqp: u16,
    /// HTTP management port (5300).
    pub management: u16,
}

/// A namespace plus the credentials to reach it.
#[derive(Clone)]
pub struct Namespace {
    host: String,
    emulator: Option<EmulatorPorts>,
    tokens: Arc<dyn TokenSource>,
    entity_path: Option<String>,
}

impl fmt::Debug for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Namespace")
            .field("host", &self.host)
            .field("emulator", &self.emulator)
            .field("auth", &self.tokens.describe())
            .field("entity_path", &self.entity_path)
            .finish()
    }
}

impl Namespace {
    /// A cloud namespace (`myns.servicebus.windows.net`).
    pub fn new(host: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self { host: host.into().to_ascii_lowercase(), emulator: None, tokens, entity_path: None }
    }

    /// The local Service Bus Emulator.
    pub fn emulator(host: impl Into<String>, ports: EmulatorPorts, tokens: Arc<dyn TokenSource>) -> Self {
        Self { host: host.into().to_ascii_lowercase(), emulator: Some(ports), tokens, entity_path: None }
    }

    /// Build from a parsed connection string.
    pub fn from_connection_string(cs: &ConnectionString) -> Result<Self> {
        let tokens: Arc<dyn TokenSource> = match (&cs.shared_access_signature, &cs.shared_access_key_name, &cs.shared_access_key) {
            (Some(sas), _, _) => Arc::new(PresignedSasSource::parse(sas.clone())?),
            (None, Some(name), Some(key)) => Arc::new(SasKeySource::new(name.clone(), key.clone())),
            _ => return Err(Error::Invalid("connection string has no usable credentials".into())),
        };
        let mut ns = if cs.use_development_emulator {
            Self::emulator(
                cs.host.clone(),
                EmulatorPorts { amqp: 5672, management: cs.port.unwrap_or(5300) },
                tokens,
            )
        } else {
            Self::new(cs.host.clone(), tokens)
        };
        ns.entity_path = cs.entity_path.clone();
        Ok(ns)
    }

    /// Namespace host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Whether this is the local emulator.
    pub fn is_emulator(&self) -> bool {
        self.emulator.is_some()
    }

    /// Credential source.
    pub fn tokens(&self) -> &Arc<dyn TokenSource> {
        &self.tokens
    }

    /// `EntityPath` from an entity-scoped connection string.
    pub fn entity_path(&self) -> Option<&str> {
        self.entity_path.as_deref()
    }

    /// AMQP connection URL.
    pub fn amqp_url(&self) -> String {
        match self.emulator {
            Some(p) => format!("amqp://{}:{}", self.host, p.amqp),
            None => format!("amqps://{}:5671", self.host),
        }
    }

    /// CBS audience / link address URI for `address` (e.g. `orders/$DeadLetterQueue`).
    pub fn amqp_audience(&self, address: &str) -> String {
        let scheme = if self.is_emulator() { "amqp" } else { "amqps" };
        format!("{scheme}://{}/{address}", self.host)
    }

    /// Base URL of the ATOM management API, without trailing slash.
    pub fn management_base(&self) -> String {
        match self.emulator {
            Some(p) => format!("http://{}:{}", self.host, p.management),
            None => format!("https://{}", self.host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emulator_ns() -> Namespace {
        let cs = ConnectionString::parse(
            "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;",
        )
        .unwrap();
        Namespace::from_connection_string(&cs).unwrap()
    }

    #[test]
    fn emulator_endpoints() {
        let ns = emulator_ns();
        assert!(ns.is_emulator());
        assert_eq!(ns.amqp_url(), "amqp://localhost:5672");
        assert_eq!(ns.management_base(), "http://localhost:5300");
        assert_eq!(ns.amqp_audience("orders"), "amqp://localhost/orders");
    }

    #[test]
    fn cloud_endpoints() {
        let cs = ConnectionString::parse("Endpoint=sb://MyNs.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v").unwrap();
        let ns = Namespace::from_connection_string(&cs).unwrap();
        assert_eq!(ns.amqp_url(), "amqps://myns.servicebus.windows.net:5671");
        assert_eq!(ns.management_base(), "https://myns.servicebus.windows.net");
        assert_eq!(ns.amqp_audience("orders/$DeadLetterQueue"), "amqps://myns.servicebus.windows.net/orders/$DeadLetterQueue");
    }

    #[test]
    fn entity_path_parse_and_addresses() {
        let q = EntityPath::parse("orders").unwrap();
        assert_eq!(q.address(SubQueue::DeadLetter), "orders/$DeadLetterQueue");
        let s = EntityPath::parse("events/subscriptions/all").unwrap();
        assert_eq!(s.base(), "events/Subscriptions/all");
        assert_eq!(s.address(SubQueue::TransferDeadLetter), "events/Subscriptions/all/$Transfer/$DeadLetterQueue");
        assert!(EntityPath::parse("a/b").is_err());
        assert!(EntityPath::parse("").is_err());
    }

    #[test]
    fn debug_never_prints_secrets() {
        let dbg = format!("{:?}", emulator_ns());
        assert!(!dbg.contains("SAS_KEY_VALUE"));
        assert!(dbg.contains("SAS key RootManageSharedAccessKey"));
    }
}
```

Modify `src/lib.rs`. Replace the module list and re-exports with:
```rust
pub mod connstr;
pub mod error;
pub mod namespace;
pub mod tokens;

pub use connstr::ConnectionString;
pub use error::{Error, Result};
pub use namespace::{EmulatorPorts, EntityPath, Namespace, SubQueue};
pub use tokens::{ServiceBusToken, TokenKind, TokenSource};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p dispatched-core`
Expected: PASS. 17 tests: Task 10's 10 tests plus 7 new.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: Namespace endpoints, EntityPath, SAS/presigned/Entra token sources"
```

---

### Task 12: ATOM management client — list queues/topics/subscriptions with counts

**Files:**
- Create: `crates/dispatched-core/src/mgmt/mod.rs`, `src/mgmt/xml.rs`, `src/mgmt/models.rs`
- Create: `crates/dispatched-core/tests/fixtures/queue_feed.xml`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Namespace`, `TokenSource`, `Error`.
- Produces:
  - `mgmt::ManagementClient`:
    - `new(&Namespace) -> Self`
    - `with_base(base: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self`
    - `async list_queues(&self) -> Result<Vec<QueueInfo>>`
    - `async list_topics(&self) -> Result<Vec<TopicInfo>>`
    - `async list_subscriptions(&self, topic: &str) -> Result<Vec<SubscriptionInfo>>`
    - `async get_queue(&self, name: &str) -> Result<QueueInfo>`
  - `mgmt::{EntityCounts, QueueInfo, TopicInfo, SubscriptionInfo}` (Serialize).
  - `mgmt::needed_claims(detail: &str) -> Option<String>`, which Task 13 reuses.
  - `counts: Option<EntityCounts>` is `None` when the service omits `CountDetails` (possible on the emulator).

- [ ] **Step 1: Add the fixture**

`crates/dispatched-core/tests/fixtures/queue_feed.xml` is the real ATOM shape, from the Azure SDK test fixtures:
```xml
<feed xmlns="http://www.w3.org/2005/Atom">
  <title type="text">Queues</title>
  <id>feed-id</id>
  <updated>2020-12-05T07:17:21Z</updated>
  <entry xml:base="https://foo.servicebus.windows.net/$Resources/queues?api-version=2024-05">
    <id>https://foo.servicebus.windows.net/orders?api-version=2024-05</id>
    <title type="text">orders</title>
    <content type="application/xml">
      <QueueDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">
        <LockDuration>PT1M</LockDuration>
        <MaxSizeInMegabytes>1024</MaxSizeInMegabytes>
        <RequiresSession>false</RequiresSession>
        <DeadLetteringOnMessageExpiration>true</DeadLetteringOnMessageExpiration>
        <MaxDeliveryCount>10</MaxDeliveryCount>
        <SizeInBytes>542</SizeInBytes>
        <MessageCount>4</MessageCount>
        <Status>Active</Status>
        <CountDetails xmlns:d2p1="http://schemas.microsoft.com/netservices/2011/06/servicebus">
          <d2p1:ActiveMessageCount>3</d2p1:ActiveMessageCount>
          <d2p1:DeadLetterMessageCount>1</d2p1:DeadLetterMessageCount>
          <d2p1:ScheduledMessageCount>0</d2p1:ScheduledMessageCount>
          <d2p1:TransferMessageCount>0</d2p1:TransferMessageCount>
          <d2p1:TransferDeadLetterMessageCount>0</d2p1:TransferDeadLetterMessageCount>
        </CountDetails>
        <ForwardTo i:nil="true"/>
      </QueueDescription>
    </content>
  </entry>
  <entry>
    <title type="text">sessions-q</title>
    <content type="application/xml">
      <QueueDescription xmlns="http://schemas.microsoft.com/netservices/2010/10/servicebus/connect">
        <RequiresSession>true</RequiresSession>
        <Status>Active</Status>
      </QueueDescription>
    </content>
  </entry>
</feed>
```

- [ ] **Step 2: Write xml.rs (lenient local-name tree)**

`src/mgmt/xml.rs`:
```rust
//! A tiny lenient XML tree keyed by local names. ATOM responses vary in
//! namespace prefixes, so matching on local names is the robust choice.

use crate::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

/// An element: local name, concatenated text, children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    /// Local name (no prefix).
    pub name: String,
    /// Text content (trimmed, unescaped).
    pub text: String,
    /// Child elements in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// First child with local name `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// All children with local name `name`.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Descend through `path` of local names.
    pub fn path(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |node, name| node.child(name))
    }

    /// Non-empty text of child `name`.
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|n| n.text.as_str()).filter(|t| !t.is_empty())
    }
}

/// Parse `xml` into its root element.
pub fn parse(xml: &str) -> Result<Node> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack = vec![Node { name: "#document".into(), ..Node::default() }];
    loop {
        match reader.read_event().map_err(|e| Error::Protocol(format!("invalid XML: {e}")))? {
            Event::Start(e) => stack.push(Node {
                name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                ..Node::default()
            }),
            Event::Empty(e) => {
                let node = Node { name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(), ..Node::default() };
                stack.last_mut().expect("document node").children.push(node);
            }
            Event::Text(t) => {
                let text = t.unescape().map_err(|e| Error::Protocol(format!("invalid XML text: {e}")))?;
                stack.last_mut().expect("document node").text.push_str(&text);
            }
            Event::CData(c) => {
                stack.last_mut().expect("document node").text.push_str(&String::from_utf8_lossy(&c));
            }
            Event::End(_) => {
                let node = stack.pop().expect("balanced end");
                stack
                    .last_mut()
                    .ok_or_else(|| Error::Protocol("unbalanced XML".into()))?
                    .children
                    .push(node);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        return Err(Error::Protocol("truncated XML document".into()));
    }
    stack
        .pop()
        .and_then(|doc| doc.children.into_iter().next())
        .ok_or_else(|| Error::Protocol("empty XML document".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_names_ignore_prefixes() {
        let root = parse(r#"<a xmlns:p="x"><p:b>1</p:b><c/></a>"#).unwrap();
        assert_eq!(root.name, "a");
        assert_eq!(root.text_of("b"), Some("1"));
        assert!(root.child("c").is_some());
    }

    #[test]
    fn truncated_is_an_error() {
        assert!(parse("<a><b>").is_err());
    }
}
```

- [ ] **Step 3: Write models.rs**

`src/mgmt/models.rs`:
```rust
//! Entity models parsed from ATOM entries.

use super::xml::Node;
use serde::Serialize;
use std::str::FromStr;

/// Message counts from `CountDetails`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EntityCounts {
    /// Active messages.
    pub active: u64,
    /// Dead-lettered messages.
    pub dead_letter: u64,
    /// Scheduled messages.
    pub scheduled: u64,
    /// Messages pending transfer.
    pub transfer: u64,
    /// Transfer dead-letter messages.
    pub transfer_dead_letter: u64,
}

/// A queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueueInfo {
    /// Queue name.
    pub name: String,
    /// `Active`, `Disabled`, `SendDisabled`, `ReceiveDisabled`.
    pub status: Option<String>,
    /// Counts; `None` if the service did not report them.
    pub counts: Option<EntityCounts>,
    /// Size in bytes.
    pub size_in_bytes: Option<u64>,
    /// Session-enabled.
    pub requires_session: bool,
    /// Max delivery count.
    pub max_delivery_count: Option<u32>,
    /// ISO-8601 lock duration (e.g. `PT1M`).
    pub lock_duration: Option<String>,
    /// Auto-forward target.
    pub forward_to: Option<String>,
}

/// A topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopicInfo {
    /// Topic name.
    pub name: String,
    /// Status.
    pub status: Option<String>,
    /// Size in bytes.
    pub size_in_bytes: Option<u64>,
    /// Number of subscriptions.
    pub subscription_count: Option<u32>,
}

/// A topic subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionInfo {
    /// Parent topic.
    pub topic: String,
    /// Subscription name.
    pub name: String,
    /// Status.
    pub status: Option<String>,
    /// Counts; `None` if not reported.
    pub counts: Option<EntityCounts>,
    /// Session-enabled.
    pub requires_session: bool,
    /// Max delivery count.
    pub max_delivery_count: Option<u32>,
    /// Auto-forward target.
    pub forward_to: Option<String>,
}

/// Entries of a feed, or the root itself when it is a single entry.
pub(crate) fn entries(root: &Node) -> Vec<&Node> {
    match root.name.as_str() {
        "feed" => root.children_named("entry").collect(),
        "entry" => vec![root],
        _ => Vec::new(),
    }
}

fn description<'a>(entry: &'a Node, kind: &str) -> Option<&'a Node> {
    entry.path(&["content", kind])
}

fn opt<T: FromStr>(node: &Node, name: &str) -> Option<T> {
    node.text_of(name).and_then(|t| t.parse().ok())
}

fn flag(node: &Node, name: &str) -> bool {
    node.text_of(name).is_some_and(|t| t.eq_ignore_ascii_case("true"))
}

fn text(node: &Node, name: &str) -> Option<String> {
    node.text_of(name).map(str::to_string)
}

fn counts(desc: &Node) -> Option<EntityCounts> {
    let c = desc.child("CountDetails")?;
    Some(EntityCounts {
        active: opt(c, "ActiveMessageCount").unwrap_or_default(),
        dead_letter: opt(c, "DeadLetterMessageCount").unwrap_or_default(),
        scheduled: opt(c, "ScheduledMessageCount").unwrap_or_default(),
        transfer: opt(c, "TransferMessageCount").unwrap_or_default(),
        transfer_dead_letter: opt(c, "TransferDeadLetterMessageCount").unwrap_or_default(),
    })
}

pub(crate) fn queue_from_entry(entry: &Node) -> Option<QueueInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = description(entry, "QueueDescription")?;
    Some(QueueInfo {
        name,
        status: text(d, "Status"),
        counts: counts(d),
        size_in_bytes: opt(d, "SizeInBytes"),
        requires_session: flag(d, "RequiresSession"),
        max_delivery_count: opt(d, "MaxDeliveryCount"),
        lock_duration: text(d, "LockDuration"),
        forward_to: text(d, "ForwardTo"),
    })
}

pub(crate) fn topic_from_entry(entry: &Node) -> Option<TopicInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = description(entry, "TopicDescription")?;
    Some(TopicInfo {
        name,
        status: text(d, "Status"),
        size_in_bytes: opt(d, "SizeInBytes"),
        subscription_count: opt(d, "SubscriptionCount"),
    })
}

pub(crate) fn subscription_from_entry(topic: &str, entry: &Node) -> Option<SubscriptionInfo> {
    let name = entry.text_of("title")?.to_string();
    let d = description(entry, "SubscriptionDescription")?;
    Some(SubscriptionInfo {
        topic: topic.to_string(),
        name,
        status: text(d, "Status"),
        counts: counts(d),
        requires_session: flag(d, "RequiresSession"),
        max_delivery_count: opt(d, "MaxDeliveryCount"),
        forward_to: text(d, "ForwardTo"),
    })
}
```

- [ ] **Step 4: Write the client with the Review Focus #4 tests**

`src/mgmt/mod.rs`:
```rust
//! Service Bus management API (ATOM over HTTPS; plain HTTP on the emulator).

mod models;
pub(crate) mod xml;

pub use models::{EntityCounts, QueueInfo, SubscriptionInfo, TopicInfo};

use crate::namespace::Namespace;
use crate::tokens::TokenSource;
use crate::{Error, Result};
use models::{entries, queue_from_entry, subscription_from_entry, topic_from_entry};
use std::sync::Arc;
use std::time::Duration;
use xml::Node;

const API_VERSION: &str = "2024-05";
const PAGE_SIZE: usize = 100;

/// Read-side management client (M0). Create/update/delete arrive in M3.
#[derive(Clone)]
pub struct ManagementClient {
    http: reqwest::Client,
    base: String,
    tokens: Arc<dyn TokenSource>,
}

impl ManagementClient {
    /// Client for `ns`.
    pub fn new(ns: &Namespace) -> Self {
        Self::with_base(ns.management_base(), ns.tokens().clone())
    }

    /// Client for an explicit base URL (tests).
    pub fn with_base(base: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self { http: reqwest::Client::new(), base: base.into().trim_end_matches('/').to_string(), tokens }
    }

    /// All queues with counts.
    pub async fn list_queues(&self) -> Result<Vec<QueueInfo>> {
        self.list("$Resources/queues", queue_from_entry).await
    }

    /// All topics.
    pub async fn list_topics(&self) -> Result<Vec<TopicInfo>> {
        self.list("$Resources/topics", topic_from_entry).await
    }

    /// Subscriptions of `topic` with counts.
    pub async fn list_subscriptions(&self, topic: &str) -> Result<Vec<SubscriptionInfo>> {
        self.list(&format!("{topic}/Subscriptions"), |e| subscription_from_entry(topic, e)).await
    }

    /// One queue. A missing queue is `NotFound` (the service answers 200 + empty feed).
    pub async fn get_queue(&self, name: &str) -> Result<QueueInfo> {
        let root = self.get(name, &[("enrich", "false")]).await?;
        entries(&root)
            .into_iter()
            .find_map(queue_from_entry)
            .ok_or_else(|| Error::NotFound(format!("queue `{name}`")))
    }

    async fn list<T>(&self, path: &str, map: impl Fn(&Node) -> Option<T>) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut skip = 0usize;
        loop {
            let skip_s = skip.to_string();
            let top_s = PAGE_SIZE.to_string();
            let root = self
                .get(path, &[("enrich", "false"), ("$skip", &skip_s), ("$top", &top_s)])
                .await?;
            let page = entries(&root);
            let n = page.len();
            out.extend(page.into_iter().filter_map(&map));
            if n < PAGE_SIZE {
                return Ok(out);
            }
            skip += n;
        }
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Node> {
        let url = format!("{}/{path}", self.base);
        let token = self.tokens.token(&url.to_ascii_lowercase()).await?;
        let resp = self
            .http
            .get(&url)
            .query(&[("api-version", API_VERSION)])
            .query(query)
            .header(reqwest::header::AUTHORIZATION, token.http_authorization())
            .send()
            .await
            .map_err(|e| Error::Network(format!("management API {url}: {e}")))?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = resp.text().await.map_err(|e| Error::Network(e.to_string()))?;
        if (200..300).contains(&status) {
            return xml::parse(&body);
        }
        Err(map_http_error(status, &body, retry_after, &url))
    }
}

fn map_http_error(status: u16, body: &str, retry_after: Option<Duration>, url: &str) -> Error {
    let detail = xml::parse(body)
        .ok()
        .and_then(|n| n.text_of("Detail").map(str::to_string))
        .unwrap_or_else(|| body.trim().chars().take(500).collect());
    match status {
        401 => Error::Auth(detail),
        403 => Error::Forbidden { needed: needed_claims(&detail), detail },
        404 => Error::NotFound(url.to_string()),
        429 | 503 => Error::Throttled { retry_after },
        _ => Error::Protocol(format!("management API returned {status}: {detail}")),
    }
}

/// Extract the missing rights from Service Bus error detail text, e.g.
/// "Manage,EntityRead claims required for this operation." → `Manage,EntityRead`.
pub fn needed_claims(detail: &str) -> Option<String> {
    if let Some(idx) = detail.find(" claims required") {
        let before = &detail[..idx];
        let start = before.rfind([' ', ':']).map(|i| i + 1).unwrap_or(0);
        let claims = before[start..].trim();
        return (!claims.is_empty()).then(|| claims.to_string());
    }
    detail
        .split("specified action: ")
        .nth(1)
        .and_then(|rest| rest.split('.').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{ServiceBusToken, TokenKind};
    use async_trait::async_trait;
    use mockito::Matcher;
    use secrecy::SecretString;

    struct FixedToken;
    #[async_trait]
    impl TokenSource for FixedToken {
        async fn token(&self, _audience: &str) -> Result<ServiceBusToken> {
            Ok(ServiceBusToken {
                token: SecretString::from("SharedAccessSignature sr=x&sig=y&se=9999999999&skn=k"),
                kind: TokenKind::Sas,
                expires_on: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            })
        }
        fn describe(&self) -> String {
            "fixed".into()
        }
    }

    fn client(server: &mockito::Server) -> ManagementClient {
        ManagementClient::with_base(server.url(), Arc::new(FixedToken))
    }

    #[tokio::test]
    async fn list_queues_parses_counts_and_sends_auth() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("api-version".into(), "2024-05".into()),
                Matcher::UrlEncoded("$top".into(), "100".into()),
            ]))
            .match_header("authorization", Matcher::Regex("^SharedAccessSignature sr=".into()))
            .with_status(200)
            .with_body(include_str!("../../tests/fixtures/queue_feed.xml"))
            .create_async()
            .await;
        let queues = client(&server).list_queues().await.unwrap();
        assert_eq!(queues.len(), 2);
        let orders = &queues[0];
        assert_eq!(orders.name, "orders");
        assert_eq!(orders.counts.as_ref().unwrap().active, 3);
        assert_eq!(orders.counts.as_ref().unwrap().dead_letter, 1);
        assert_eq!(orders.max_delivery_count, Some(10));
        assert_eq!(orders.forward_to, None);
        assert!(queues[1].requires_session);
        assert_eq!(queues[1].counts, None);
    }

    #[tokio::test]
    async fn list_pages_until_short_page() {
        let mut server = mockito::Server::new_async().await;
        let entry = |i: usize| format!(r#"<entry><title>q{i}</title><content><QueueDescription><Status>Active</Status></QueueDescription></content></entry>"#);
        let page1 = format!("<feed>{}</feed>", (0..100).map(entry).collect::<String>());
        let page2 = format!("<feed>{}</feed>", entry(100));
        let _p1 = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::UrlEncoded("$skip".into(), "0".into()))
            .with_status(200)
            .with_body(page1)
            .create_async()
            .await;
        let _p2 = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::UrlEncoded("$skip".into(), "100".into()))
            .with_status(200)
            .with_body(page2)
            .create_async()
            .await;
        assert_eq!(client(&server).list_queues().await.unwrap().len(), 101);
    }

    #[tokio::test]
    async fn get_missing_queue_is_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/nope")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"<feed xmlns="http://www.w3.org/2005/Atom"><title type="text">Publicly Listed Services</title></feed>"#)
            .create_async()
            .await;
        assert!(matches!(client(&server).get_queue("nope").await.unwrap_err(), Error::NotFound(_)));
    }

    #[tokio::test]
    async fn forbidden_names_missing_claims() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/queues")
            .match_query(Matcher::Any)
            .with_status(403)
            .with_body("<Error><Code>403</Code><Detail>Unauthorized access. Manage,EntityRead claims required for this operation. TrackingId:abc</Detail></Error>")
            .create_async()
            .await;
        match client(&server).list_queues().await.unwrap_err() {
            Error::Forbidden { needed, .. } => assert_eq!(needed.as_deref(), Some("Manage,EntityRead")),
            other => panic!("expected Forbidden, got {other}"),
        }
    }

    #[tokio::test]
    async fn unauthorized_is_auth() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/$Resources/topics")
            .match_query(Matcher::Any)
            .with_status(401)
            .with_body("<Error><Code>401</Code><Detail>MissingToken: The authorization header was not found.</Detail></Error>")
            .create_async()
            .await;
        assert!(matches!(client(&server).list_topics().await.unwrap_err(), Error::Auth(m) if m.contains("MissingToken")));
    }

    #[test]
    fn needed_claims_variants() {
        assert_eq!(needed_claims("Authorization failed for specified action: Manage,EntityWrite. TrackingId:x").as_deref(), Some("Manage,EntityWrite"));
        assert_eq!(needed_claims("Listen claims required").as_deref(), Some("Listen"));
        assert_eq!(needed_claims("something else"), None);
    }
}
```

Modify `src/lib.rs`. Add `pub mod mgmt;` to the module list, and `pub use mgmt::{EntityCounts, ManagementClient, QueueInfo, SubscriptionInfo, TopicInfo};` to the re-exports.

- [ ] **Step 5: Run tests**

Run: `cargo test -p dispatched-core mgmt`
Expected: PASS, 8 tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: ATOM management client (queues, topics, subscriptions, counts, error mapping)"
```

---
### Task 13: AMQP connection, CBS, correlated `$management` link

**Files:**
- Create: `crates/dispatched-core/src/messaging/mod.rs`, `src/messaging/connection.rs`, `src/messaging/management.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `Namespace` (`amqp_url`, `amqp_audience`, `tokens`, `host`, `is_emulator`), `TokenKind::cbs_type`, `mgmt::needed_claims`, `Error`.
- Produces:
  - `messaging::ServiceBusConnection`:
    - `async open(&Namespace) -> Result<Self>`
    - `async authorize(&mut self, address: &str) -> Result<()>`
    - `async management(&mut self, address: &str) -> Result<&mut ManagementLink>` (cached per address)
    - `session_mut(&mut self) -> &mut SessionHandle<()>`
    - `async close(self) -> Result<()>`
  - `messaging::ManagementLink`:
    - `async attach(session: &mut SessionHandle<()>, address: &str) -> Result<Self>`
    - `async call(&mut self, operation: &str, body: OrderedMap<String, Value>) -> Result<MgmtResponse>`
    - `async close(self)`
  - `messaging::MgmtResponse { status: u16, description: Option<String>, body: Value }`, with `fn error(&self, operation: &str) -> Error`.
- Protocol notes:
  - Requests carry application properties `operation` and `com.microsoft:server-timeout` (u32 ms), plus properties `message_id` (ulong, incrementing) and `reply_to`.
  - The response is matched on `correlation_id == message_id`; unmatched responses are logged and skipped.
  - Status is read from `statusCode` or `status-code`, as an int of any width.

This task has no unit tests of its own: these types need a live broker. They are exercised end to end by Task 16's emulator gate test. Task 15 unit-tests the response parsing (`MgmtResponse` → messages) with constructed values.

- [ ] **Step 1: Write management.rs**

`src/messaging/management.rs`:
```rust
//! Request/response over an entity's `$management` node, correlated by
//! message id. Protocol: "AMQP 1.0 in Azure Service Bus: request-response
//! operations" (Microsoft Learn); request shapes cross-checked with the MIT
//! `azservicebus` crate (see NOTICE).

use crate::mgmt::needed_claims;
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::messaging::{ApplicationProperties, Body, Message, MessageId, Properties};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Value};
use fe2o3_amqp::{Delivery, Receiver, Sender};
use std::time::Duration;

const SERVER_TIMEOUT_MS: u32 = 60_000;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(65);

/// A decoded management response.
#[derive(Debug, Clone)]
pub struct MgmtResponse {
    /// HTTP-like status (200, 204, 401, 404, ...).
    pub status: u16,
    /// `statusDescription`, if present.
    pub description: Option<String>,
    /// AMQP-value body (`Value::Null` when absent).
    pub body: Value,
}

impl MgmtResponse {
    pub(crate) fn from_message(message: Message<Body<Value>>) -> Self {
        let props = message.application_properties.as_ref().map(|p| &p.0);
        let lookup = |a: &str, b: &str| props.and_then(|p| p.get(a).or_else(|| p.get(b)));
        let status = lookup("statusCode", "status-code").and_then(simple_to_u16).unwrap_or(0);
        let description = match lookup("statusDescription", "status-description") {
            Some(SimpleValue::String(s)) => Some(s.clone()),
            _ => None,
        };
        let body = match message.body {
            Body::Value(v) => v.0,
            _ => Value::Null,
        };
        Self { status, description, body }
    }

    /// Map a non-success status to an [`Error`].
    pub fn error(&self, operation: &str) -> Error {
        let detail = self.description.clone().unwrap_or_else(|| format!("status {}", self.status));
        match self.status {
            401 => Error::Auth(format!("{operation}: {detail}")),
            403 => Error::Forbidden { needed: needed_claims(&detail), detail },
            404 => Error::NotFound(format!("{operation}: {detail}")),
            429 | 503 => Error::Throttled { retry_after: None },
            _ => Error::Protocol(format!("{operation} returned {}: {detail}", self.status)),
        }
    }
}

fn simple_to_u16(v: &SimpleValue) -> Option<u16> {
    match v {
        SimpleValue::Int(n) => u16::try_from(*n).ok(),
        SimpleValue::Uint(n) => u16::try_from(*n).ok(),
        SimpleValue::Long(n) => u16::try_from(*n).ok(),
        SimpleValue::Ulong(n) => u16::try_from(*n).ok(),
        SimpleValue::Short(n) => u16::try_from(*n).ok(),
        SimpleValue::Ushort(n) => Some(*n),
        _ => None,
    }
}

/// A sender/receiver pair attached to `{address}/$management`.
pub struct ManagementLink {
    sender: Sender,
    receiver: Receiver,
    reply_to: String,
    next_id: u64,
}

impl ManagementLink {
    /// Attach to `{address}/$management` on `session`.
    pub async fn attach(session: &mut SessionHandle<()>, address: &str) -> Result<Self> {
        let node = format!("{address}/$management");
        let reply_to = format!("{address}-mgmt-{}", uuid::Uuid::new_v4().simple());
        let sender = Sender::builder()
            .name(format!("{reply_to}-tx"))
            .target(node.clone())
            .attach(session)
            .await
            .map_err(|e| Error::Protocol(format!("attach sender to {node}: {e}")))?;
        let receiver = Receiver::builder()
            .name(format!("{reply_to}-rx"))
            .source(node.clone())
            .target(reply_to.clone())
            .credit_mode(CreditMode::Auto(10))
            .attach(session)
            .await
            .map_err(|e| Error::Protocol(format!("attach receiver to {node}: {e}")))?;
        Ok(Self { sender, receiver, reply_to, next_id: 1 })
    }

    /// Send `operation` with a map body and wait for its correlated response.
    pub async fn call(&mut self, operation: &str, body: OrderedMap<String, Value>) -> Result<MgmtResponse> {
        let id = self.next_id;
        self.next_id += 1;
        let mut app = OrderedMap::new();
        app.insert("operation".to_string(), SimpleValue::String(operation.to_string()));
        app.insert("com.microsoft:server-timeout".to_string(), SimpleValue::Uint(SERVER_TIMEOUT_MS));
        let request = Message::builder()
            .properties(Properties::builder().message_id(MessageId::Ulong(id)).reply_to(self.reply_to.clone()).build())
            .application_properties(ApplicationProperties(app))
            .value(body)
            .build();
        let outcome = self
            .sender
            .send(request)
            .await
            .map_err(|e| Error::Protocol(format!("{operation}: send failed: {e}")))?;
        outcome
            .accepted_or_else(|o| o)
            .map_err(|o| Error::Protocol(format!("{operation}: request not accepted: {o:?}")))?;

        let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
        loop {
            let delivery: Delivery<Body<Value>> = tokio::time::timeout_at(deadline, self.receiver.recv())
                .await
                .map_err(|_| Error::Timeout(format!("{operation} got no response within {}s", CLIENT_TIMEOUT.as_secs())))?
                .map_err(|e| Error::Protocol(format!("{operation}: receive failed: {e}")))?;
            self.receiver
                .accept(&delivery)
                .await
                .map_err(|e| Error::Protocol(format!("{operation}: settle response: {e}")))?;
            let message = delivery.into_message();
            let correlated = message
                .properties
                .as_ref()
                .and_then(|p| p.correlation_id.as_ref())
                .is_some_and(|c| *c == MessageId::Ulong(id));
            if correlated {
                return Ok(MgmtResponse::from_message(message));
            }
            tracing::warn!(operation, "skipping uncorrelated management response");
        }
    }

    /// Detach both links (best effort).
    pub async fn close(self) {
        let _ = self.sender.close().await;
        let _ = self.receiver.close().await;
    }
}
```

- [ ] **Step 2: Write connection.rs**

`src/messaging/connection.rs`:
```rust
//! One AMQP connection per namespace: connection + session + CBS client,
//! plus cached management links per entity address.

use super::management::ManagementLink;
use crate::namespace::Namespace;
use crate::{Error, Result};
use fe2o3_amqp::connection::ConnectionHandle;
use fe2o3_amqp::sasl_profile::SaslProfile;
use fe2o3_amqp::session::SessionHandle;
use fe2o3_amqp::types::primitives::Timestamp;
use fe2o3_amqp::{Connection, Session};
use fe2o3_amqp_cbs::client::CbsClient;
use fe2o3_amqp_cbs::token::CbsToken;
use secrecy::ExposeSecret;
use std::collections::HashMap;

/// An open AMQP connection to a Service Bus namespace (or the emulator).
pub struct ServiceBusConnection {
    ns: Namespace,
    connection: ConnectionHandle<()>,
    session: SessionHandle<()>,
    cbs: CbsClient,
    management: HashMap<String, ManagementLink>,
}

impl ServiceBusConnection {
    /// Connect (TLS for the cloud, plain TCP for the emulator), begin a
    /// session, and attach the CBS node.
    pub async fn open(ns: &Namespace) -> Result<Self> {
        let url = ns.amqp_url();
        let builder = Connection::builder()
            .container_id(format!("dispatched-{}", uuid::Uuid::new_v4().simple()))
            .hostname(Some(ns.host()))
            .sasl_profile(SaslProfile::Anonymous)
            .max_frame_size(65_536u32)
            .idle_time_out(60_000u32);
        let builder = if ns.is_emulator() { builder } else { builder.alt_tls_establishment(true) };
        let mut connection = builder
            .open(url.as_str())
            .await
            .map_err(|e| Error::Network(format!("cannot connect to {url}: {e}")))?;
        let mut session = Session::begin(&mut connection)
            .await
            .map_err(|e| Error::Protocol(format!("begin AMQP session: {e}")))?;
        let cbs = CbsClient::attach(&mut session)
            .await
            .map_err(|e| Error::Protocol(format!("attach $cbs: {e}")))?;
        Ok(Self { ns: ns.clone(), connection, session, cbs, management: HashMap::new() })
    }

    /// Put a token for `address` (e.g. `orders/$DeadLetterQueue`) on the CBS node.
    pub async fn authorize(&mut self, address: &str) -> Result<()> {
        let audience = self.ns.amqp_audience(address);
        let token = self.ns.tokens().token(&audience).await?;
        let cbs_token = CbsToken::new(
            token.token.expose_secret().to_string(),
            token.kind.cbs_type(),
            Some(Timestamp::from(token.expires_on)),
        );
        self.cbs
            .put_token(audience.clone(), cbs_token)
            .await
            .map_err(|e| Error::Auth(format!("token for {audience} was rejected: {e}")))
    }

    /// The management link for `address`, attached and authorized on first use.
    pub async fn management(&mut self, address: &str) -> Result<&mut ManagementLink> {
        if !self.management.contains_key(address) {
            self.authorize(address).await?;
            self.authorize(&format!("{address}/$management")).await?;
            let link = ManagementLink::attach(&mut self.session, address).await?;
            self.management.insert(address.to_string(), link);
        }
        Ok(self.management.get_mut(address).expect("inserted above"))
    }

    /// The AMQP session (for data links).
    pub fn session_mut(&mut self) -> &mut SessionHandle<()> {
        &mut self.session
    }

    /// Close links, session and connection.
    pub async fn close(mut self) -> Result<()> {
        for (_, link) in self.management.drain() {
            link.close().await;
        }
        let _ = self.cbs.close().await;
        let _ = self.session.end().await;
        self.connection
            .close()
            .await
            .map_err(|e| Error::Network(format!("close AMQP connection: {e}")))
    }
}
```

`src/messaging/mod.rs`:
```rust
//! AMQP data plane: connection, CBS, management link, message model.

mod connection;
mod management;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
```

Modify `src/lib.rs`. Add `pub mod messaging;`.

- [ ] **Step 3: Compile**

Run: `cargo build -p dispatched-core && cargo clippy -p dispatched-core --all-targets -- -D warnings`
Expected: builds clean. If an fe2o3 signature differs from the code above, fix it against the crate source with `cargo doc -p fe2o3-amqp --open`, or read `~/.cargo/registry/src/*/fe2o3-amqp-0.18.*/src/link/builder.rs`. Builder methods `name`, `source`, `target` and `credit_mode` take `impl Into<…>`. Keep the semantics identical, and record any deviation in the commit message.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: AMQP connection with CBS and correlated \$management link"
```

---

### Task 14: Message model and decoding

**Files:**
- Create: `crates/dispatched-core/src/messaging/message.rs`
- Modify: `src/messaging/mod.rs`, `src/lib.rs`

**Interfaces:**
- Produces:
  - `messaging::PeekedMessage` (Serialize, Clone, Debug), with fields:
    - `sequence_number: i64`, `delivery_count: u32`, `time_to_live_ms: Option<u32>`
    - `message_id`, `correlation_id`, `session_id`, `subject`, `content_type`, `to`, `reply_to`: each `Option<String>`
    - `enqueued_time`, `scheduled_enqueue_time`, `locked_until`: each `Option<OffsetDateTime>`, serialized RFC 3339
    - `dead_letter_reason`, `dead_letter_description`, `dead_letter_source`: each `Option<String>`
    - `application_properties: BTreeMap<String, PropValue>`
    - `body: MessageBody`
  - `messaging::MessageBody::{Binary(Vec<u8>), Text(String), Value(String), Sequence(String), Empty}`, with `as_text(&self) -> Option<String>` and `len(&self) -> usize`. It serializes as `{"kind": ..., "value": ...}`, with binary as base64.
  - `messaging::PropValue::{Null, Bool(bool), Int(i64), UInt(u64), Double(f64), String(String), Timestamp(OffsetDateTime), Uuid(String), Binary(Vec<u8>), Other(String)}`
  - `pub fn decode_message(Message<Body<Value>>) -> PeekedMessage`
  - `pub fn decode_bytes(&[u8]) -> Result<PeekedMessage>`

- [ ] **Step 1: Write message.rs with the Review Focus #3 tests**

`src/messaging/message.rs`:
```rust
//! Display model for Service Bus messages and decoding from AMQP.

use crate::{Error, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fe2o3_amqp::types::messaging::annotations::AnnotationKey;
use fe2o3_amqp::types::messaging::message::__private::Deserializable;
use fe2o3_amqp::types::messaging::{Body, Message, MessageId};
use fe2o3_amqp::types::primitives::{SimpleValue, Value};
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use time::OffsetDateTime;

/// A message as shown to users and agents (peeked; no lock held).
#[derive(Debug, Clone, Serialize)]
pub struct PeekedMessage {
    /// Broker-assigned sequence number.
    pub sequence_number: i64,
    /// `MessageId`.
    pub message_id: Option<String>,
    /// `CorrelationId`.
    pub correlation_id: Option<String>,
    /// `SessionId` (AMQP group-id).
    pub session_id: Option<String>,
    /// `Subject` (formerly Label).
    pub subject: Option<String>,
    /// `ContentType`.
    pub content_type: Option<String>,
    /// `To`.
    pub to: Option<String>,
    /// `ReplyTo`.
    pub reply_to: Option<String>,
    /// When the broker accepted the message.
    #[serde(with = "time::serde::rfc3339::option")]
    pub enqueued_time: Option<OffsetDateTime>,
    /// Scheduled enqueue time, for scheduled messages.
    #[serde(with = "time::serde::rfc3339::option")]
    pub scheduled_enqueue_time: Option<OffsetDateTime>,
    /// Lock expiry, for locked messages.
    #[serde(with = "time::serde::rfc3339::option")]
    pub locked_until: Option<OffsetDateTime>,
    /// Delivery attempts so far.
    pub delivery_count: u32,
    /// Time-to-live in ms, if set on the message.
    pub time_to_live_ms: Option<u32>,
    /// `DeadLetterReason` application property.
    pub dead_letter_reason: Option<String>,
    /// `DeadLetterErrorDescription` application property.
    pub dead_letter_description: Option<String>,
    /// Entity the message was dead-lettered from (`x-opt-deadletter-source`).
    pub dead_letter_source: Option<String>,
    /// Custom (application) properties.
    pub application_properties: BTreeMap<String, PropValue>,
    /// Body.
    pub body: MessageBody,
}

/// Message body in display form.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum MessageBody {
    /// One or more data sections, concatenated.
    Binary(#[serde(serialize_with = "as_base64")] Vec<u8>),
    /// AMQP-value string body.
    Text(String),
    /// Other AMQP-value body, rendered for display.
    Value(String),
    /// AMQP-sequence body, rendered for display.
    Sequence(String),
    /// No body.
    Empty,
}

fn as_base64<S: Serializer>(bytes: &[u8], s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(&STANDARD.encode(bytes))
}

impl MessageBody {
    /// Body as text, if it is (valid UTF-8) text.
    pub fn as_text(&self) -> Option<String> {
        match self {
            MessageBody::Binary(b) => String::from_utf8(b.clone()).ok(),
            MessageBody::Text(s) | MessageBody::Value(s) | MessageBody::Sequence(s) => Some(s.clone()),
            MessageBody::Empty => None,
        }
    }

    /// Size in bytes (UTF-8 length for text forms).
    pub fn len(&self) -> usize {
        match self {
            MessageBody::Binary(b) => b.len(),
            MessageBody::Text(s) | MessageBody::Value(s) | MessageBody::Sequence(s) => s.len(),
            MessageBody::Empty => 0,
        }
    }

    /// Whether the body is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A typed application-property value.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum PropValue {
    /// null
    Null,
    /// bool
    Bool(bool),
    /// any signed integer
    Int(i64),
    /// any unsigned integer
    UInt(u64),
    /// float / double
    Double(f64),
    /// string or symbol
    String(String),
    /// timestamp
    #[serde(with = "time::serde::rfc3339")]
    Timestamp(OffsetDateTime),
    /// uuid
    Uuid(String),
    /// binary
    Binary(#[serde(serialize_with = "as_base64")] Vec<u8>),
    /// anything else, rendered
    Other(String),
}

impl PropValue {
    /// String content, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropValue::String(s) => Some(s),
            _ => None,
        }
    }
}

fn simple_to_prop(v: &SimpleValue) -> PropValue {
    match v {
        SimpleValue::Null => PropValue::Null,
        SimpleValue::Bool(b) => PropValue::Bool(*b),
        SimpleValue::Byte(n) => PropValue::Int(i64::from(*n)),
        SimpleValue::Short(n) => PropValue::Int(i64::from(*n)),
        SimpleValue::Int(n) => PropValue::Int(i64::from(*n)),
        SimpleValue::Long(n) => PropValue::Int(*n),
        SimpleValue::Ubyte(n) => PropValue::UInt(u64::from(*n)),
        SimpleValue::Ushort(n) => PropValue::UInt(u64::from(*n)),
        SimpleValue::Uint(n) => PropValue::UInt(u64::from(*n)),
        SimpleValue::Ulong(n) => PropValue::UInt(*n),
        SimpleValue::Float(f) => PropValue::Double(f64::from(f.into_inner())),
        SimpleValue::Double(f) => PropValue::Double(f.into_inner()),
        SimpleValue::String(s) => PropValue::String(s.clone()),
        SimpleValue::Symbol(s) => PropValue::String(s.0.clone()),
        SimpleValue::Timestamp(t) => PropValue::Timestamp(OffsetDateTime::from(t.clone())),
        SimpleValue::Uuid(u) => PropValue::Uuid(uuid::Uuid::from(u.clone()).to_string()),
        SimpleValue::Binary(b) => PropValue::Binary(b.to_vec()),
        other => PropValue::Other(format!("{other:?}")),
    }
}

fn message_id_string(id: &MessageId) -> String {
    match id {
        MessageId::String(s) => s.clone(),
        MessageId::Ulong(n) => n.to_string(),
        MessageId::Uuid(u) => uuid::Uuid::from(u.clone()).to_string(),
        MessageId::Binary(b) => b.iter().map(|byte| format!("{byte:02x}")).collect(),
    }
}

fn annotation<'a>(m: &'a Message<Body<Value>>, key: &str) -> Option<&'a Value> {
    m.message_annotations.as_ref().and_then(|ma| ma.get(&key as &dyn AnnotationKey))
}

fn annotation_time(m: &Message<Body<Value>>, key: &str) -> Option<OffsetDateTime> {
    match annotation(m, key)? {
        Value::Timestamp(t) => Some(OffsetDateTime::from(t.clone())),
        _ => None,
    }
}

fn annotation_string(m: &Message<Body<Value>>, key: &str) -> Option<String> {
    match annotation(m, key)? {
        Value::String(s) => Some(s.clone()),
        Value::Symbol(s) => Some(s.0.clone()),
        _ => None,
    }
}

/// Convert an AMQP message into the display model.
pub fn decode_message(m: Message<Body<Value>>) -> PeekedMessage {
    let sequence_number = match annotation(&m, "x-opt-sequence-number") {
        Some(Value::Long(n)) => *n,
        _ => -1,
    };
    let props = m.properties.as_ref();
    let header = m.header.as_ref();
    let mut application_properties = BTreeMap::new();
    if let Some(ap) = m.application_properties.as_ref() {
        for (k, v) in ap.0.iter() {
            application_properties.insert(k.clone(), simple_to_prop(v));
        }
    }
    let dead_letter_reason = application_properties.get("DeadLetterReason").and_then(|v| v.as_str()).map(str::to_string);
    let dead_letter_description = application_properties
        .get("DeadLetterErrorDescription")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let body = match &m.body {
        Body::Data(batch) => MessageBody::Binary(batch.iter().flat_map(|d| d.0.iter().copied()).collect()),
        Body::Value(v) => match &v.0 {
            Value::String(s) => MessageBody::Text(s.clone()),
            Value::Null => MessageBody::Empty,
            other => MessageBody::Value(format!("{other:?}")),
        },
        Body::Sequence(seq) => MessageBody::Sequence(format!("{seq:?}")),
        Body::Empty => MessageBody::Empty,
    };
    PeekedMessage {
        sequence_number,
        message_id: props.and_then(|p| p.message_id.as_ref()).map(message_id_string),
        correlation_id: props.and_then(|p| p.correlation_id.as_ref()).map(message_id_string),
        session_id: props.and_then(|p| p.group_id.clone()),
        subject: props.and_then(|p| p.subject.clone()),
        content_type: props.and_then(|p| p.content_type.as_ref()).map(|s| s.0.clone()),
        to: props.and_then(|p| p.to.clone()),
        reply_to: props.and_then(|p| p.reply_to.clone()),
        enqueued_time: annotation_time(&m, "x-opt-enqueued-time"),
        scheduled_enqueue_time: annotation_time(&m, "x-opt-scheduled-enqueue-time"),
        locked_until: annotation_time(&m, "x-opt-locked-until"),
        delivery_count: header.map(|h| h.delivery_count).unwrap_or(0),
        time_to_live_ms: header.and_then(|h| h.ttl),
        dead_letter_reason,
        dead_letter_description,
        dead_letter_source: annotation_string(&m, "x-opt-deadletter-source"),
        application_properties,
        body,
    }
}

/// Decode a binary-encoded AMQP message (as returned by peek / receive-by-sequence-number).
pub fn decode_bytes(bytes: &[u8]) -> Result<PeekedMessage> {
    let decoded: Deserializable<Message<Body<Value>>> =
        serde_amqp::from_slice(bytes).map_err(|e| Error::Protocol(format!("cannot decode AMQP message: {e}")))?;
    Ok(decode_message(decoded.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fe2o3_amqp::types::messaging::message::__private::Serializable;
    use fe2o3_amqp::types::messaging::{AmqpValue, ApplicationProperties, Batch, Data, Header, MessageAnnotations, Properties};
    use fe2o3_amqp::types::primitives::OrderedMap;

    fn base(body: Body<Value>) -> Message<Body<Value>> {
        let mut app = OrderedMap::new();
        app.insert("DeadLetterReason".to_string(), SimpleValue::String("MaxDeliveryCountExceeded".into()));
        app.insert("tenant".to_string(), SimpleValue::Int(7));
        Message {
            header: Some(Header { delivery_count: 3, ..Default::default() }),
            delivery_annotations: None,
            message_annotations: Some(MessageAnnotations::builder().insert("x-opt-sequence-number", 42i64).build()),
            properties: Some(
                Properties::builder()
                    .message_id(String::from("m-1"))
                    .subject("order.created")
                    .content_type("application/json")
                    .group_id(Some("s-1".to_string()))
                    .build(),
            ),
            application_properties: Some(ApplicationProperties(app)),
            body,
            footer: None,
        }
    }

    #[test]
    fn decodes_system_and_custom_properties() {
        let m = decode_message(base(Body::Data(Batch::from(vec![Data::from(b"{\"a\":1}".to_vec())]))));
        assert_eq!(m.sequence_number, 42);
        assert_eq!(m.message_id.as_deref(), Some("m-1"));
        assert_eq!(m.session_id.as_deref(), Some("s-1"));
        assert_eq!(m.subject.as_deref(), Some("order.created"));
        assert_eq!(m.content_type.as_deref(), Some("application/json"));
        assert_eq!(m.delivery_count, 3);
        assert_eq!(m.dead_letter_reason.as_deref(), Some("MaxDeliveryCountExceeded"));
        assert_eq!(m.application_properties["tenant"], PropValue::Int(7));
        assert_eq!(m.body.as_text().as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn decodes_amqp_value_string_body() {
        let m = decode_message(base(Body::Value(AmqpValue(Value::String("hello".into())))));
        assert_eq!(m.body, MessageBody::Text("hello".into()));
    }

    #[test]
    fn decodes_multiple_data_sections() {
        let body = Body::Data(Batch::from(vec![Data::from(b"ab".to_vec()), Data::from(b"cd".to_vec())]));
        assert_eq!(decode_message(base(body)).body, MessageBody::Binary(b"abcd".to_vec()));
    }

    #[test]
    fn non_utf8_binary_has_no_text() {
        let m = decode_message(base(Body::Data(Batch::from(vec![Data::from(vec![0xff, 0xfe, 0x00])]))));
        assert_eq!(m.body.as_text(), None);
        assert_eq!(m.body.len(), 3);
    }

    #[test]
    fn empty_body_and_missing_annotations_do_not_panic() {
        let mut msg = base(Body::Empty);
        msg.message_annotations = None;
        msg.header = None;
        let m = decode_message(msg);
        assert_eq!(m.body, MessageBody::Empty);
        assert_eq!(m.sequence_number, -1);
        assert_eq!(m.delivery_count, 0);
    }

    #[test]
    fn roundtrips_through_wire_bytes() {
        let msg = Message::builder()
            .properties(Properties::builder().message_id(String::from("wire")).build())
            .message_annotations(MessageAnnotations::builder().insert("x-opt-sequence-number", 7i64).build())
            .data(b"payload".to_vec())
            .build();
        let bytes = serde_amqp::to_vec(&Serializable(msg)).unwrap();
        let m = decode_bytes(&bytes).unwrap();
        assert_eq!(m.sequence_number, 7);
        assert_eq!(m.message_id.as_deref(), Some("wire"));
        assert_eq!(m.body.as_text().as_deref(), Some("payload"));
    }

    #[test]
    fn json_shape_is_stable() {
        let m = decode_message(base(Body::Data(Batch::from(vec![Data::from(vec![1, 2, 3])]))));
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["body"]["kind"], "binary");
        assert_eq!(json["body"]["value"], "AQID");
        assert_eq!(json["application_properties"]["tenant"]["type"], "int");
    }
}
```

Modify `src/messaging/mod.rs`. Add `mod message;` and `pub use message::{MessageBody, PeekedMessage, PropValue, decode_bytes, decode_message};`.

Modify `src/lib.rs`. Add `pub use messaging::{MessageBody, PeekedMessage, PropValue};`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p dispatched-core messaging::message`
Expected: PASS, 7 tests. Possible compile errors:
- **Unknown variant name:** if `serde_amqp` names a variant differently (e.g. `SimpleValue::Ubyte`), check `~/.cargo/registry/src/*/serde_amqp-0.18.*/src/primitives/` and `.../src/value/`, then adjust the match arms. Keep the catch-all `other =>` arm.
- **`clone_on_copy` lint:** if clippy flags `t.clone()` on `Timestamp` or `Uuid`, replace it with `*t`.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: message display model and AMQP decoding"
```

---

### Task 15: Peek, the Explorer facade, and test-support seeding helpers

**Files:**
- Create: `crates/dispatched-core/src/messaging/peek.rs`, `src/messaging/testing.rs`, `src/explorer.rs`
- Modify: `src/messaging/mod.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `ServiceBusConnection`, `ManagementLink::call`, `MgmtResponse` (Task 13); `decode_bytes`, `PeekedMessage` (Task 14); `ManagementClient` and models (Task 12); `Namespace`, `EntityPath`, `SubQueue` (Task 11).
- Produces:
  - `messaging::peek::{peek_request(from_sequence: i64, count: i32) -> OrderedMap<String, Value>, parse_peek_response(MgmtResponse) -> Result<Vec<PeekedMessage>>, async peek(conn: &mut ServiceBusConnection, address: &str, from_sequence: i64, max: u32) -> Result<Vec<PeekedMessage>>}`
  - `Explorer`:
    - `new(Namespace) -> Self`
    - `namespace(&self) -> &Namespace`
    - `async list_queues(&self)`, `async list_topics(&self)`, `async list_subscriptions(&self, topic: &str)`
    - `async peek(&self, entity: &EntityPath, sub_queue: SubQueue, from_sequence: i64, max: u32) -> Result<Vec<PeekedMessage>>`
    - `async close(&self)`
  - With feature `test-support`, `messaging::testing`:
    - `async send_text(conn, address, body: &str, props: &[(&str, &str)]) -> Result<()>`
    - `async dead_letter_next(conn, address, reason: &str, description: &str, wait: Duration) -> Result<bool>`
    - `async drain(conn, address, idle: Duration) -> Result<usize>`

- [ ] **Step 1: Write peek.rs with the Review Focus #2 tests**

`src/messaging/peek.rs`:
```rust
//! `com.microsoft:peek-message`: non-destructive browse by sequence number.

use super::connection::ServiceBusConnection;
use super::management::MgmtResponse;
use super::message::{PeekedMessage, decode_bytes};
use crate::{Error, Result};
use fe2o3_amqp::types::primitives::{OrderedMap, Value};

const OPERATION: &str = "com.microsoft:peek-message";
/// Upper bound per request; larger peeks are paged by the caller.
pub const MAX_PER_REQUEST: u32 = 250;

/// Request body for a peek.
pub fn peek_request(from_sequence: i64, count: i32) -> OrderedMap<String, Value> {
    let mut body = OrderedMap::new();
    body.insert("from-sequence-number".to_string(), Value::Long(from_sequence));
    body.insert("message-count".to_string(), Value::Int(count));
    body
}

/// Decode a peek response. `204` (or no `messages`) means nothing to peek.
pub fn parse_peek_response(resp: MgmtResponse) -> Result<Vec<PeekedMessage>> {
    match resp.status {
        204 => return Ok(Vec::new()),
        200 => {}
        _ => return Err(resp.error(OPERATION)),
    }
    let Value::Map(map) = &resp.body else {
        return Ok(Vec::new());
    };
    let items: Vec<&Value> = match map.get(&Value::String("messages".into())) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::List(list)) => list.iter().collect(),
        Some(Value::Array(array)) => array.0.iter().collect(),
        Some(other) => return Err(Error::Protocol(format!("peek: unexpected `messages` value {other:?}"))),
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::Map(entry) => match entry.get(&Value::String("message".into())) {
                Some(Value::Binary(bytes)) => decode_bytes(bytes),
                _ => Err(Error::Protocol("peek: entry without `message` binary".into())),
            },
            other => Err(Error::Protocol(format!("peek: unexpected entry {other:?}"))),
        })
        .collect()
}

/// Peek up to `max` messages from `address`, starting at `from_sequence`.
pub async fn peek(conn: &mut ServiceBusConnection, address: &str, from_sequence: i64, max: u32) -> Result<Vec<PeekedMessage>> {
    let count = i32::try_from(max.clamp(1, MAX_PER_REQUEST)).expect("bounded by MAX_PER_REQUEST");
    let link = conn.management(address).await?;
    let resp = link.call(OPERATION, peek_request(from_sequence, count)).await?;
    parse_peek_response(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fe2o3_amqp::types::messaging::message::__private::Serializable;
    use fe2o3_amqp::types::messaging::{Message, MessageAnnotations};
    use serde_bytes::ByteBuf;

    fn wire_message(seq: i64) -> Value {
        let msg = Message::builder()
            .message_annotations(MessageAnnotations::builder().insert("x-opt-sequence-number", seq).build())
            .data(format!("m{seq}").into_bytes())
            .build();
        let bytes = serde_amqp::to_vec(&Serializable(msg)).unwrap();
        let mut entry = OrderedMap::new();
        entry.insert(Value::String("message".into()), Value::Binary(ByteBuf::from(bytes)));
        Value::Map(entry)
    }

    fn response(status: u16, body: Value) -> MgmtResponse {
        MgmtResponse { status, description: None, body }
    }

    #[test]
    fn request_body_shape() {
        let body = peek_request(5, 10);
        assert_eq!(body.get("from-sequence-number"), Some(&Value::Long(5)));
        assert_eq!(body.get("message-count"), Some(&Value::Int(10)));
    }

    #[test]
    fn peek_response_with_list_decodes_messages() {
        let mut map = OrderedMap::new();
        map.insert(Value::String("messages".into()), Value::List(vec![wire_message(1), wire_message(2)]));
        let msgs = parse_peek_response(response(200, Value::Map(map))).unwrap();
        assert_eq!(msgs.iter().map(|m| m.sequence_number).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn peek_response_204_is_empty() {
        assert!(parse_peek_response(response(204, Value::Null)).unwrap().is_empty());
    }

    #[test]
    fn peek_response_200_without_messages_is_empty() {
        assert!(parse_peek_response(response(200, Value::Map(OrderedMap::new()))).unwrap().is_empty());
    }

    #[test]
    fn peek_unauthorized_maps_to_auth() {
        let r = MgmtResponse { status: 401, description: Some("Unauthorized".into()), body: Value::Null };
        assert!(matches!(parse_peek_response(r).unwrap_err(), Error::Auth(_)));
    }
}
```

Add `serde_bytes = "0.11"` to `[dev-dependencies]` in `crates/dispatched-core/Cargo.toml`. It's needed for `ByteBuf` in tests. If `Value::Binary` wraps a different type, construct it the way the compiler says.

- [ ] **Step 2: Write testing.rs (feature `test-support`)**

`src/messaging/testing.rs`:
```rust
//! Seeding helpers for integration tests. NOT a product API: real send,
//! receive and settle (with policy confirmation) arrive in M2.

use super::connection::ServiceBusConnection;
use crate::{Error, Result};
use fe2o3_amqp::link::receiver::CreditMode;
use fe2o3_amqp::types::definitions::{Error as AmqpError, ErrorCondition, Fields, ReceiverSettleMode, SenderSettleMode};
use fe2o3_amqp::types::messaging::{ApplicationProperties, Body, Message, Properties};
use fe2o3_amqp::types::primitives::{OrderedMap, SimpleValue, Symbol, Value};
use fe2o3_amqp::{Receiver, Sender};
use std::time::Duration;

fn link_name(kind: &str) -> String {
    format!("dispatched-test-{kind}-{}", uuid::Uuid::new_v4().simple())
}

/// Send one text message with string application properties.
pub async fn send_text(conn: &mut ServiceBusConnection, address: &str, body: &str, props: &[(&str, &str)]) -> Result<()> {
    conn.authorize(address).await?;
    let mut sender = Sender::builder()
        .name(link_name("tx"))
        .target(address.to_string())
        .attach(conn.session_mut())
        .await
        .map_err(|e| Error::Protocol(format!("attach sender: {e}")))?;
    let mut app = OrderedMap::new();
    for (k, v) in props {
        app.insert((*k).to_string(), SimpleValue::String((*v).to_string()));
    }
    let message = Message::builder()
        .properties(Properties::builder().message_id(uuid::Uuid::new_v4().to_string()).content_type("application/json").build())
        .application_properties(ApplicationProperties(app))
        .data(body.as_bytes().to_vec())
        .build();
    let outcome = sender.send(message).await.map_err(|e| Error::Protocol(format!("send: {e}")))?;
    outcome
        .accepted_or_else(|o| o)
        .map_err(|o| Error::Protocol(format!("send not accepted: {o:?}")))?;
    let _ = sender.close().await;
    Ok(())
}

/// Receive the next message (peek-lock) and dead-letter it with `reason`.
/// Returns `false` if nothing arrived within `wait`.
pub async fn dead_letter_next(
    conn: &mut ServiceBusConnection,
    address: &str,
    reason: &str,
    description: &str,
    wait: Duration,
) -> Result<bool> {
    conn.authorize(address).await?;
    let mut receiver = Receiver::builder()
        .name(link_name("rx"))
        .source(address.to_string())
        .receiver_settle_mode(ReceiverSettleMode::Second)
        .credit_mode(CreditMode::Auto(1))
        .attach(conn.session_mut())
        .await
        .map_err(|e| Error::Protocol(format!("attach receiver: {e}")))?;
    let delivery = match tokio::time::timeout(wait, receiver.recv::<Body<Value>>()).await {
        Err(_) => {
            let _ = receiver.close().await;
            return Ok(false);
        }
        Ok(r) => r.map_err(|e| Error::Protocol(format!("receive: {e}")))?,
    };
    let mut info = Fields::default();
    info.insert(Symbol::from("DeadLetterReason"), Value::from(reason));
    info.insert(Symbol::from("DeadLetterErrorDescription"), Value::from(description));
    let error = AmqpError::new(ErrorCondition::Custom(Symbol::from("com.microsoft:dead-letter")), None, Some(info));
    receiver
        .reject(&delivery, error)
        .await
        .map_err(|e| Error::Protocol(format!("dead-letter: {e}")))?;
    let _ = receiver.close().await;
    Ok(true)
}

/// Receive-and-delete everything in `address` until it has been idle for `idle`.
pub async fn drain(conn: &mut ServiceBusConnection, address: &str, idle: Duration) -> Result<usize> {
    conn.authorize(address).await?;
    let mut receiver = Receiver::builder()
        .name(link_name("drain"))
        .source(address.to_string())
        .sender_settle_mode(SenderSettleMode::Settled)
        .credit_mode(CreditMode::Auto(50))
        .attach(conn.session_mut())
        .await
        .map_err(|e| Error::Protocol(format!("attach drain receiver: {e}")))?;
    let mut drained = 0;
    while let Ok(result) = tokio::time::timeout(idle, receiver.recv::<Body<Value>>()).await {
        let delivery = result.map_err(|e| Error::Protocol(format!("drain receive: {e}")))?;
        let _ = receiver.accept(&delivery).await;
        drained += 1;
    }
    let _ = receiver.close().await;
    Ok(drained)
}
```

Replace `src/messaging/mod.rs`:
```rust
//! AMQP data plane: connection, CBS, management link, message model, peek.

mod connection;
mod management;
mod message;
pub mod peek;
#[cfg(feature = "test-support")]
pub mod testing;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
pub use message::{MessageBody, PeekedMessage, PropValue, decode_bytes, decode_message};
```

- [ ] **Step 3: Write explorer.rs**

`src/explorer.rs`:
```rust
//! The facade every surface uses: one namespace, lazily-opened AMQP connection.

use crate::messaging::{PeekedMessage, ServiceBusConnection, peek};
use crate::mgmt::{ManagementClient, QueueInfo, SubscriptionInfo, TopicInfo};
use crate::namespace::{EntityPath, Namespace, SubQueue};
use crate::{Error, Result};
use tokio::sync::Mutex;

/// Browse and (from M2) operate one namespace.
pub struct Explorer {
    ns: Namespace,
    mgmt: ManagementClient,
    amqp: Mutex<Option<ServiceBusConnection>>,
}

impl Explorer {
    /// Explorer for `ns`. No network I/O until first use.
    pub fn new(ns: Namespace) -> Self {
        let mgmt = ManagementClient::new(&ns);
        Self { ns, mgmt, amqp: Mutex::new(None) }
    }

    /// The namespace.
    pub fn namespace(&self) -> &Namespace {
        &self.ns
    }

    /// Queues with counts.
    pub async fn list_queues(&self) -> Result<Vec<QueueInfo>> {
        self.mgmt.list_queues().await
    }

    /// Topics.
    pub async fn list_topics(&self) -> Result<Vec<TopicInfo>> {
        self.mgmt.list_topics().await
    }

    /// Subscriptions of `topic`.
    pub async fn list_subscriptions(&self, topic: &str) -> Result<Vec<SubscriptionInfo>> {
        self.mgmt.list_subscriptions(topic).await
    }

    /// Peek up to `max` messages from `entity`'s `sub_queue`, starting at `from_sequence`.
    pub async fn peek(&self, entity: &EntityPath, sub_queue: SubQueue, from_sequence: i64, max: u32) -> Result<Vec<PeekedMessage>> {
        let address = entity.address(sub_queue);
        let mut guard = self.amqp.lock().await;
        if guard.is_none() {
            *guard = Some(ServiceBusConnection::open(&self.ns).await?);
        }
        let conn = guard.as_mut().expect("opened above");
        let result = peek_all(conn, &address, from_sequence, max).await;
        if matches!(result, Err(Error::Network(_) | Error::Protocol(_) | Error::Timeout(_))) {
            // Drop a possibly broken connection; the next call reconnects.
            if let Some(conn) = guard.take() {
                let _ = conn.close().await;
            }
        }
        result
    }

    /// Close the AMQP connection, if open.
    pub async fn close(&self) {
        if let Some(conn) = self.amqp.lock().await.take() {
            let _ = conn.close().await;
        }
    }
}

async fn peek_all(conn: &mut ServiceBusConnection, address: &str, from_sequence: i64, max: u32) -> Result<Vec<PeekedMessage>> {
    let mut out: Vec<PeekedMessage> = Vec::new();
    let mut next = from_sequence;
    while (out.len() as u32) < max {
        let want = (max - out.len() as u32).min(peek::MAX_PER_REQUEST);
        let batch = peek::peek(conn, address, next, want).await?;
        let got = batch.len() as u32;
        if let Some(last) = batch.last() {
            next = last.sequence_number + 1;
        }
        out.extend(batch);
        if got < want {
            break;
        }
    }
    Ok(out)
}
```

Replace `src/lib.rs` with the complete module list:
```rust
//! Core library for Dispatched, a cross-platform Azure Service Bus explorer.
//! All Service Bus behaviour lives here; the CLI, MCP server and desktop app
//! only translate input and render output.

#![deny(rust_2018_idioms, unsafe_code)]
#![warn(clippy::all)]

pub mod connstr;
pub mod error;
pub mod explorer;
pub mod messaging;
pub mod mgmt;
pub mod namespace;
pub mod tokens;

pub use connstr::ConnectionString;
pub use error::{Error, Result};
pub use explorer::Explorer;
pub use messaging::{MessageBody, PeekedMessage, PropValue};
pub use mgmt::{EntityCounts, ManagementClient, QueueInfo, SubscriptionInfo, TopicInfo};
pub use namespace::{EmulatorPorts, EntityPath, Namespace, SubQueue};
pub use tokens::{ServiceBusToken, TokenKind, TokenSource};
```

- [ ] **Step 4: Run tests and clippy, with and without the feature**

Run: `cargo test -p dispatched-core && cargo clippy -p dispatched-core --all-targets -- -D warnings && cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings`
Expected: all unit tests PASS (the peek tests included), and clippy is clean both ways.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: peek over \$management, Explorer facade, test seeding helpers"
```

---
### Task 16: Emulator harness and the M0 gate test

**Files:**
- Create: `dispatched/testing/emulator/docker-compose.yml`, `Config.json`, `.env.example`, `emulator.sh`
- Create: `crates/dispatched-core/tests/emulator_m0.rs`

**Interfaces:**
- Consumes: `ConnectionString`, `Namespace`, `Explorer`, `EntityPath`, `SubQueue`, `messaging::{ServiceBusConnection, testing}`.
- Produces: `testing/emulator/emulator.sh up|down|status`, and the gated integration test `emulator_m0`, which runs only when `DISPATCHED_EMULATOR=1`.

**EULA:** `ACCEPT_EULA=Y` accepts the Service Bus Emulator and SQL Server licence terms. The person running the harness is the one accepting them. Show `.env.example` to the user the first time.

- [ ] **Step 1: Write the harness**

`testing/emulator/docker-compose.yml`:
```yaml
name: dispatched-servicebus-emulator
services:
  emulator:
    image: mcr.microsoft.com/azure-messaging/servicebus-emulator:2.0.1
    volumes:
      - "./Config.json:/ServiceBus_Emulator/ConfigFiles/Config.json"
    ports:
      - "5672:5672"
      - "5300:5300"
    environment:
      SQL_SERVER: mssql
      MSSQL_SA_PASSWORD: "${MSSQL_SA_PASSWORD}"
      ACCEPT_EULA: "${ACCEPT_EULA}"
      EMULATOR_HTTP_PORT: 5300
    depends_on: [mssql]
  mssql:
    image: mcr.microsoft.com/mssql/server:2022-latest
    platform: linux/amd64
    environment:
      ACCEPT_EULA: "${ACCEPT_EULA}"
      MSSQL_SA_PASSWORD: "${MSSQL_SA_PASSWORD}"
```

`testing/emulator/.env.example` (the password must not contain `;`):
```
ACCEPT_EULA=Y
MSSQL_SA_PASSWORD=Dispatched-Emulator-2026!
```

`testing/emulator/Config.json`:
```json
{
  "UserConfig": {
    "Namespaces": [
      {
        "Name": "sbemulatorns",
        "Queues": [
          { "Name": "orders", "Properties": {
              "DeadLetteringOnMessageExpiration": true, "DefaultMessageTimeToLive": "PT1H",
              "DuplicateDetectionHistoryTimeWindow": "PT20S", "ForwardDeadLetteredMessagesTo": "",
              "ForwardTo": "", "LockDuration": "PT1M", "MaxDeliveryCount": 3,
              "RequiresDuplicateDetection": false, "RequiresSession": false } },
          { "Name": "sessions-q", "Properties": {
              "DeadLetteringOnMessageExpiration": false, "DefaultMessageTimeToLive": "PT1H",
              "DuplicateDetectionHistoryTimeWindow": "PT20S", "ForwardDeadLetteredMessagesTo": "",
              "ForwardTo": "", "LockDuration": "PT1M", "MaxDeliveryCount": 10,
              "RequiresDuplicateDetection": false, "RequiresSession": true } }
        ],
        "Topics": [
          { "Name": "events",
            "Properties": { "DefaultMessageTimeToLive": "PT1H",
              "DuplicateDetectionHistoryTimeWindow": "PT20S", "RequiresDuplicateDetection": false },
            "Subscriptions": [
              { "Name": "all", "Properties": {
                  "DeadLetteringOnMessageExpiration": false, "DefaultMessageTimeToLive": "PT1H",
                  "LockDuration": "PT1M", "MaxDeliveryCount": 3,
                  "ForwardDeadLetteredMessagesTo": "", "ForwardTo": "", "RequiresSession": false } }
            ] }
        ]
      }
    ],
    "Logging": { "Type": "Console" }
  }
}
```

`testing/emulator/emulator.sh`:
```bash
#!/usr/bin/env bash
# Start/stop the Service Bus Emulator used by Dispatched's integration tests.
set -euo pipefail
cd "$(dirname "$0")"

case "${1:-}" in
  up)
    [ -f .env ] || { cp .env.example .env; echo "Created testing/emulator/.env (ACCEPT_EULA=Y accepts the emulator and SQL Server EULAs)."; }
    docker compose --env-file .env up -d
    echo "Waiting for http://localhost:5300/health ..."
    for _ in $(seq 1 90); do
      if curl -fsS http://localhost:5300/health >/dev/null 2>&1; then echo "Emulator healthy."; exit 0; fi
      sleep 2
    done
    echo "Emulator did not become healthy within 180s:"; docker compose logs --tail 80 emulator; exit 1
    ;;
  down)
    docker compose --env-file .env down -v
    ;;
  status)
    curl -fsS http://localhost:5300/health && echo
    ;;
  *)
    echo "usage: $0 up|down|status"; exit 2
    ;;
esac
```

Run: `chmod +x testing/emulator/emulator.sh`

- [ ] **Step 2: Write the gate test**

`crates/dispatched-core/tests/emulator_m0.rs`:
```rust
//! M0 gate: list → peek → peek DLQ with reason, against the Service Bus Emulator.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 -- --nocapture`

use dispatched_core::messaging::{ServiceBusConnection, testing};
use dispatched_core::{ConnectionString, EntityPath, Explorer, Namespace, PeekedMessage, SubQueue};
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

fn run_of(m: &PeekedMessage) -> Option<&str> {
    m.application_properties.get("run").and_then(|v| v.as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn m0_gate_flow_against_emulator() {
    if !enabled() {
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns = Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();

    // Seed: clean slate, 3 messages, dead-letter the first with a reason.
    let mut conn = ServiceBusConnection::open(&ns).await.expect("AMQP connect to emulator");
    testing::drain(&mut conn, "orders", Duration::from_secs(2)).await.unwrap();
    testing::drain(&mut conn, "orders/$DeadLetterQueue", Duration::from_secs(2)).await.unwrap();
    testing::drain(&mut conn, "events/Subscriptions/all", Duration::from_secs(2)).await.unwrap();
    for i in 0..3 {
        testing::send_text(&mut conn, "orders", &format!("{{\"run\":\"{run}\",\"i\":{i}}}"), &[("run", &run)])
            .await
            .unwrap();
    }
    assert!(
        testing::dead_letter_next(&mut conn, "orders", "m0-gate", "dead-lettered by the M0 gate test", Duration::from_secs(10))
            .await
            .unwrap(),
        "a message should have been received for dead-lettering"
    );
    conn.close().await.unwrap();

    let explorer = Explorer::new(ns);

    // 1. List queues over ATOM (HTTP :5300).
    let queues = explorer.list_queues().await.expect("ATOM list queues on emulator");
    let orders = queues.iter().find(|q| q.name == "orders").expect("orders queue listed");
    eprintln!("GATE-NOTE emulator counts for orders: {:?}", orders.counts);
    assert!(queues.iter().any(|q| q.name == "sessions-q" && q.requires_session));

    // 2. Peek active messages over AMQP $management.
    let active = explorer.peek(&EntityPath::queue("orders"), SubQueue::Active, 0, 100).await.unwrap();
    let ours: Vec<_> = active.iter().filter(|m| run_of(m) == Some(run.as_str())).collect();
    assert_eq!(ours.len(), 2, "two of three messages remain active");
    assert!(ours[0].sequence_number < ours[1].sequence_number);
    assert!(ours[0].enqueued_time.is_some());
    assert!(ours[0].body.as_text().unwrap().contains(&run));

    // 3. Peek the dead-letter queue and see the reason.
    let dead = explorer.peek(&EntityPath::queue("orders"), SubQueue::DeadLetter, 0, 100).await.unwrap();
    let dead = dead.iter().find(|m| run_of(m) == Some(run.as_str())).expect("dead-lettered message visible");
    assert_eq!(dead.dead_letter_reason.as_deref(), Some("m0-gate"));
    assert_eq!(dead.dead_letter_description.as_deref(), Some("dead-lettered by the M0 gate test"));

    // 4. Empty entity and past-the-end peeks are empty, not errors.
    let sub = EntityPath::subscription("events", "all");
    assert!(explorer.peek(&sub, SubQueue::Active, 0, 10).await.unwrap().is_empty());
    let last = ours[1].sequence_number;
    assert!(
        explorer
            .peek(&EntityPath::queue("orders"), SubQueue::Active, last + 10_000, 10)
            .await
            .unwrap()
            .is_empty()
    );

    explorer.close().await;
}
```

Modify `crates/dispatched-core/Cargo.toml`:
- Add `uuid = { workspace = true }` to `[dev-dependencies]`.
- Append the test target, so the test only builds with the seeding helpers:
```toml
[[test]]
name = "emulator_m0"
required-features = ["test-support"]
```

- [ ] **Step 3: Verify the test is skipped by default**

Run: `cargo test -p dispatched-core --features test-support --test emulator_m0 -- --nocapture`
Expected: PASS, printing `skipped: set DISPATCHED_EMULATOR=1 ...`.

- [ ] **Step 4: Run against the emulator locally**

Run:
- `df -h /System/Volumes/Data` (stop if under 30 GB free; the images are about 2 GB)
- `docker info >/dev/null 2>&1 || echo "Docker not running"`, then ask the user to start Docker Desktop if needed.
- `testing/emulator/emulator.sh up`
- `DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 -- --nocapture`

Expected: PASS, plus a `GATE-NOTE emulator counts ...` line. Record that line for Task 20.

Known risks and what to do about each:
- **mssql crashes under Rosetta on Apple Silicon.** Record it, and rely on the CI emulator job (Task 19). This is not a code failure.
- **`list_queues` fails on the emulator** (missing `$Resources` listing support). Record the error verbatim for the gate report and continue with steps 2–4 of the test by temporarily commenting out step 1. Do not commit that change. The gate report (Task 20) decides the M1 fallback.
- **A peek hangs until the 65 s timeout.** The management reply routing is wrong. Check that `ManagementLink` sets the receiver `target` to `reply_to`, and compare with azservicebus `amqp_management_link.rs`. This is the riskiest protocol point (R1 in research).

Then: `testing/emulator/emulator.sh down`

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Emulator harness and M0 gate integration test"
```

---

### Task 17: Namespace discovery and the `dispatched` CLI

**Files:**
- Create: `crates/dispatched-core/src/discovery.rs`
- Modify: `crates/dispatched-core/src/lib.rs`
- Create: `crates/dispatched-cli/Cargo.toml`, `src/main.rs`, `src/state.rs`, `src/resolve.rs`, `src/output.rs`
- Modify: `dispatched/Cargo.toml` (add the CLI to `members` and `default-members`)

**Interfaces:**
- Consumes: kit `EntraSession`, `SignIn`, `OsKeychain`, `ArmClient`, `AzureCloud`; core `Explorer`, `Namespace`, `ConnectionString`, `EntityPath`, `SubQueue`, `tokens::EntraSource`.
- Produces:
  - `dispatched_core::discovery::{DiscoveredNamespace, discover_namespaces(&ArmClient) -> Result<Vec<DiscoveredNamespace>>, NAMESPACES_API}`.
  - Binary `dispatched` with these subcommands:
    - `login [--device-code] [--tenant T]`, `logout`, `whoami`, `namespaces`
    - `ls [--topics]`
    - `peek <ENTITY> [--dlq | --transfer-dlq] [--top N] [--from-seq N]`
  - Global flags: `--connection-string` (env `DISPATCHED_CONNECTION_STRING`), `--namespace` (env `DISPATCHED_NAMESPACE`), `--format table|json`.
  - Keychain service `io.horizon-tech.dispatched`, prefix `dispatched`. State file at `{config_dir}/dispatched/cli-state.json`, overridable with `DISPATCHED_CONFIG_DIR`.

- [ ] **Step 1: Write discovery.rs with tests**

`crates/dispatched-core/src/discovery.rs`:
```rust
//! Find Service Bus namespaces visible to an Entra sign-in (via ARM).

use crate::Result;
use horizon_azure_kit::KitError;
use horizon_azure_kit::arm::{ArmClient, Subscription};
use serde::{Deserialize, Serialize};

/// ARM api-version for `Microsoft.ServiceBus/namespaces`.
pub const NAMESPACES_API: &str = "2024-01-01";

/// A namespace found in a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredNamespace {
    /// Namespace name.
    pub name: String,
    /// Host, e.g. `myns.servicebus.windows.net`.
    pub host: String,
    /// ARM resource id.
    pub resource_id: String,
    /// Subscription id.
    pub subscription_id: String,
    /// Subscription display name.
    pub subscription_name: String,
    /// Azure region.
    pub location: Option<String>,
    /// `Basic`, `Standard` or `Premium`.
    pub tier: Option<String>,
    /// SAS (local auth) disabled: Entra only.
    pub local_auth_disabled: bool,
}

#[derive(Deserialize)]
struct RawNamespace {
    id: String,
    name: String,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    sku: Option<RawSku>,
    #[serde(default)]
    properties: RawProperties,
}

#[derive(Deserialize)]
struct RawSku {
    #[serde(default)]
    tier: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawProperties {
    #[serde(default)]
    service_bus_endpoint: Option<String>,
    #[serde(default)]
    disable_local_auth: Option<bool>,
}

fn host_from(endpoint: Option<&str>, name: &str) -> String {
    endpoint
        .and_then(|e| url::Url::parse(e).ok())
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| format!("{}.servicebus.windows.net", name.to_ascii_lowercase()))
}

fn map(sub: &Subscription, raw: RawNamespace) -> DiscoveredNamespace {
    DiscoveredNamespace {
        host: host_from(raw.properties.service_bus_endpoint.as_deref(), &raw.name),
        name: raw.name,
        resource_id: raw.id,
        subscription_id: sub.subscription_id.clone(),
        subscription_name: sub.display_name.clone(),
        location: raw.location,
        tier: raw.sku.and_then(|s| s.tier),
        local_auth_disabled: raw.properties.disable_local_auth.unwrap_or(false),
    }
}

/// All namespaces in all enabled subscriptions the token can read.
/// Subscriptions that deny access are skipped.
pub async fn discover_namespaces(arm: &ArmClient) -> Result<Vec<DiscoveredNamespace>> {
    let mut out = Vec::new();
    for sub in arm.list_subscriptions().await? {
        if sub.state.as_deref() == Some("Disabled") {
            continue;
        }
        match arm
            .list_by_provider::<RawNamespace>(&sub.subscription_id, "Microsoft.ServiceBus/namespaces", NAMESPACES_API)
            .await
        {
            Ok(list) => out.extend(list.into_iter().map(|raw| map(&sub, raw))),
            Err(KitError::Forbidden(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;
    use secrecy::SecretString;

    #[tokio::test]
    async fn discovers_namespaces_and_skips_forbidden_subscriptions() {
        let mut server = mockito::Server::new_async().await;
        let _subs = server
            .mock("GET", "/subscriptions")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"value":[{"subscriptionId":"s1","displayName":"Prod"},{"subscriptionId":"s2","displayName":"Locked"}]}"#)
            .create_async()
            .await;
        let _ns = server
            .mock("GET", "/subscriptions/s1/providers/Microsoft.ServiceBus/namespaces")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_body(r#"{"value":[{"id":"/subscriptions/s1/resourceGroups/rg/providers/Microsoft.ServiceBus/namespaces/SBPremium","name":"SBPremium","location":"West Europe","sku":{"name":"Premium","tier":"Premium"},"properties":{"serviceBusEndpoint":"https://SBPremium.servicebus.windows.net:443/","disableLocalAuth":true}}]}"#)
            .create_async()
            .await;
        let _denied = server
            .mock("GET", "/subscriptions/s2/providers/Microsoft.ServiceBus/namespaces")
            .match_query(Matcher::Any)
            .with_status(403)
            .with_body("AuthorizationFailed")
            .create_async()
            .await;
        let arm = ArmClient::with_base(server.url(), SecretString::from("tok"));
        let found = discover_namespaces(&arm).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].host, "sbpremium.servicebus.windows.net");
        assert_eq!(found[0].tier.as_deref(), Some("Premium"));
        assert!(found[0].local_auth_disabled);
        assert_eq!(found[0].subscription_name, "Prod");
    }
}
```

Modify `crates/dispatched-core/src/lib.rs`. Add `pub mod discovery;` to the module list.

Run: `cargo test -p dispatched-core discovery`
Expected: PASS, 1 test.

- [ ] **Step 2: Create the CLI crate**

`dispatched/Cargo.toml`. Change the members to:
```toml
members = ["crates/dispatched-core", "crates/dispatched-cli"]
default-members = ["crates/dispatched-core", "crates/dispatched-cli"]
```

`crates/dispatched-cli/Cargo.toml`:
```toml
[package]
name = "dispatched"
description = "Fast, cross-platform Azure Service Bus explorer (CLI)."
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true

[[bin]]
name = "dispatched"
path = "src/main.rs"

[dependencies]
dispatched-core = { workspace = true }
horizon-azure-kit = { workspace = true }
anyhow = { workspace = true }
clap = { workspace = true }
dirs = { workspace = true }
open = { workspace = true }
secrecy = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
time = { workspace = true }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

[dev-dependencies]
tempfile = "3"
```

`src/state.rs`:
```rust
//! Small non-secret CLI state (which Entra sign-in is current).

use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::OsKeychain;
use horizon_azure_kit::AzureCloud;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
pub const KEY_PREFIX: &str = "dispatched";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CliState {
    pub sign_in: Option<SignIn>,
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("DISPATCHED_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("dispatched"))
}

fn state_path() -> PathBuf {
    config_dir().join("cli-state.json")
}

pub fn load() -> CliState {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(state: &CliState) -> anyhow::Result<()> {
    let path = state_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

pub fn entra_session() -> Arc<EntraSession> {
    Arc::new(EntraSession::new(AzureCloud::Public, Arc::new(OsKeychain::new(KEYCHAIN_SERVICE)), KEY_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrips_in_custom_dir() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: single-threaded test; edition 2024 requires `unsafe` for set_var.
        unsafe { std::env::set_var("DISPATCHED_CONFIG_DIR", dir.path()) };
        assert!(load().sign_in.is_none());
        let state = CliState {
            sign_in: Some(SignIn {
                id: "o".into(),
                user_principal: "a@x.io".into(),
                tenant_id: "t".into(),
                display_name: None,
                cloud: AzureCloud::Public,
            }),
        };
        save(&state).unwrap();
        assert_eq!(load().sign_in.unwrap().user_principal, "a@x.io");
        unsafe { std::env::remove_var("DISPATCHED_CONFIG_DIR") };
    }
}
```

The workspace lint `unsafe_code` is denied only in `dispatched-core`; the CLI's test uses `unsafe` for `set_var`, as edition 2024 requires.

`src/resolve.rs`:
```rust
//! Turn CLI flags into a `Namespace`.

use crate::state::CliState;
use anyhow::{Context, bail};
use dispatched_core::tokens::EntraSource;
use dispatched_core::{ConnectionString, Namespace};
use horizon_azure_kit::auth::EntraSession;
use std::sync::Arc;

pub fn namespace_host(input: &str) -> String {
    let input = input.trim().to_ascii_lowercase();
    if input.contains('.') { input } else { format!("{input}.servicebus.windows.net") }
}

pub fn namespace(
    connection_string: Option<&str>,
    namespace: Option<&str>,
    session: &Arc<EntraSession>,
    state: &CliState,
) -> anyhow::Result<Namespace> {
    if let Some(cs) = connection_string {
        return Ok(Namespace::from_connection_string(&ConnectionString::parse(cs)?)?);
    }
    if let Some(ns) = namespace {
        let sign_in = state
            .sign_in
            .clone()
            .context("not signed in: run `dispatched login`, or pass --connection-string")?;
        let tokens = Arc::new(EntraSource::new(session.clone(), sign_in, None));
        return Ok(Namespace::new(namespace_host(ns), tokens));
    }
    bail!("no namespace: pass --connection-string (or set DISPATCHED_CONNECTION_STRING), or --namespace after `dispatched login`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_expand_to_public_cloud_host() {
        assert_eq!(namespace_host("MyNs"), "myns.servicebus.windows.net");
        assert_eq!(namespace_host("x.servicebus.chinacloudapi.cn"), "x.servicebus.chinacloudapi.cn");
    }

    #[test]
    fn namespace_without_sign_in_explains_what_to_do() {
        let session = crate::state::entra_session();
        let err = namespace(None, Some("myns"), &session, &CliState::default()).unwrap_err();
        assert!(err.to_string().contains("dispatched login"));
    }

    #[test]
    fn nothing_given_explains_both_options() {
        let session = crate::state::entra_session();
        let err = namespace(None, None, &session, &CliState::default()).unwrap_err();
        assert!(err.to_string().contains("--connection-string"));
    }
}
```

`src/output.rs`:
```rust
//! Plain aligned tables for humans; JSON for machines.

pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = vec![line(headers.iter().map(|h| h.to_string()).collect())];
    out.extend(rows.iter().map(|r| line(r.clone())));
    out.join("\n")
}

pub fn ellipsize(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_columns() {
        let t = table(&["NAME", "N"], &[vec!["orders".into(), "3".into()], vec!["q".into(), "10".into()]]);
        assert_eq!(t, "NAME    N\norders  3\nq       10");
    }

    #[test]
    fn ellipsize_flattens_newlines() {
        assert_eq!(ellipsize("a\nb", 10), "a b");
        assert_eq!(ellipsize("abcdef", 4), "abc…");
    }
}
```

`src/main.rs`:
```rust
//! `dispatched`: Azure Service Bus explorer CLI.

mod output;
mod resolve;
mod state;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use dispatched_core::discovery::discover_namespaces;
use dispatched_core::{EntityPath, Explorer, MessageBody, PeekedMessage, SubQueue};
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::arm::ArmClient;
use output::{ellipsize, table};
use std::process::ExitCode;
use time::format_description::well_known::Rfc3339;

#[derive(Parser)]
#[command(name = "dispatched", version, about = "Fast, cross-platform Azure Service Bus explorer")]
struct Cli {
    /// Service Bus connection string (SAS or emulator).
    #[arg(long, env = "DISPATCHED_CONNECTION_STRING", global = true, hide_env_values = true)]
    connection_string: Option<String>,
    /// Namespace name or host, used with the signed-in Entra account.
    #[arg(long, env = "DISPATCHED_NAMESPACE", global = true)]
    namespace: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Table, global = true)]
    format: Format,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Table,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in with Microsoft Entra ID (browser by default).
    Login {
        /// Use the device-code flow (for SSH / headless).
        #[arg(long)]
        device_code: bool,
        /// Tenant id or domain.
        #[arg(long, default_value = "organizations")]
        tenant: String,
    },
    /// Sign out and forget the stored refresh token.
    Logout,
    /// Show the current sign-in.
    Whoami,
    /// List Service Bus namespaces visible to the signed-in account.
    Namespaces,
    /// List queues (and with --topics, topics and subscriptions) with counts.
    Ls {
        #[arg(long)]
        topics: bool,
    },
    /// Peek messages without removing them.
    Peek {
        /// `queue` or `topic/subscriptions/subscription`.
        entity: String,
        /// Read the dead-letter queue.
        #[arg(long, conflicts_with = "transfer_dlq")]
        dlq: bool,
        /// Read the transfer dead-letter queue.
        #[arg(long)]
        transfer_dlq: bool,
        /// Maximum messages.
        #[arg(long, default_value_t = 20)]
        top: u32,
        /// Start at this sequence number.
        #[arg(long, default_value_t = 0)]
        from_seq: i64,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn count_cell(n: Option<u64>) -> String {
    n.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
}

fn body_cell(body: &MessageBody) -> String {
    match body.as_text() {
        Some(text) => ellipsize(&text, 60),
        None => format!("<{} bytes>", body.len()),
    }
}

fn print_messages(messages: &[PeekedMessage], format: Format, dlq: bool) -> anyhow::Result<()> {
    if let Format::Json = format {
        println!("{}", serde_json::to_string_pretty(messages)?);
        return Ok(());
    }
    let mut headers = vec!["SEQ", "ENQUEUED", "MESSAGE ID", "DELIV", "SUBJECT"];
    if dlq {
        headers.push("REASON");
    }
    headers.push("BODY");
    let rows: Vec<Vec<String>> = messages
        .iter()
        .map(|m| {
            let mut row = vec![
                m.sequence_number.to_string(),
                m.enqueued_time.and_then(|t| t.format(&Rfc3339).ok()).unwrap_or_default(),
                ellipsize(m.message_id.as_deref().unwrap_or(""), 36),
                m.delivery_count.to_string(),
                ellipsize(m.subject.as_deref().unwrap_or(""), 24),
            ];
            if dlq {
                row.push(ellipsize(m.dead_letter_reason.as_deref().unwrap_or(""), 32));
            }
            row.push(body_cell(&m.body));
            row
        })
        .collect();
    println!("{}", table(&headers, &rows));
    if messages.is_empty() {
        eprintln!("(no messages)");
    }
    Ok(())
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let session = state::entra_session();
    let mut st = state::load();
    match cli.command {
        Command::Login { device_code, tenant } => {
            let sign_in = if device_code {
                session.login_device_code(&tenant, |dc| eprintln!("{}", dc.message)).await?
            } else {
                session
                    .login_browser(&tenant, |url| {
                        eprintln!("Opening your browser to sign in. If it does not open, visit:\n{url}");
                        let _ = open::that(url);
                    })
                    .await?
            };
            eprintln!("Signed in as {} (tenant {}).", sign_in.user_principal, sign_in.tenant_id);
            st.sign_in = Some(sign_in);
            state::save(&st)?;
        }
        Command::Logout => {
            if let Some(sign_in) = st.sign_in.take() {
                session.logout(&sign_in)?;
                state::save(&st)?;
                eprintln!("Signed out {}.", sign_in.user_principal);
            } else {
                eprintln!("Not signed in.");
            }
        }
        Command::Whoami => match &st.sign_in {
            Some(s) => println!("{} (tenant {})", s.user_principal, s.tenant_id),
            None => println!("Not signed in."),
        },
        Command::Namespaces => {
            let sign_in = st.sign_in.clone().context("not signed in: run `dispatched login`")?;
            let token = session.token_for(&sign_in, None, &AzureCloud::Public.arm_scope()).await?;
            let arm = ArmClient::new(AzureCloud::Public, token.token);
            let found = discover_namespaces(&arm).await?;
            if let Format::Json = cli.format {
                println!("{}", serde_json::to_string_pretty(&found)?);
            } else {
                let rows: Vec<Vec<String>> = found
                    .iter()
                    .map(|n| {
                        vec![
                            n.name.clone(),
                            n.host.clone(),
                            n.tier.clone().unwrap_or_default(),
                            n.location.clone().unwrap_or_default(),
                            n.subscription_name.clone(),
                            if n.local_auth_disabled { "Entra only".into() } else { String::new() },
                        ]
                    })
                    .collect();
                println!("{}", table(&["NAME", "HOST", "TIER", "LOCATION", "SUBSCRIPTION", "AUTH"], &rows));
            }
        }
        Command::Ls { topics } => {
            let ns = resolve::namespace(cli.connection_string.as_deref(), cli.namespace.as_deref(), &session, &st)?;
            let explorer = Explorer::new(ns);
            let queues = explorer.list_queues().await?;
            if let Format::Json = cli.format {
                let mut value = serde_json::json!({ "queues": queues });
                if topics {
                    let mut list = Vec::new();
                    for t in explorer.list_topics().await? {
                        let subs = explorer.list_subscriptions(&t.name).await?;
                        list.push(serde_json::json!({ "topic": t, "subscriptions": subs }));
                    }
                    value["topics"] = serde_json::Value::Array(list);
                }
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = queues
                .iter()
                .map(|q| {
                    vec![
                        q.name.clone(),
                        count_cell(q.counts.as_ref().map(|c| c.active)),
                        count_cell(q.counts.as_ref().map(|c| c.dead_letter)),
                        count_cell(q.counts.as_ref().map(|c| c.scheduled)),
                        if q.requires_session { "yes".into() } else { String::new() },
                        q.status.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            println!("{}", table(&["QUEUE", "ACTIVE", "DLQ", "SCHEDULED", "SESSIONS", "STATUS"], &rows));
            if topics {
                let mut rows = Vec::new();
                for t in explorer.list_topics().await? {
                    for s in explorer.list_subscriptions(&t.name).await? {
                        rows.push(vec![
                            format!("{}/subscriptions/{}", t.name, s.name),
                            count_cell(s.counts.as_ref().map(|c| c.active)),
                            count_cell(s.counts.as_ref().map(|c| c.dead_letter)),
                            s.status.clone().unwrap_or_default(),
                        ]);
                    }
                }
                println!("\n{}", table(&["SUBSCRIPTION", "ACTIVE", "DLQ", "STATUS"], &rows));
            }
        }
        Command::Peek { entity, dlq, transfer_dlq, top, from_seq } => {
            let ns = resolve::namespace(cli.connection_string.as_deref(), cli.namespace.as_deref(), &session, &st)?;
            let explorer = Explorer::new(ns);
            let path = EntityPath::parse(&entity)?;
            let sub_queue = match (dlq, transfer_dlq) {
                (true, _) => SubQueue::DeadLetter,
                (_, true) => SubQueue::TransferDeadLetter,
                _ => SubQueue::Active,
            };
            let messages = explorer.peek(&path, sub_queue, from_seq, top).await?;
            explorer.close().await;
            print_messages(&messages, cli.format, dlq || transfer_dlq)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Run tests and a help smoke test**

Run: `cargo test -p dispatched && cargo run -q -p dispatched -- --help && cargo run -q -p dispatched -- peek --help`
Expected: tests PASS (state, resolve, output: 6 tests), and the help output lists `login, logout, whoami, namespaces, ls, peek`.

- [ ] **Step 4: End-to-end CLI check against the emulator**

Run:
```bash
testing/emulator/emulator.sh up
export DISPATCHED_CONNECTION_STRING='Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;'
DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0
cargo run -q -p dispatched -- ls --topics
cargo run -q -p dispatched -- peek orders
cargo run -q -p dispatched -- peek orders --dlq
cargo run -q -p dispatched -- peek orders --dlq --format json | head -40
testing/emulator/emulator.sh down
```
Expected:
- `ls` shows `orders` and `sessions-q`, and `events/subscriptions/all`.
- `peek orders` shows 2+ rows.
- `peek orders --dlq` shows a row with REASON `m0-gate`.
- JSON has `"dead_letter_reason": "m0-gate"`.

If the emulator can't run locally (Task 16 risks), skip this step and note it for Task 20.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Namespace discovery and the dispatched CLI (login, namespaces, ls, peek)"
```

---

### Task 18: Desktop app skeleton (Tauri 2 + React)

**Files:**
- Create: `app/package.json`, `app/index.html`, `app/vite.config.ts`, `app/tsconfig.json`, `app/src/main.tsx`, `app/src/App.tsx`, `app/src/api.ts`
- Create: `app/src-tauri/Cargo.toml`, `build.rs`, `tauri.conf.json`, `capabilities/default.json`, `src/main.rs`, `src/lib.rs`, `src/state.rs`, `src/error.rs`, `src/commands/mod.rs`, `src/commands/connection.rs`, `src/commands/browse.rs`
- Create: `app/src-tauri/icons/*` (generated)
- Modify: `dispatched/Cargo.toml` (add `app/src-tauri` to `members` only)

**Interfaces:**
- Consumes: `Explorer`, `Namespace`, `ConnectionString`, `EntityPath`, `SubQueue`, `QueueInfo`, `PeekedMessage`, `Error`.
- Produces these Tauri commands:
  - `connect_connection_string(connection_string: String) -> ConnectionInfo { host, emulator, auth }`
  - `disconnect()`
  - `list_queues() -> Vec<QueueInfo>`
  - `peek(entity: String, sub_queue: "active"|"dead-letter"|"transfer-dead-letter", from_sequence: i64, max: u32) -> Vec<PeekedMessage>`
  - Errors serialize as `{ kind, message }`.
- The UI is plumbing proof only (see Global Constraints): semantic HTML with no styling work. Entra sign-in in the GUI is M1.

- [ ] **Step 1: Frontend scaffold**

`app/package.json`:
```json
{
  "name": "dispatched-app",
  "private": true,
  "version": "0.0.1",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2",
    "react": "^19.1.0",
    "react-dom": "^19.1.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2",
    "@types/react": "^19",
    "@types/react-dom": "^19",
    "@vitejs/plugin-react": "^5",
    "typescript": "^5.9",
    "vite": "^7"
  }
}
```

`app/index.html`:
```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Dispatched</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`app/vite.config.ts`:
```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari14",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
```

`app/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2021",
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "skipLibCheck": true,
    "isolatedModules": true,
    "noEmit": true
  },
  "include": ["src"]
}
```

`app/src/api.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export type SubQueue = "active" | "dead-letter" | "transfer-dead-letter";

export interface CommandError { kind: string; message: string }
export interface ConnectionInfo { host: string; emulator: boolean; auth: string }
export interface EntityCounts { active: number; dead_letter: number; scheduled: number; transfer: number; transfer_dead_letter: number }
export interface QueueInfo { name: string; status: string | null; counts: EntityCounts | null; requires_session: boolean }
export type MessageBody =
  | { kind: "binary"; value: string }
  | { kind: "text" | "value" | "sequence"; value: string }
  | { kind: "empty" };
export interface PeekedMessage {
  sequence_number: number;
  message_id: string | null;
  subject: string | null;
  enqueued_time: string | null;
  delivery_count: number;
  dead_letter_reason: string | null;
  dead_letter_description: string | null;
  body: MessageBody;
}

export const connectConnectionString = (connectionString: string) =>
  invoke<ConnectionInfo>("connect_connection_string", { connectionString });
export const disconnect = () => invoke<void>("disconnect");
export const listQueues = () => invoke<QueueInfo[]>("list_queues");
export const peek = (entity: string, subQueue: SubQueue, fromSequence = 0, max = 50) =>
  invoke<PeekedMessage[]>("peek", { entity, subQueue, fromSequence, max });

export function bodyPreview(body: MessageBody): string {
  if (body.kind === "empty") return "";
  if (body.kind === "binary") {
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(Uint8Array.from(atob(body.value), (c) => c.charCodeAt(0)));
    } catch {
      return `<${atob(body.value).length} bytes>`;
    }
  }
  return body.value;
}
```

`app/src/main.tsx`:
```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

`app/src/App.tsx`:
```tsx
// M0 plumbing proof: connect → list queues → peek / peek DLQ.
// No visual design here; the design system is M1 (impeccable + taste).
import { useState } from "react";
import * as api from "./api";

export default function App() {
  const [connectionString, setConnectionString] = useState("");
  const [info, setInfo] = useState<api.ConnectionInfo | null>(null);
  const [queues, setQueues] = useState<api.QueueInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [subQueue, setSubQueue] = useState<api.SubQueue>("active");
  const [messages, setMessages] = useState<api.PeekedMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function guarded<T>(work: () => Promise<T>): Promise<T | undefined> {
    setBusy(true);
    setError(null);
    try {
      return await work();
    } catch (e) {
      const err = e as api.CommandError;
      setError(err?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  const connect = () =>
    guarded(async () => {
      const i = await api.connectConnectionString(connectionString);
      setInfo(i);
      setQueues(await api.listQueues());
    });

  const open = (name: string, sq: api.SubQueue) =>
    guarded(async () => {
      setSelected(name);
      setSubQueue(sq);
      setMessages(await api.peek(name, sq));
    });

  return (
    <main>
      <h1>Dispatched</h1>
      {!info ? (
        <form onSubmit={(e) => { e.preventDefault(); void connect(); }}>
          <label>
            Connection string{" "}
            <input type="password" value={connectionString} onChange={(e) => setConnectionString(e.target.value)} size={80} />
          </label>{" "}
          <button type="submit" disabled={busy || !connectionString}>Connect</button>
        </form>
      ) : (
        <p>
          Connected to <strong>{info.host}</strong> {info.emulator ? "(emulator)" : ""} via {info.auth}.{" "}
          <button onClick={() => guarded(async () => { await api.disconnect(); setInfo(null); setQueues([]); setMessages([]); })}>Disconnect</button>
        </p>
      )}
      {error && <p role="alert">Error: {error}</p>}
      {queues.length > 0 && (
        <table>
          <thead><tr><th>Queue</th><th>Active</th><th>DLQ</th><th></th></tr></thead>
          <tbody>
            {queues.map((q) => (
              <tr key={q.name}>
                <td>{q.name}</td>
                <td>{q.counts?.active ?? "?"}</td>
                <td>{q.counts?.dead_letter ?? "?"}</td>
                <td>
                  <button onClick={() => open(q.name, "active")} disabled={busy}>Peek</button>{" "}
                  <button onClick={() => open(q.name, "dead-letter")} disabled={busy}>Peek DLQ</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {selected && (
        <section>
          <h2>{selected} {subQueue === "active" ? "" : "— dead-letter"}</h2>
          <table>
            <thead><tr><th>Seq</th><th>Enqueued</th><th>Message ID</th><th>Deliveries</th><th>Reason</th><th>Body</th></tr></thead>
            <tbody>
              {messages.map((m) => (
                <tr key={m.sequence_number}>
                  <td>{m.sequence_number}</td>
                  <td>{m.enqueued_time}</td>
                  <td>{m.message_id}</td>
                  <td>{m.delivery_count}</td>
                  <td>{m.dead_letter_reason}</td>
                  <td><code>{api.bodyPreview(m.body).slice(0, 200)}</code></td>
                </tr>
              ))}
            </tbody>
          </table>
          {messages.length === 0 && <p>No messages.</p>}
        </section>
      )}
    </main>
  );
}
```

Run: `cd app && npm install && npm run build`
Expected: `dist/` is produced and there are no TypeScript errors.

- [ ] **Step 2: Tauri crate**

`dispatched/Cargo.toml`. Change only `members` (leave `default-members` unchanged, so plain `cargo test` doesn't need the frontend build or webkit):
```toml
members = ["crates/dispatched-core", "crates/dispatched-cli", "app/src-tauri"]
```

`app/src-tauri/Cargo.toml`:
```toml
[package]
name = "dispatched-app"
description = "Dispatched desktop app (Tauri)"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
publish = false

[lib]
name = "dispatched_app_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
dispatched-core = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tauri = { version = "2", features = [] }
tokio = { version = "1", features = ["sync"] }

[features]
default = ["custom-protocol"]
custom-protocol = ["tauri/custom-protocol"]
```

`app/src-tauri/build.rs`:
```rust
fn main() {
    tauri_build::build()
}
```

`app/src-tauri/tauri.conf.json`. The updater is added in Task 19, once the signing key exists.
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Dispatched",
  "version": "0.0.1",
  "identifier": "io.horizon-tech.dispatched",
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      { "title": "Dispatched", "width": 1280, "height": 800, "minWidth": 900, "minHeight": 560 }
    ],
    "security": { "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; connect-src ipc: http://ipc.localhost" }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns", "icons/icon.ico"],
    "copyright": "Copyright (c) 2026 Horizon Tech d.o.o.",
    "category": "DeveloperTool",
    "shortDescription": "Azure Service Bus explorer, agent-ready.",
    "longDescription": "Dispatched — a fast, cross-platform explorer for Azure Service Bus.",
    "publisher": "Horizon Tech d.o.o.",
    "windows": { "nsis": { "installMode": "currentUser" } },
    "linux": { "deb": { "depends": ["libwebkit2gtk-4.1-0", "libgtk-3-0"] } }
  }
}
```

`app/src-tauri/capabilities/default.json`:
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capabilities for the Dispatched window.",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

`app/src-tauri/src/main.rs`:
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dispatched_app_lib::run()
}
```

`app/src-tauri/src/state.rs`:
```rust
use dispatched_core::Explorer;
use std::sync::Arc;
use tokio::sync::RwLock;

/// App-wide state: the active namespace explorer, if connected.
#[derive(Default)]
pub struct AppState {
    pub explorer: RwLock<Option<Arc<Explorer>>>,
}

impl AppState {
    pub async fn explorer(&self) -> Result<Arc<Explorer>, crate::error::CommandError> {
        self.explorer
            .read()
            .await
            .clone()
            .ok_or_else(|| crate::error::CommandError { kind: "not_connected", message: "connect to a namespace first".into() })
    }
}
```

`app/src-tauri/src/error.rs`:
```rust
use dispatched_core::Error;
use serde::Serialize;

/// Error shape returned to the frontend.
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        let kind = match &e {
            Error::Auth(_) => "auth",
            Error::NotSignedIn(_) => "not_signed_in",
            Error::Forbidden { .. } => "forbidden",
            Error::NotFound(_) => "not_found",
            Error::Throttled { .. } => "throttled",
            Error::Timeout(_) => "timeout",
            Error::Cancelled => "cancelled",
            Error::Unsupported { .. } => "unsupported",
            Error::Invalid(_) => "invalid",
            Error::Network(_) => "network",
            _ => "protocol",
        };
        Self { kind, message: e.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_maps_to_kind_and_message() {
        let e: CommandError = Error::Forbidden { needed: Some("Listen".into()), detail: "no".into() }.into();
        assert_eq!(e.kind, "forbidden");
        assert!(e.message.contains("needs Listen"));
    }
}
```

`app/src-tauri/src/commands/mod.rs`:
```rust
pub mod browse;
pub mod connection;
```

`app/src-tauri/src/commands/connection.rs`:
```rust
use crate::error::CommandError;
use crate::state::AppState;
use dispatched_core::{ConnectionString, Explorer, Namespace};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
pub struct ConnectionInfo {
    host: String,
    emulator: bool,
    auth: String,
}

#[tauri::command]
pub async fn connect_connection_string(
    state: State<'_, AppState>,
    connection_string: String,
) -> Result<ConnectionInfo, CommandError> {
    let ns = Namespace::from_connection_string(&ConnectionString::parse(&connection_string)?)?;
    let info = ConnectionInfo { host: ns.host().to_string(), emulator: ns.is_emulator(), auth: ns.tokens().describe() };
    let previous = state.explorer.write().await.replace(Arc::new(Explorer::new(ns)));
    if let Some(old) = previous {
        old.close().await;
    }
    Ok(info)
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(old) = state.explorer.write().await.take() {
        old.close().await;
    }
    Ok(())
}
```

`app/src-tauri/src/commands/browse.rs`:
```rust
use crate::error::CommandError;
use crate::state::AppState;
use dispatched_core::{EntityPath, PeekedMessage, QueueInfo, SubQueue};
use tauri::State;

#[tauri::command]
pub async fn list_queues(state: State<'_, AppState>) -> Result<Vec<QueueInfo>, CommandError> {
    Ok(state.explorer().await?.list_queues().await?)
}

#[tauri::command]
pub async fn peek(
    state: State<'_, AppState>,
    entity: String,
    sub_queue: SubQueue,
    from_sequence: i64,
    max: u32,
) -> Result<Vec<PeekedMessage>, CommandError> {
    let path = EntityPath::parse(&entity)?;
    Ok(state.explorer().await?.peek(&path, sub_queue, from_sequence, max.min(500)).await?)
}
```

`app/src-tauri/src/lib.rs`:
```rust
mod commands;
mod error;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::connection::connect_connection_string,
            commands::connection::disconnect,
            commands::browse::list_queues,
            commands::browse::peek,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dispatched");
}
```

`SubQueue` deserializes from `"active" | "dead-letter" | "transfer-dead-letter"`, because Task 11 applies `#[serde(rename_all = "kebab-case")]` to it.

- [ ] **Step 3: Generate placeholder icons**

These are a neutral monogram placeholder; real branding comes in M1 with the design system. Write the generator in the scratchpad, never in the repo:
```bash
python3 - <<'PY'
import struct, zlib
W = H = 1024
def px(x, y):
    inside = 192 <= x < 832 and 192 <= y < 832
    return (24, 28, 36, 255) if not inside else (232, 236, 242, 255)
raw = b"".join(b"\x00" + b"".join(bytes(px(x, y)) for x in range(W)) for y in range(H))
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
open("/private/tmp/claude-501/-Users-hamzaabdagic-Development-Horizon-Tech-doo/6f2510c1-4fa3-4366-b1b3-e33a97a8c7d7/scratchpad/dispatched-icon.png", "wb").write(png)
PY
cd app && npx tauri icon /private/tmp/claude-501/-Users-hamzaabdagic-Development-Horizon-Tech-doo/6f2510c1-4fa3-4366-b1b3-e33a97a8c7d7/scratchpad/dispatched-icon.png
rm /private/tmp/claude-501/-Users-hamzaabdagic-Development-Horizon-Tech-doo/6f2510c1-4fa3-4366-b1b3-e33a97a8c7d7/scratchpad/dispatched-icon.png
```
Expected: `app/src-tauri/icons/` contains `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns` and `icon.ico`. Delete any generated mobile icon folders (`android/`, `ios/`) the CLI adds; they're not needed.

- [ ] **Step 4: Build and test the app crate**

Run: `df -h /System/Volumes/Data` (stop if under 30 GB free), then `cargo test -p dispatched-app && cargo clippy -p dispatched-app -- -D warnings`
Expected: PASS (1 test), and clippy is clean.

- [ ] **Step 5: Manual smoke against the emulator**

Run: `testing/emulator/emulator.sh up`, then `cd app && npm run tauri dev`.
Expected:
- The window opens.
- Pasting the emulator connection string and pressing Connect lists `orders` and `sessions-q` with counts.
- "Peek" shows messages; "Peek DLQ" shows the `m0-gate` row with its reason. Seed it first by running the Task 16 test.

Use the `run` skill to take a screenshot for the gate report if you can. Then close the app and run `testing/emulator/emulator.sh down`.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Desktop app skeleton: connect, list queues, peek and peek DLQ"
```

---

### Task 19: CI, release workflow, updater, NOTICE and README

**Files:**
- Create: `dispatched/.github/workflows/ci.yml`, `dispatched/.github/workflows/release.yml`, `dispatched/NOTICE`, `dispatched/README.md`
- Modify: `app/src-tauri/Cargo.toml`, `app/src-tauri/src/lib.rs`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/capabilities/default.json`, `app/package.json` (updater + process plugins)

**Interfaces:**
- Consumes: the kit's reusable `tauri-release.yml` (Task 9) and `testing/emulator/emulator.sh` (Task 16).
- Produces:
  - CI jobs `rust` (3 OS), `app` (ubuntu) and `emulator` (ubuntu).
  - Release on tag `app-v*`.
  - The updater checks `https://github.com/Horizon-Tech-doo/dispatched/releases/latest/download/latest.json`.

- [ ] **Step 1: CI workflow**

`dispatched/.github/workflows/ci.yml`:
```yaml
name: CI
on:
  push: { branches: [main] }
  pull_request: { branches: [main] }
env:
  CARGO_TERM_COLOR: always
defaults:
  run:
    working-directory: dispatched
jobs:
  rust:
    strategy:
      fail-fast: false
      matrix: { os: [ubuntu-latest, macos-latest, windows-latest] }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
        with: { path: dispatched }
      - uses: actions/checkout@v4
        with: { repository: Horizon-Tech-doo/horizon-azure-kit, path: horizon-azure-kit }
      - uses: dtolnay/rust-toolchain@stable
        with: { components: "rustfmt, clippy" }
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: "dispatched -> target" }
      - if: matrix.os == 'ubuntu-latest'
        run: cargo fmt --all --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings
      - run: cargo test
  app:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { path: dispatched }
      - uses: actions/checkout@v4
        with: { repository: Horizon-Tech-doo/horizon-azure-kit, path: horizon-azure-kit }
      - run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy }
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: "dispatched -> target" }
      - run: npm ci && npm run build
        working-directory: dispatched/app
      - run: cargo clippy -p dispatched-app -- -D warnings && cargo test -p dispatched-app
  emulator:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { path: dispatched }
      - uses: actions/checkout@v4
        with: { repository: Horizon-Tech-doo/horizon-azure-kit, path: horizon-azure-kit }
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: "dispatched -> target" }
      - run: testing/emulator/emulator.sh up
      - run: DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 -- --nocapture
      - if: always()
        run: |
          (cd testing/emulator && docker compose --env-file .env logs --tail 200 emulator) || true
          testing/emulator/emulator.sh down
```

- [ ] **Step 2: Updater signing key (needs the user)**

Ask the user to run this themselves, because it prompts for a password:
```
! cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched/app && npx tauri signer generate -w ~/.tauri/dispatched.key
```
Tell them to keep the private key and password safe; they'll add them as GitHub secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when the repo is created (Task 20). Copy the printed **public key**. If the user defers this, skip Steps 3–4 and record "updater pending key" in the gate report.

- [ ] **Step 3: Wire the updater and process plugins**

Run: `cd app && npm install @tauri-apps/plugin-updater@^2 @tauri-apps/plugin-process@^2`

`app/src-tauri/Cargo.toml`. Add to `[dependencies]`:
```toml
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
```

`app/src-tauri/src/lib.rs`. Add the plugins to the builder chain, before `.manage(...)`:
```rust
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
```

`app/src-tauri/capabilities/default.json`. Change `permissions` to:
```json
  "permissions": ["core:default", "updater:default", "process:default"]
```

`app/src-tauri/tauri.conf.json`. Add `"createUpdaterArtifacts": true` inside `bundle`, and this top-level `plugins` object. Paste the public key from Step 2 exactly as printed:
```json
  "plugins": {
    "updater": {
      "endpoints": ["https://github.com/Horizon-Tech-doo/dispatched/releases/latest/download/latest.json"],
      "pubkey": "<the public key printed in Step 2>"
    }
  }
```
The update UI ("update available" prompt) is part of M1's shell work. M0 only wires the plugin so release artifacts are signed from the first release.

Run: `cd app && npm run build && cd .. && cargo clippy -p dispatched-app -- -D warnings`
Expected: clean.

- [ ] **Step 4: Release workflow**

`dispatched/.github/workflows/release.yml`:
```yaml
name: Release
on:
  workflow_dispatch:
  push:
    tags: ["app-v*"]
permissions:
  contents: write
jobs:
  desktop:
    uses: Horizon-Tech-doo/horizon-azure-kit/.github/workflows/tauri-release.yml@main
    with:
      app-name: Dispatched
      project-path: app
      cargo-workspace: "."
    secrets: inherit
```

- [ ] **Step 5: NOTICE and README**

`dispatched/NOTICE`:
```
Dispatched
Copyright (c) 2026 Horizon Tech d.o.o.

This product is licensed under the Apache License, Version 2.0.

Protocol details for Azure Service Bus over AMQP 1.0 (management request
shapes, CBS token types, message annotation keys) were cross-checked against
the `azservicebus` crate (https://crates.io/crates/azservicebus),
Copyright (c) minghuaw, MIT License, itself a port of the Azure SDK for .NET
(Copyright (c) Microsoft Corporation, MIT License).

AMQP 1.0 is provided by the fe2o3-amqp crates (MIT OR Apache-2.0).

Dispatched is not affiliated with, endorsed by, or sponsored by Microsoft
Corporation. "Microsoft", "Azure" and "Azure Service Bus" are trademarks of
Microsoft Corporation.
```

`dispatched/README.md`:
````markdown
# Dispatched

**A fast, cross-platform explorer for Azure Service Bus. Built for humans and agents.**

Dispatched is an open-source alternative to [ServiceBusExplorer](https://github.com/paolosalvatori/ServiceBusExplorer) that runs on Windows, macOS and Linux, signs in with Microsoft Entra ID, works with the Service Bus Emulator, and (soon) exposes an MCP server so agents can inspect and operate your messaging safely.

> **Status:** 🚧 M0 — spike and skeleton. Browse-only. Not ready for production use.

## Try it (CLI)

```bash
cargo install --path crates/dispatched-cli   # from a checkout with ../horizon-azure-kit beside it
dispatched login                              # browser sign-in (or --device-code)
dispatched namespaces
dispatched --namespace myns ls --topics
dispatched --namespace myns peek orders --dlq
```

With a connection string (SAS or emulator):

```bash
export DISPATCHED_CONNECTION_STRING='Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;'
dispatched peek orders
```

## Develop

- Clone `horizon-azure-kit` next to this repo (shared Entra / keychain / ARM code).
- `cargo test` runs the core and CLI tests.
- Emulator tests: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0`.
- Desktop: `cd app && npm install && npm run tauri dev`.

Design: [`docs/superpowers/specs/2026-10-13-dispatched-v1-design.md`](docs/superpowers/specs/2026-10-13-dispatched-v1-design.md).

## License

Apache-2.0. See [NOTICE](NOTICE). Not affiliated with Microsoft.
````

- [ ] **Step 6: Validate the workflow YAML and commit**

Run: `for f in .github/workflows/*.yml; do python3 -c "import yaml,sys; yaml.safe_load(open('$f'))" && echo "ok $f"; done`
Expected: `ok` for each file.

```bash
cargo fmt --all && git add -A && git commit -m "CI (rust/app/emulator), release via kit workflow, updater, NOTICE, README"
```

---

### Task 20: M0 gate — live verification, publish, cleanup

**Files:**
- Create: `dispatched/docs/live-verification-m0.md`

**Interfaces:**
- Consumes: everything above.
- Produces: the go/no-go decision for approach A′, recorded in `docs/live-verification-m0.md`.

- [ ] **Step 1: Ask the user for a real namespace**

Ask: "For the M0 gate I need one real Service Bus namespace you're happy to read from (peek only; nothing is modified, except that the seeding step sends and dead-letters 3 test messages, but only if you allow it). Which namespace, which queue, and may I seed test messages there?" Creating a namespace costs money and is outward-facing: only do it if the user explicitly asks, and then follow the `az` tenant rules in the user's CLAUDE.md (check `az account show` first).

- [ ] **Step 2: Live checks with Entra**

Run, with the user signing in in their browser:
```bash
cargo run -q -p dispatched -- login
cargo run -q -p dispatched -- namespaces
cargo run -q -p dispatched -- --namespace <ns> ls --topics
cargo run -q -p dispatched -- --namespace <ns> peek <queue>
cargo run -q -p dispatched -- --namespace <ns> peek <queue> --dlq
```
Expected: the namespace is listed, queues have counts, peeks return messages (or "(no messages)"), and DLQ rows show REASON. If the user lacks the "Azure Service Bus Data Owner" role, `ls` fails with `access denied (needs Manage,EntityRead)`. Record that, because it confirms the least-privilege message works. Peek then needs the Data Receiver role.

- [ ] **Step 3: Live checks with SAS**

Ask the user to paste a connection string as a one-off environment variable; never write it to a file:
```
! export DISPATCHED_CONNECTION_STRING='<namespace connection string>'
```
Then run `ls`, `peek <queue>` and `peek <queue> --dlq` without `--namespace`.
Expected: the same results as Step 2.

- [ ] **Step 4: Write the gate report**

`docs/live-verification-m0.md`:
```markdown
# M0 live verification — <date>

| Check | Emulator | Cloud + Entra | Cloud + SAS |
|---|---|---|---|
| Connect (AMQP + CBS) | | | |
| List queues (ATOM) + counts | | | |
| Peek active | | | |
| Peek DLQ with reason | | | |
| Empty / past-end peek is empty | | n/a | n/a |
| Desktop window: connect → list → peek | | n/a | |

Emulator counts (GATE-NOTE line): <paste>
Notes / deviations: <management-link routing, emulator listing support, lock-token byte order not exercised yet, mssql on Apple Silicon, ...>

## Decision
Approach A′ (own Service Bus layer on fe2o3-amqp + ATOM + ARM): **GO / NO-GO**, because <one paragraph>.
Follow-ups for M1: <list>
```
Fill every cell with ✅, ❌ (plus the error text), or "not run" (plus the reason).

- [ ] **Step 5: Ask before publishing**

Ask the user: "Create public GitHub repos `Horizon-Tech-doo/horizon-azure-kit` and `Horizon-Tech-doo/dispatched`, and push `main` for both? The kit must be public (or the org must allow cross-repo workflow access) for Dispatched's CI and release to check it out." Only on an explicit yes:
```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit && gh repo create Horizon-Tech-doo/horizon-azure-kit --public --source . --push
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && gh repo create Horizon-Tech-doo/dispatched --public --source . --push
```
Then ask the user to add the updater secrets to the dispatched repo:
```
! gh secret set TAURI_SIGNING_PRIVATE_KEY -R Horizon-Tech-doo/dispatched < ~/.tauri/dispatched.key
! gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD -R Horizon-Tech-doo/dispatched
```
Watch CI with `gh run watch -R Horizon-Tech-doo/dispatched` and `gh run watch -R Horizon-Tech-doo/horizon-azure-kit`. Expected: all jobs are green, including `emulator`. Record CI results (especially the emulator job on Linux) in the gate report. Do **not** tag a release in M0 unless the user asks.

- [ ] **Step 6: Clean up**

Run:
```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && testing/emulator/emulator.sh down || true
docker image rm mcr.microsoft.com/azure-messaging/servicebus-emulator:2.0.1 mcr.microsoft.com/mssql/server:2022-latest || true
df -h /System/Volumes/Data
```
Leave both repos' `target/` folders in place: they're the real checkouts' build dirs and are reused. Confirm that no scratch files remain in the scratchpad.

- [ ] **Step 7: Commit the gate report**

```bash
git add docs/live-verification-m0.md && git commit -m "M0 gate report" && (git push || true)
```
Push only if Step 5 created the remote.
