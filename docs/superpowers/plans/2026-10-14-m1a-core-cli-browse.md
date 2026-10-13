# M1a — Core and CLI Browse: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the CLI a complete browse tool, using engine features the desktop UI (M1c) will reuse. It adds:
- saved profiles and multi-tenant Entra sign-in
- long sessions that refresh their tokens
- AMQP over WebSockets, with proxy support
- every peek view: active, scheduled, deferred, DLQ, transfer DLQ
- property and body filters
- export
- approximate counts where the service doesn't report any
- entity-scoped keys

**Architecture:**
- Every behaviour lives in `dispatched-core` as pure, testable modules: `filter/`, `export`, `profiles/`, `diagnostics`, plus extensions to `messaging/` and `explorer`.
- The CLI only parses input and renders output.
- The shared kit gets a v0.2.0 fix for multi-tenant refresh safety.

**Tech Stack:** M0 stack, plus `rusqlite` 0.40 (bundled SQLite), `regex` 1, `serde_json_path` 0.7 (RFC 9535 JSONPath), and `fe2o3-amqp-ws` 0.18.

**Spec:** `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` (§2, §3, §4.1, §4.2, §4.4, §7, §8, §9). The M0 plan `docs/superpowers/plans/2026-10-13-m0-kit-spike-skeleton.md` describes the existing code. `docs/live-verification-m0.md` lists the M0 follow-ups this plan picks up.

## Global Constraints

- **Repo locations:**
  - Dispatched: `/Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched`
  - Kit: `/Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit`
  - Both repos are public. The default branch is `main`. Work on a branch named `m1a` in each repo, and never commit directly to `main`.
- **Disk hygiene (hard rule):**
  - Build only in each repo's own `target/`. Never set `CARGO_TARGET_DIR`. Never copy repos to build them.
  - Run `df -h /System/Volumes/Data` before heavy builds, and stop if free space is under 30 GB.
  - Don't delete `target/`.
  - Emulator: always run `testing/emulator/emulator.sh down` after use. The controller removes images at the end.
- **Edition and toolchain:** Rust 2024 edition, MSRV 1.88. The workspace uses resolver 3, which picks dependency versions compatible with the MSRV.
- **TLS:** rustls only, and never `native-tls`.
  - `fe2o3-amqp` uses `features = ["rustls"]`.
  - `fe2o3-amqp-ws` uses `features = ["rustls-tls-webpki-roots"]`.
- **New dependencies:** use exactly these major/minor lines.
  - `rusqlite = { version = "0.40", features = ["bundled"] }`
  - `regex = "1"`
  - `serde_json_path = "0.7"`
  - `fe2o3-amqp-ws = { version = "0.18", features = ["rustls-tls-webpki-roots"] }`
  - The existing manifest bound `tokio-util = ">=0.7, <0.7.20"` stays.
- **Kit:**
  - This plan releases kit `v0.2.0`: bump `version` in the kit's root `Cargo.toml`, and Dispatched's workspace dependency becomes `version = "0.2"`.
  - Dispatched's CI and release pins move from `v0.1.0` to `v0.2.0` in Task 15. The tag is created only after the user approves it.
- **Secrets:**
  - Connection strings, keys, and tokens never appear in errors, logs, JSON output, the SQLite database, or command-line arguments the user is told to use.
  - Profile secrets live in the OS keychain under service `io.horizon-tech.dispatched`, with key `dispatched:profile:{profile_id}:connection-string`.
- **Error messages say what to do.** This follows spec §8 and the existing `Error` variants. Never change the existing `Error` `Display` texts that M0 tests pin.
- **Message state:**
  - The annotation `x-opt-message-state` is an int: `0` active, `1` deferred, `2` scheduled.
  - If the annotation is missing, the message is active.
- **Peek scanning:**
  - Each request asks for at most `MAX_PER_REQUEST = 250` messages.
  - The default scan limit is `10_000` messages per peek call.
  - Scanning stops on an empty batch, when the sequence number fails to advance, or when a sequence number is `-1`.
- **Token refresh:** re-put a CBS token when it has less than **5 minutes** left. Cache at most **64** management links per connection, evicting the least recently used.
- **WebSockets:**
  - URL: `wss://{host}/$servicebus/websocket/`.
  - Configuring a proxy forces WebSockets.
  - The emulator supports neither WebSockets nor a proxy, and requesting either on the emulator returns `Error::Unsupported`.
- **Proxy source:** the profile setting first, else environment `HTTPS_PROXY` or `https_proxy`. Only `http://host:port` proxies (with optional `user:pass@` for Basic auth) are supported.
- **Database:**
  - Path: `{config_dir}/dispatched/dispatched.db`, which the `DISPATCHED_CONFIG_DIR` override also covers.
  - Schema `user_version = 1`.
  - On first open, the M0 `cli-state.json` is migrated into the database and then deleted.
- **Export schema id:** `"dispatched.message/v1"`.
- **Desktop app (`app/src-tauri`):** only keep it compiling when core signatures change. Feature work is M1c.
- **Outward-facing actions need the user's explicit yes, every time:** pushing, tagging, creating Azure resources.
- **Out of scope for M1a:**
  - the desktop UI (M1b/M1c)
  - sovereign clouds
  - sessions, send, receive, settle (M2)
  - entity create/update/delete (M3)
  - MCP (M4)

## Review Focus

1. **Filter expressions with odd input.** Covers empty input, unknown `sys.` fields, string-vs-number comparison, `''` escaping, unbalanced parentheses, trailing tokens, Unicode, and non-UTF-8 bodies.
   - **Expected:** a clear `Error::Invalid` that names the column for syntax errors. Type mismatches evaluate to "no match". Nothing panics.
   - **Tests:** Task 3 `parse_errors_name_the_column`, `type_mismatch_is_no_match`, `quotes_escape`. Task 4 `body_filters_ignore_binary_bodies`.
2. **Rare matches in a big entity.** A scheduled-message view over 100k active messages.
   - **Expected:** peek stops at the scan limit and returns a continuation sequence, never loops forever, and never re-reads the same range when sequence numbers are missing.
   - **Tests:** Task 5 `scan_stops_at_limit_with_continuation`, `scan_stops_when_sequence_does_not_advance`.
3. **Sessions longer than one hour.**
   - **Expected:** the next peek after token expiry works without the user doing anything.
   - **Tests:** Task 7 `ledger_refreshes_within_margin`, `explorer_retries_once_after_auth_error`.
4. **Profiles.**
   - **Expected:** names are unique case-insensitively, secrets never land in the DB file, deleting a profile deletes its keychain secret, an M0 `cli-state.json` migrates once, and a corrupt DB gives a clear error.
   - **Tests:** Task 10 `names_are_case_insensitive_unique`, `secret_never_written_to_db`, `delete_removes_secret`, `migrates_cli_state_once`.
5. **Corporate proxy failures.** A proxy returns 407, 403, a garbage status line, or closes early.
   - **Expected:** `Error::Network` naming the proxy status, with no hang.
   - **Tests:** Task 8 `proxy_407_is_named`, `proxy_garbage_is_network_error`, `proxy_timeout`.

---

## File Structure

### horizon-azure-kit (Task 1)
- Modify: `crates/horizon-azure-kit/src/auth/session.rs` (multi-tenant-safe rotation, ordering fixes)
- Modify: `Cargo.toml` (version 0.2.0), `README.md` (changelog section)

### dispatched
```
crates/dispatched-core/src/
├── lib.rs                       + filter, export, profiles, diagnostics modules / re-exports
├── namespace.rs                 + PeekView, Transport, ProxyConfig on Namespace
├── tokens.rs                    + SasKeySource::with_ttl
├── explorer.rs                  peek(view, PeekOptions) -> PeekPage; reconnect-once; approximate counts; known_entities
├── filter/mod.rs                MessageFilter (property + body)
├── filter/lexer.rs              tokens for the property-filter language
├── filter/expr.rs               parser + evaluator (PropertyFilter)
├── filter/body.rs               BodyFilter: contains / regex / JSONPath
├── export.rs                    ExportedMessage v1, JSON / NDJSON writers
├── diagnostics.rs               ConnectionReport (auth path, management, messaging)
├── profiles/mod.rs              Profile model + resolve to Namespace
├── profiles/store.rs            SQLite ProfileStore (+ sign-ins, settings, migration)
├── mgmt/models.rs               EntityCounts.approximate
├── mgmt/mod.rs                  ManagementClient proxy support
└── messaging/
    ├── message.rs               MessageState on PeekedMessage
    ├── scan.rs                  PeekSource trait + scan_messages (view/filter/limit)
    ├── tokens_ledger.rs         TokenLedger + LinkLru (pure)
    ├── transport.rs             open AMQP/TLS or WebSockets(+proxy CONNECT) stream
    ├── connection.rs            uses transport, ledger, LRU
    └── testing.rs               + send_scheduled, defer_next, timeouts
crates/dispatched-core/tests/emulator_m1.rs   views/filters/export/counts against the emulator
crates/dispatched-cli/src/
├── main.rs                      new global flags; dispatch to command modules
├── commands/{mod.rs, auth.rs, profile.rs, browse.rs}   split by area (main.rs is getting large)
├── state.rs                     replaced by profile DB (sign-ins, current sign-in)
├── resolve.rs                   precedence: --connection-string > --namespace > --profile > default profile
└── output.rs                    sanitize LRM/RLM/ALM/zero-width
app/src-tauri/src/commands/browse.rs        compile with new peek signature
testing/emulator/docker-compose.arm64.yml   pin azure-sql-edge:1.0.7
.github/workflows/ci.yml                     run emulator_m0 + emulator_m1
```

---
### Task 1: Kit v0.2.0 — multi-tenant-safe token refresh

**Files:**
- Modify: `horizon-azure-kit/crates/horizon-azure-kit/src/auth/session.rs` (`token_for`, `finish_login`, new `generation()` helper, tests)
- Modify: `horizon-azure-kit/Cargo.toml` (`version = "0.2.0"`), `horizon-azure-kit/README.md` (add a "Changes" section)

**Interfaces:**
- The public API is unchanged: `EntraSession::{token_for, login_device_code, login_browser, logout, is_signed_in}`.
- Behaviour changes:
  - (a) A refresh token rotated by a refresh against a non-home tenant is **not** persisted. Only home-tenant rotations replace the stored token.
  - (b) The generation is snapshotted **before** `load`, which closes the logout-vs-load window.
  - (c) `finish_login` bumps the sign-in's generation, under the state lock, before it stores the new refresh token. This stops an in-flight failing refresh from deleting a fresh login, and clears that sign-in's cached access tokens.

- [ ] **Step 1: Branch**

```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/horizon-azure-kit && git checkout -b m1a
```

- [ ] **Step 2: Write the failing tests** (append to the `tests` module in `session.rs`)

```rust
    #[tokio::test]
    async fn cross_tenant_refresh_does_not_replace_home_refresh_token() {
        let mut server = mockito::Server::new_async().await;
        let _login = mock_device_login(&mut server).await;
        let _guest = server
            .mock("POST", "/tenant-2/oauth2/v2.0/token")
            .with_status(200)
            .with_body(r#"{"access_token":"guest-at","refresh_token":"rt-guest","expires_in":3600}"#)
            .create_async()
            .await;
        let store = Arc::new(MemoryStore::new());
        let s = session(&server, store.clone());
        let sign_in = s.login_device_code("organizations", |_| {}).await.unwrap();
        let t = s.token_for(&sign_in, Some("tenant-2"), "scope").await.unwrap();
        assert_eq!(t.token.expose_secret(), "guest-at");
        let stored = store.get("test:entra:oid-1").unwrap().unwrap();
        assert!(stored.expose_secret().contains("rt-1"), "home refresh token must be kept");
        assert!(!stored.expose_secret().contains("rt-guest"));
    }

    #[tokio::test]
    async fn fresh_login_survives_an_in_flight_revocation() {
        let mut server = mockito::Server::new_async().await;
        let _login = mock_device_login(&mut server).await;
        let _slow_revoked = server
            .mock("POST", "/tenant-1/oauth2/v2.0/token")
            .with_status(400)
            .with_body_from_request(|_| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                br#"{"error":"invalid_grant","error_description":"AADSTS50173: revoked"}"#.to_vec()
            })
            .create_async()
            .await;
        let store = Arc::new(MemoryStore::new());
        let s = Arc::new(session(&server, store.clone()));
        let sign_in = s.login_device_code("organizations", |_| {}).await.unwrap();
        let s2 = s.clone();
        let si2 = sign_in.clone();
        let refresh = tokio::spawn(async move { s2.token_for(&si2, None, "scope").await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // The user signs in again while the doomed refresh is still in flight.
        s.login_device_code("organizations", |_| {}).await.unwrap();
        assert!(refresh.await.unwrap().is_err());
        assert!(s.is_signed_in(&sign_in).unwrap(), "fresh login must not be deleted");
    }
```

Run: `cargo test -p horizon-azure-kit auth::session`
Expected: both new tests FAIL. The first finds `rt-guest` stored. The second finds `is_signed_in` false.

- [ ] **Step 3: Implement**

In `session.rs`, add a helper on `EntraSession`:

```rust
    fn generation(&self, sign_in_id: &str) -> u64 {
        self.state
            .lock()
            .expect("session state poisoned")
            .generations
            .get(sign_in_id)
            .copied()
            .unwrap_or(0)
    }
```

In `token_for`:
- **Snapshot first.** The first line becomes `let generation = self.generation(&sign_in.id);`, followed by `let stored = self.load(sign_in)?;`.
- **Cache check.** The block that returned `generation` now only checks the cache:

```rust
        {
            let state = self.state.lock().expect("session state poisoned");
            if let Some(hit) = state.cache.get(&key)
                && hit.expires_on - OffsetDateTime::now_utc() > REFRESH_MARGIN
            {
                return Ok(hit.clone());
            }
        }
```

- **Persist only home rotations.** Change the rotation persistence guard to `if home && let Some(rotated) = resp.refresh_token.as_ref().filter(|rt| **rt != stored.refresh_token) {`. Keep the body.

In `finish_login`, replace the single `self.secrets.put(...)` with:

```rust
        {
            let mut state = self.state.lock().expect("session state poisoned");
            *state.generations.entry(id.clone()).or_insert(0) += 1;
            state.cache.retain(|(cached_id, _, _), _| cached_id != &id);
            self.secrets.put(&self.key(&id), &SecretString::from(json))?;
        }
```

Adjust the existing variable names so `id` is the sign-in id string at that point.

- [ ] **Step 4: Run tests and lint**

Run: `cargo test -p horizon-azure-kit && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all --check`
Expected: all tests PASS (71 passed, 1 ignored) and the code is clean.

- [ ] **Step 5: Version and changelog**

In `horizon-azure-kit/Cargo.toml`, set `version = "0.2.0"` under `[workspace.package]`. Append to `README.md`:

```markdown
## Changes

- **0.2.0**
  - `EntraSession::token_for` no longer replaces the stored (home-tenant) refresh token after refreshing against another tenant.
  - Signing in again wins over a refresh that is still in flight.
- **0.1.0**: initial release (Dispatched M0).
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Kit 0.2.0: multi-tenant-safe refresh rotation; login wins over in-flight refresh"
```

Do NOT tag or push. Task 15 handles publishing.

---

### Task 2: Message state on peeked messages

**Files:**
- Modify: `dispatched/crates/dispatched-core/src/messaging/message.rs`
- Modify: `dispatched/crates/dispatched-core/src/messaging/mod.rs`, `src/lib.rs` (re-export `MessageState`)
- Modify: `dispatched/Cargo.toml` (kit dependency `version = "0.2"`)

**Interfaces:**
- Produces:
  - `messaging::MessageState::{Active, Deferred, Scheduled}`, serialised lowercase, with `as_str(self) -> &'static str`.
  - New field `PeekedMessage.state: MessageState`, placed directly after `sequence_number`.
  - A test-only constructor `#[cfg(test)] pub(crate) fn test_message(seq: i64) -> PeekedMessage`. All optional fields are `None`, the state is `Active`, the body is `MessageBody::Text(String::new())`, and the delivery count is 0.

- [ ] **Step 1: Branch and bump the kit dependency**

```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && git checkout m1a   # created with the plan commit; branched from main
```

In the root `Cargo.toml` `[workspace.dependencies]`, change the kit line to `horizon-azure-kit = { version = "0.2", path = "../horizon-azure-kit/crates/horizon-azure-kit" }`. The kit checkout must be on branch `m1a` from Task 1.

- [ ] **Step 2: Write failing tests** (append to `message.rs` tests)

```rust
    fn with_state(state: Option<i32>) -> Message<Body<Value>> {
        let mut ann = MessageAnnotations::builder().insert("x-opt-sequence-number", 5i64);
        if let Some(s) = state {
            ann = ann.insert("x-opt-message-state", s);
        }
        Message::builder()
            .message_annotations(ann.build())
            .data(b"x".to_vec())
            .build()
            .map_body(Body::from)
    }

    #[test]
    fn message_state_comes_from_annotation() {
        assert_eq!(decode_message(with_state(None)).state, MessageState::Active);
        assert_eq!(decode_message(with_state(Some(0))).state, MessageState::Active);
        assert_eq!(decode_message(with_state(Some(1))).state, MessageState::Deferred);
        assert_eq!(decode_message(with_state(Some(2))).state, MessageState::Scheduled);
        assert_eq!(decode_message(with_state(Some(9))).state, MessageState::Active);
    }

    #[test]
    fn message_state_serialises_lowercase() {
        let json = serde_json::to_value(decode_message(with_state(Some(2)))).unwrap();
        assert_eq!(json["state"], "scheduled");
    }
```

If `Message::map_body` or `Body::from` doesn't exist in fe2o3 0.18, build the message the way the existing `base()` test helper in this file does: a struct literal with `body: Body::Data(...)`. Keep the test's intent unchanged.

Run: `cargo test -p dispatched-core messaging::message`
Expected: FAIL to compile (`MessageState` is not defined).

- [ ] **Step 3: Implement**

Add the type near `MessageBody` in `message.rs`:

```rust
/// Broker-side state of a message (`x-opt-message-state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageState {
    /// Available for receipt.
    Active,
    /// Set aside by a receiver; only receivable by sequence number.
    Deferred,
    /// Not yet enqueued (scheduled for later).
    Scheduled,
}

impl MessageState {
    /// Lowercase name, as used in filters and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            MessageState::Active => "active",
            MessageState::Deferred => "deferred",
            MessageState::Scheduled => "scheduled",
        }
    }
}

fn value_as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Byte(n) => Some(i64::from(*n)),
        Value::Short(n) => Some(i64::from(*n)),
        Value::Int(n) => Some(i64::from(*n)),
        Value::Long(n) => Some(*n),
        Value::Ubyte(n) => Some(i64::from(*n)),
        Value::Ushort(n) => Some(i64::from(*n)),
        Value::Uint(n) => Some(i64::from(*n)),
        Value::Ulong(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}
```

In `PeekedMessage`, add after `sequence_number`:

```rust
    /// Active, deferred or scheduled.
    pub state: MessageState,
```

In `decode_message`, compute:

```rust
    let state = match annotation(&m, "x-opt-message-state").and_then(value_as_i64) {
        Some(1) => MessageState::Deferred,
        Some(2) => MessageState::Scheduled,
        _ => MessageState::Active,
    };
```

and set `state,` in the struct literal after `sequence_number`.

Add the test constructor at the bottom of `message.rs`, outside the tests module:

```rust
#[cfg(test)]
pub(crate) fn test_message(seq: i64) -> PeekedMessage {
    PeekedMessage {
        sequence_number: seq,
        state: MessageState::Active,
        message_id: None,
        correlation_id: None,
        session_id: None,
        subject: None,
        content_type: None,
        to: None,
        reply_to: None,
        enqueued_time: None,
        scheduled_enqueue_time: None,
        locked_until: None,
        delivery_count: 0,
        time_to_live_ms: None,
        dead_letter_reason: None,
        dead_letter_description: None,
        dead_letter_source: None,
        application_properties: BTreeMap::new(),
        body: MessageBody::Text(String::new()),
    }
}
```

Re-export the type:
- `messaging/mod.rs`: `pub use message::{MessageBody, MessageState, PeekedMessage, PropValue, decode_bytes, decode_message};`
- `lib.rs`: `pub use messaging::{MessageBody, MessageState, PeekedMessage, PropValue};`

The desktop TypeScript type `PeekedMessage` in `app/src/api.ts` gains `state: "active" | "deferred" | "scheduled";`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dispatched-core && cargo clippy --all-targets -- -D warnings && (cd app && npm run build)`
Expected: PASS and clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: message state (active/deferred/scheduled) on peeked messages; kit 0.2"
```

---

### Task 3: Property filter language

**Files:**
- Create: `crates/dispatched-core/src/filter/mod.rs` (temporary: `mod lexer; mod expr; pub use expr::PropertyFilter;`)
- Create: `crates/dispatched-core/src/filter/lexer.rs`, `crates/dispatched-core/src/filter/expr.rs`
- Modify: `crates/dispatched-core/src/lib.rs` (`pub mod filter;`)

**Interfaces:**
- Produces `filter::PropertyFilter`:
  - `parse(src: &str) -> Result<Self>`
  - `matches(&self, msg: &PeekedMessage) -> bool`
  - `source(&self) -> &str`
- Language. Keywords are case-insensitive. Comparisons with a type mismatch or a null value evaluate to false.
  - `expr := or ; or := and (OR and)* ; and := not (AND not)* ; not := NOT not | primary`
  - `primary := '(' expr ')' | field cmp_op literal | field IS [NOT] NULL | field [NOT] LIKE 'pattern' | field [NOT] IN '(' literal (',' literal)* ')'`
  - `cmp_op := = | != | <> | < | <= | > | >=`
  - `literal := 'string' ('' escapes ') | number | TRUE | FALSE`
  - `field := sys.<Name> | user.<name> | <name>`. A bare name is a user (application) property.
  - System names are case-insensitive: MessageId, CorrelationId, SessionId, Subject (alias Label), ContentType, To, ReplyTo, SequenceNumber, EnqueuedTime, ScheduledEnqueueTime, DeliveryCount, DeadLetterReason, DeadLetterErrorDescription, DeadLetterSource, State, Size.
  - LIKE uses `%` and `_` and is case-insensitive.
  - Time fields compare against RFC 3339 string literals.
  - A string property compares numerically with a number literal when it parses as a number.

- [ ] **Step 1: Write the lexer with tests**

`filter/lexer.rs`:
```rust
//! Tokens for the property-filter language.

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub tok: Tok,
    /// 1-based column of the token's first character.
    pub col: usize,
}

pub(crate) fn err(col: usize, msg: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("filter: {msg} at column {col}"))
}

pub(crate) fn lex(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let tok = match c {
            '(' => { i += 1; Tok::LParen }
            ')' => { i += 1; Tok::RParen }
            ',' => { i += 1; Tok::Comma }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(err(col, "unterminated string")),
                        Some('\'') if chars.get(i + 1) == Some(&'\'') => { s.push('\''); i += 2; }
                        Some('\'') => { i += 1; break; }
                        Some(ch) => { s.push(*ch); i += 1; }
                    }
                }
                Tok::Str(s)
            }
            _ if two == "!=" || two == "<>" || two == "<=" || two == ">=" => {
                i += 2;
                Tok::Op(match two.as_str() { "!=" | "<>" => "!=", "<=" => "<=", _ => ">=" })
            }
            '=' => { i += 1; Tok::Op("=") }
            '<' => { i += 1; Tok::Op("<") }
            '>' => { i += 1; Tok::Op(">") }
            _ if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let start = i;
                i += 1;
                while chars.get(i).is_some_and(|d| d.is_ascii_digit() || *d == '.' || *d == 'e' || *d == 'E') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                Tok::Num(text.parse().map_err(|_| err(col, format!("invalid number `{text}`")))?)
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while chars.get(i).is_some_and(|d| d.is_alphanumeric() || matches!(d, '_' | '.' | '-')) {
                    i += 1;
                }
                Tok::Ident(chars[start..i].iter().collect())
            }
            other => return Err(err(col, format!("unexpected character `{other}`"))),
        };
        out.push(Token { tok, col });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn lexes_operators_strings_numbers() {
        assert_eq!(
            toks("sys.DeliveryCount >= 3 AND name <> 'it''s'"),
            vec![
                Tok::Ident("sys.DeliveryCount".into()),
                Tok::Op(">="),
                Tok::Num(3.0),
                Tok::Ident("AND".into()),
                Tok::Ident("name".into()),
                Tok::Op("!="),
                Tok::Str("it's".into()),
            ]
        );
        assert_eq!(toks("x = -2.5"), vec![Tok::Ident("x".into()), Tok::Op("="), Tok::Num(-2.5)]);
    }

    #[test]
    fn unterminated_string_names_column() {
        let e = lex("a = 'oops").unwrap_err().to_string();
        assert!(e.contains("unterminated string at column 5"), "{e}");
    }

    #[test]
    fn unexpected_character_is_an_error() {
        assert!(lex("a = #").unwrap_err().to_string().contains("column 5"));
    }
}
```

Run: `cargo test -p dispatched-core filter::lexer` once `filter/mod.rs` declares `mod lexer;`.
Expected: PASS, 3 tests.

- [ ] **Step 2: Write the parser and evaluator tests first** (bottom of `filter/expr.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{MessageBody, MessageState, PropValue};
    use crate::messaging::test_message;
    use time::OffsetDateTime;

    fn msg() -> PeekedMessage {
        let mut m = test_message(42);
        m.subject = Some("order.created".into());
        m.message_id = Some("m-1".into());
        m.delivery_count = 3;
        m.state = MessageState::Scheduled;
        m.enqueued_time = Some(OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap());
        m.application_properties.insert("tenant".into(), PropValue::Int(7));
        m.application_properties.insert("city".into(), PropValue::String("Pisa".into()));
        m.application_properties.insert("amount".into(), PropValue::String("12.5".into()));
        m.body = MessageBody::Text("hello".into());
        m
    }

    fn eval(src: &str) -> bool {
        PropertyFilter::parse(src).unwrap().matches(&msg())
    }

    #[test]
    fn system_and_user_properties() {
        assert!(eval("sys.Subject = 'order.created'"));
        assert!(eval("sys.label = 'order.created'"));
        assert!(eval("sys.SequenceNumber = 42 AND tenant = 7"));
        assert!(eval("user.city = 'Pisa' AND NOT city = 'Rome'"));
        assert!(eval("sys.DeliveryCount >= 3 AND sys.DeliveryCount < 4"));
        assert!(eval("sys.State = 'scheduled'"));
        assert!(eval("sys.Size = 5"));
        assert!(!eval("tenant = 8"));
    }

    #[test]
    fn like_in_null_and_precedence() {
        assert!(eval("sys.Subject LIKE 'ORDER.%'"));
        assert!(eval("city LIKE 'P_sa'"));
        assert!(eval("city NOT LIKE 'R%'"));
        assert!(eval("city IN ('Rome', 'Pisa')"));
        assert!(eval("city NOT IN ('Rome')"));
        assert!(eval("missing IS NULL AND city IS NOT NULL"));
        assert!(eval("tenant = 1 OR tenant = 7 AND city = 'Pisa'"));
        assert!(!eval("(tenant = 1 OR tenant = 7) AND city = 'Rome'"));
    }

    #[test]
    fn numeric_strings_and_times() {
        assert!(eval("amount > 10"));
        assert!(eval("sys.EnqueuedTime > '2023-01-01T00:00:00Z'"));
        assert!(!eval("sys.EnqueuedTime > '2024-01-01T00:00:00Z'"));
    }

    #[test]
    fn type_mismatch_is_no_match() {
        assert!(!eval("city > 5"));
        assert!(!eval("tenant = 'seven'"));
        assert!(!eval("sys.EnqueuedTime = 'not a time'"));
        assert!(!eval("missing = 1"));
    }

    #[test]
    fn quotes_escape() {
        let mut m = msg();
        m.application_properties.insert("q".into(), PropValue::String("it's".into()));
        assert!(PropertyFilter::parse("q = 'it''s'").unwrap().matches(&m));
    }

    #[test]
    fn parse_errors_name_the_column() {
        for (src, needle) in [
            ("", "empty filter"),
            ("tenant =", "expected a value at column 9"),
            ("(tenant = 7", "expected `)` at column 12"),
            ("tenant = 7 tenant", "unexpected `tenant` at column 12"),
            ("sys.Nope = 1", "unknown system property `sys.Nope`"),
            ("tenant LIKE 5", "LIKE needs a 'string' pattern"),
        ] {
            let e = PropertyFilter::parse(src).unwrap_err().to_string();
            assert!(e.contains(needle), "{src:?} -> {e}");
        }
    }

    #[test]
    fn unicode_values_work() {
        let mut m = msg();
        m.application_properties.insert("name".into(), PropValue::String("Šejla".into()));
        assert!(PropertyFilter::parse("name LIKE 'še%'").unwrap().matches(&m));
    }
}
```

- [ ] **Step 3: Implement `filter/expr.rs` above the tests**

```rust
//! Parser and evaluator for property filters, e.g.
//! `sys.DeliveryCount > 3 AND city = 'Pisa'`.

use super::lexer::{Tok, Token, err, lex};
use crate::messaging::{PeekedMessage, PropValue};
use crate::Result;
use base64::Engine;
use std::cmp::Ordering;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SysField {
    MessageId, CorrelationId, SessionId, Subject, ContentType, To, ReplyTo, SequenceNumber,
    EnqueuedTime, ScheduledEnqueueTime, DeliveryCount, DeadLetterReason, DeadLetterErrorDescription,
    DeadLetterSource, State, Size,
}

const SYS_NAMES: &[(&str, SysField)] = &[
    ("messageid", SysField::MessageId), ("correlationid", SysField::CorrelationId),
    ("sessionid", SysField::SessionId), ("subject", SysField::Subject), ("label", SysField::Subject),
    ("contenttype", SysField::ContentType), ("to", SysField::To), ("replyto", SysField::ReplyTo),
    ("sequencenumber", SysField::SequenceNumber), ("enqueuedtime", SysField::EnqueuedTime),
    ("scheduledenqueuetime", SysField::ScheduledEnqueueTime), ("deliverycount", SysField::DeliveryCount),
    ("deadletterreason", SysField::DeadLetterReason),
    ("deadlettererrordescription", SysField::DeadLetterErrorDescription),
    ("deadlettersource", SysField::DeadLetterSource), ("state", SysField::State), ("size", SysField::Size),
];

#[derive(Debug, Clone, PartialEq)]
enum Field { Sys(SysField), User(String) }

#[derive(Debug, Clone, PartialEq)]
enum Lit { Str(String), Num(f64), Bool(bool) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge }

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cmp(Field, CmpOp, Lit),
    IsNull(Field, bool),
    Like(Field, String, bool),
    In(Field, Vec<Lit>, bool),
}

/// A parsed property filter.
#[derive(Debug, Clone)]
pub struct PropertyFilter {
    expr: Expr,
    source: String,
}

impl PropertyFilter {
    /// Parse `src`. Errors name the column of the problem.
    pub fn parse(src: &str) -> Result<Self> {
        let tokens = lex(src)?;
        if tokens.is_empty() {
            return Err(err(1, "empty filter"));
        }
        let mut p = Parser { tokens, pos: 0, end_col: src.chars().count() + 1 };
        let expr = p.or()?;
        if let Some(t) = p.peek() {
            return Err(err(t.col, format!("unexpected `{}`", describe(&t.tok))));
        }
        Ok(Self { expr, source: src.to_string() })
    }

    /// The original filter text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether `msg` satisfies the filter.
    pub fn matches(&self, msg: &PeekedMessage) -> bool {
        eval(&self.expr, msg)
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => s.clone(),
        Tok::Str(s) => format!("'{s}'"),
        Tok::Num(n) => n.to_string(),
        Tok::Op(o) => (*o).to_string(),
        Tok::LParen => "(".into(),
        Tok::RParen => ")".into(),
        Tok::Comma => ",".into(),
    }
}

struct Parser { tokens: Vec<Token>, pos: usize, end_col: usize }

impl Parser {
    fn peek(&self) -> Option<&Token> { self.tokens.get(self.pos) }
    fn col(&self) -> usize { self.peek().map(|t| t.col).unwrap_or(self.end_col) }
    fn next(&mut self) -> Option<Token> { let t = self.tokens.get(self.pos).cloned(); self.pos += 1; t }
    fn keyword(&mut self, kw: &str) -> bool {
        if matches!(self.peek(), Some(Token { tok: Tok::Ident(s), .. }) if s.eq_ignore_ascii_case(kw)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<Expr> {
        let mut left = self.and()?;
        while self.keyword("OR") {
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr> {
        let mut left = self.not()?;
        while self.keyword("AND") {
            left = Expr::And(Box::new(left), Box::new(self.not()?));
        }
        Ok(left)
    }

    fn not(&mut self) -> Result<Expr> {
        if self.keyword("NOT") {
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr> {
        if matches!(self.peek(), Some(Token { tok: Tok::LParen, .. })) {
            self.pos += 1;
            let e = self.or()?;
            match self.next() {
                Some(Token { tok: Tok::RParen, .. }) => return Ok(e),
                _ => return Err(err(self.col(), "expected `)`")),
            }
        }
        let field = self.field()?;
        if self.keyword("IS") {
            let negated = self.keyword("NOT");
            if !self.keyword("NULL") {
                return Err(err(self.col(), "expected NULL"));
            }
            return Ok(Expr::IsNull(field, negated));
        }
        let negated = self.keyword("NOT");
        if self.keyword("LIKE") {
            return match self.next() {
                Some(Token { tok: Tok::Str(p), .. }) => Ok(Expr::Like(field, p, negated)),
                _ => Err(err(self.col(), "LIKE needs a 'string' pattern")),
            };
        }
        if self.keyword("IN") {
            match self.next() {
                Some(Token { tok: Tok::LParen, .. }) => {}
                _ => return Err(err(self.col(), "IN needs a parenthesised list")),
            }
            let mut values = vec![self.literal()?];
            loop {
                match self.next() {
                    Some(Token { tok: Tok::Comma, .. }) => values.push(self.literal()?),
                    Some(Token { tok: Tok::RParen, .. }) => break,
                    _ => return Err(err(self.col(), "expected `,` or `)`")),
                }
            }
            return Ok(Expr::In(field, values, negated));
        }
        if negated {
            return Err(err(self.col(), "expected LIKE or IN after NOT"));
        }
        let op = match self.next() {
            Some(Token { tok: Tok::Op(o), .. }) => match o {
                "=" => CmpOp::Eq, "!=" => CmpOp::Ne, "<" => CmpOp::Lt,
                "<=" => CmpOp::Le, ">" => CmpOp::Gt, _ => CmpOp::Ge,
            },
            Some(t) => return Err(err(t.col, format!("expected a comparison operator, found `{}`", describe(&t.tok)))),
            None => return Err(err(self.end_col, "expected a comparison operator")),
        };
        Ok(Expr::Cmp(field, op, self.literal()?))
    }

    fn field(&mut self) -> Result<Field> {
        match self.next() {
            Some(Token { tok: Tok::Ident(name), col }) => {
                let lower = name.to_ascii_lowercase();
                if let Some(rest) = lower.strip_prefix("sys.") {
                    SYS_NAMES
                        .iter()
                        .find(|(n, _)| *n == rest)
                        .map(|(_, f)| Field::Sys(*f))
                        .ok_or_else(|| err(col, format!("unknown system property `{name}` (known: MessageId, CorrelationId, SessionId, Subject, ContentType, To, ReplyTo, SequenceNumber, EnqueuedTime, ScheduledEnqueueTime, DeliveryCount, DeadLetterReason, DeadLetterErrorDescription, DeadLetterSource, State, Size)")))
                } else if lower.starts_with("user.") {
                    Ok(Field::User(name[5..].to_string()))
                } else {
                    Ok(Field::User(name))
                }
            }
            Some(t) => Err(err(t.col, format!("expected a property name, found `{}`", describe(&t.tok)))),
            None => Err(err(self.end_col, "expected a property name")),
        }
    }

    fn literal(&mut self) -> Result<Lit> {
        match self.next() {
            Some(Token { tok: Tok::Str(s), .. }) => Ok(Lit::Str(s)),
            Some(Token { tok: Tok::Num(n), .. }) => Ok(Lit::Num(n)),
            Some(Token { tok: Tok::Ident(s), .. }) if s.eq_ignore_ascii_case("true") => Ok(Lit::Bool(true)),
            Some(Token { tok: Tok::Ident(s), .. }) if s.eq_ignore_ascii_case("false") => Ok(Lit::Bool(false)),
            Some(t) => Err(err(t.col, format!("expected a value, found `{}`", describe(&t.tok)))),
            None => Err(err(self.end_col, "expected a value")),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum FieldValue { Null, Str(String), Num(f64), Bool(bool), Time(OffsetDateTime) }

fn opt_str(v: &Option<String>) -> FieldValue {
    v.clone().map(FieldValue::Str).unwrap_or(FieldValue::Null)
}

fn field_value(msg: &PeekedMessage, field: &Field) -> FieldValue {
    match field {
        Field::Sys(f) => match f {
            SysField::MessageId => opt_str(&msg.message_id),
            SysField::CorrelationId => opt_str(&msg.correlation_id),
            SysField::SessionId => opt_str(&msg.session_id),
            SysField::Subject => opt_str(&msg.subject),
            SysField::ContentType => opt_str(&msg.content_type),
            SysField::To => opt_str(&msg.to),
            SysField::ReplyTo => opt_str(&msg.reply_to),
            SysField::SequenceNumber => FieldValue::Num(msg.sequence_number as f64),
            SysField::EnqueuedTime => msg.enqueued_time.map(FieldValue::Time).unwrap_or(FieldValue::Null),
            SysField::ScheduledEnqueueTime => msg.scheduled_enqueue_time.map(FieldValue::Time).unwrap_or(FieldValue::Null),
            SysField::DeliveryCount => FieldValue::Num(f64::from(msg.delivery_count)),
            SysField::DeadLetterReason => opt_str(&msg.dead_letter_reason),
            SysField::DeadLetterErrorDescription => opt_str(&msg.dead_letter_description),
            SysField::DeadLetterSource => opt_str(&msg.dead_letter_source),
            SysField::State => FieldValue::Str(msg.state.as_str().to_string()),
            SysField::Size => FieldValue::Num(msg.body.len() as f64),
        },
        Field::User(name) => match msg.application_properties.get(name) {
            None | Some(PropValue::Null) => FieldValue::Null,
            Some(PropValue::Bool(b)) => FieldValue::Bool(*b),
            Some(PropValue::Int(n)) => FieldValue::Num(*n as f64),
            Some(PropValue::UInt(n)) => FieldValue::Num(*n as f64),
            Some(PropValue::Double(d)) => FieldValue::Num(*d),
            Some(PropValue::String(s)) | Some(PropValue::Uuid(s)) | Some(PropValue::Other(s)) => FieldValue::Str(s.clone()),
            Some(PropValue::Timestamp(t)) => FieldValue::Time(*t),
            Some(PropValue::Binary(b)) => FieldValue::Str(base64::engine::general_purpose::STANDARD.encode(b)),
        },
    }
}

fn order(fv: &FieldValue, lit: &Lit) -> Option<Ordering> {
    match (fv, lit) {
        (FieldValue::Num(a), Lit::Num(b)) => a.partial_cmp(b),
        (FieldValue::Str(a), Lit::Num(b)) => a.trim().parse::<f64>().ok()?.partial_cmp(b),
        (FieldValue::Str(a), Lit::Str(b)) => Some(a.as_str().cmp(b.as_str())),
        (FieldValue::Time(a), Lit::Str(b)) => Some(a.cmp(&OffsetDateTime::parse(b, &Rfc3339).ok()?)),
        (FieldValue::Bool(a), Lit::Bool(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

fn compare(fv: &FieldValue, op: CmpOp, lit: &Lit) -> bool {
    let Some(ord) = order(fv, lit) else { return false };
    match op {
        CmpOp::Eq => ord == Ordering::Equal,
        CmpOp::Ne => ord != Ordering::Equal,
        CmpOp::Lt => ord == Ordering::Less,
        CmpOp::Le => ord != Ordering::Greater,
        CmpOp::Gt => ord == Ordering::Greater,
        CmpOp::Ge => ord != Ordering::Less,
    }
}

/// `%` = any run, `_` = one character; case-insensitive.
fn like(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let (mut ti, mut pi) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '_' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '%' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '%' {
        pi += 1;
    }
    pi == p.len()
}

fn eval(expr: &Expr, msg: &PeekedMessage) -> bool {
    match expr {
        Expr::And(a, b) => eval(a, msg) && eval(b, msg),
        Expr::Or(a, b) => eval(a, msg) || eval(b, msg),
        Expr::Not(e) => !eval(e, msg),
        Expr::Cmp(f, op, lit) => compare(&field_value(msg, f), *op, lit),
        Expr::IsNull(f, negated) => (field_value(msg, f) == FieldValue::Null) != *negated,
        Expr::Like(f, pattern, negated) => {
            let text = match field_value(msg, f) {
                FieldValue::Str(s) => s,
                FieldValue::Num(n) => n.to_string(),
                FieldValue::Time(t) => t.format(&Rfc3339).unwrap_or_default(),
                FieldValue::Bool(b) => b.to_string(),
                FieldValue::Null => return false,
            };
            like(&text, pattern) != *negated
        }
        Expr::In(f, values, negated) => {
            let fv = field_value(msg, f);
            if fv == FieldValue::Null {
                return false;
            }
            values.iter().any(|v| compare(&fv, CmpOp::Eq, v)) != *negated
        }
    }
}

```

Make `test_message` reachable from the `filter` tests: in `messaging/mod.rs`, add `#[cfg(test)] pub(crate) use message::test_message;`.

When the closing parenthesis is missing at the end of input, `self.col()` returns `end_col` (input length + 1), which is what the test `"(tenant = 7"` expects (column 12). Note: `next()` has already advanced past the missing token, so `self.col()` reports the position after it. That is correct at the end of input, and acceptable for a wrong token mid-input.

- [ ] **Step 4: Run tests**

Run: `cargo test -p dispatched-core filter`
Expected: PASS, 10 tests (3 lexer and 7 expr). If a column number in `parse_errors_name_the_column` is off by one, fix the parser, not the test. The columns in the test are the 1-based character positions of the offending token, or the input length + 1 at the end of input.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy -p dispatched-core --all-targets -- -D warnings && git add -A && git commit -m "Core: property filter language (sys./user. fields, AND/OR/NOT, LIKE, IN, IS NULL)"
```

---

### Task 4: Body filters and MessageFilter

**Files:**
- Create: `crates/dispatched-core/src/filter/body.rs`
- Modify: `crates/dispatched-core/src/filter/mod.rs`, `crates/dispatched-core/Cargo.toml` (`regex = "1"`, `serde_json_path = "0.7"`), root `Cargo.toml` workspace deps (same lines)
- Modify: `crates/dispatched-core/src/lib.rs` (`pub use filter::{BodyFilter, MessageFilter, PropertyFilter};`)

**Interfaces:**
- Produces `filter::BodyFilter` with:
  - `contains(text: &str) -> Self`, which is case-insensitive.
  - `regex(pattern: &str) -> Result<Self>`.
  - `json(path: &str, expected: Option<&str>) -> Result<Self>`. `expected` is parsed as JSON when valid, otherwise treated as a string.
  - `matches(&self, body: &MessageBody) -> bool`.
- Produces `filter::MessageFilter { pub property: Option<PropertyFilter>, pub body: Vec<BodyFilter> }`, which is `Default`, with:
  - `is_empty(&self) -> bool`
  - `matches(&self, msg: &PeekedMessage) -> bool`, true when every part matches.

- [ ] **Step 1: Write `filter/body.rs` with tests**

```rust
//! Filters on message body content.

use crate::messaging::MessageBody;
use crate::{Error, Result};
use serde_json_path::JsonPath;

/// Bodies larger than this are not searched (keeps regex and JSON parsing bounded).
const MAX_SEARCH_BYTES: usize = 4 * 1024 * 1024;

/// A predicate on the message body.
#[derive(Debug, Clone)]
pub enum BodyFilter {
    /// Case-insensitive substring.
    Contains(String),
    /// Regular expression (Rust `regex` syntax) on the body text.
    Regex(regex::Regex),
    /// JSONPath (RFC 9535) on a JSON body; optionally a node must equal `expected`.
    Json {
        /// Parsed path.
        path: JsonPath,
        /// Original path text.
        source: String,
        /// Required node value.
        expected: Option<serde_json::Value>,
    },
}

impl BodyFilter {
    /// Substring match (case-insensitive).
    pub fn contains(text: &str) -> Self {
        BodyFilter::Contains(text.to_lowercase())
    }

    /// Regex match.
    pub fn regex(pattern: &str) -> Result<Self> {
        regex::RegexBuilder::new(pattern)
            .size_limit(1 << 20)
            .build()
            .map(BodyFilter::Regex)
            .map_err(|e| Error::Invalid(format!("body regex: {e}")))
    }

    /// JSONPath match, e.g. `$.order.status` with expected `"shipped"`.
    pub fn json(path: &str, expected: Option<&str>) -> Result<Self> {
        let parsed = JsonPath::parse(path).map_err(|e| Error::Invalid(format!("body JSONPath `{path}`: {e}")))?;
        let expected = expected.map(|v| serde_json::from_str(v).unwrap_or_else(|_| serde_json::Value::String(v.to_string())));
        Ok(BodyFilter::Json { path: parsed, source: path.to_string(), expected })
    }

    /// Whether `body` satisfies this filter. Non-text bodies never match.
    pub fn matches(&self, body: &MessageBody) -> bool {
        if body.len() > MAX_SEARCH_BYTES {
            return false;
        }
        let Some(text) = body.as_text() else { return false };
        match self {
            BodyFilter::Contains(needle) => text.to_lowercase().contains(needle.as_str()),
            BodyFilter::Regex(re) => re.is_match(&text),
            BodyFilter::Json { path, expected, .. } => {
                let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
                let nodes = path.query(&doc).all();
                match expected {
                    None => !nodes.is_empty(),
                    Some(want) => nodes.iter().any(|n| json_equal(n, want)),
                }
            }
        }
    }
}

fn json_equal(node: &serde_json::Value, want: &serde_json::Value) -> bool {
    match (node, want) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => a.as_f64() == b.as_f64(),
        (serde_json::Value::String(a), serde_json::Value::Number(b)) => a.parse::<f64>().ok() == b.as_f64(),
        _ => *node == *want,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> MessageBody {
        MessageBody::Text(s.into())
    }

    #[test]
    fn contains_is_case_insensitive() {
        assert!(BodyFilter::contains("ORDER").matches(&text("an order arrived")));
        assert!(!BodyFilter::contains("refund").matches(&text("an order arrived")));
    }

    #[test]
    fn regex_matches_text_and_rejects_bad_patterns() {
        assert!(BodyFilter::regex(r"id=\d{3}").unwrap().matches(&text("x id=123 y")));
        assert!(BodyFilter::regex("(").unwrap_err().to_string().contains("body regex"));
    }

    #[test]
    fn json_path_presence_and_equality() {
        let body = text(r#"{"order":{"status":"shipped","qty":"2","lines":[{"sku":"A"}]}}"#);
        assert!(BodyFilter::json("$.order.status", None).unwrap().matches(&body));
        assert!(BodyFilter::json("$.order.status", Some("shipped")).unwrap().matches(&body));
        assert!(BodyFilter::json("$.order.qty", Some("2")).unwrap().matches(&body));
        assert!(BodyFilter::json("$.order.lines[*].sku", Some("\"A\"")).unwrap().matches(&body));
        assert!(!BodyFilter::json("$.order.status", Some("open")).unwrap().matches(&body));
        assert!(!BodyFilter::json("$.missing", None).unwrap().matches(&body));
        assert!(!BodyFilter::json("$.order", None).unwrap().matches(&text("not json")));
        assert!(BodyFilter::json("$[", None).is_err());
    }

    #[test]
    fn body_filters_ignore_binary_bodies() {
        let binary = MessageBody::Binary(vec![0xff, 0xfe, 0x00]);
        assert!(!BodyFilter::contains("x").matches(&binary));
        assert!(!BodyFilter::regex(".*").unwrap().matches(&binary));
    }
}
```

- [ ] **Step 2: Write `filter/mod.rs`**

```rust
//! Client-side filters applied to peeked messages.

mod body;
mod expr;
mod lexer;

pub use body::BodyFilter;
pub use expr::PropertyFilter;

use crate::messaging::PeekedMessage;

/// Property and body conditions combined with AND.
#[derive(Debug, Clone, Default)]
pub struct MessageFilter {
    /// Optional property expression.
    pub property: Option<PropertyFilter>,
    /// Body conditions (all must match).
    pub body: Vec<BodyFilter>,
}

impl MessageFilter {
    /// No conditions.
    pub fn is_empty(&self) -> bool {
        self.property.is_none() && self.body.is_empty()
    }

    /// Whether `msg` satisfies every condition.
    pub fn matches(&self, msg: &PeekedMessage) -> bool {
        self.property.as_ref().is_none_or(|p| p.matches(msg)) && self.body.iter().all(|b| b.matches(&msg.body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::{MessageBody, test_message};

    #[test]
    fn empty_filter_matches_everything() {
        assert!(MessageFilter::default().is_empty());
        assert!(MessageFilter::default().matches(&test_message(1)));
    }

    #[test]
    fn property_and_body_are_anded() {
        let mut m = test_message(1);
        m.subject = Some("a".into());
        m.body = MessageBody::Text("hello world".into());
        let f = MessageFilter {
            property: Some(PropertyFilter::parse("sys.Subject = 'a'").unwrap()),
            body: vec![BodyFilter::contains("WORLD")],
        };
        assert!(f.matches(&m));
        m.subject = Some("b".into());
        assert!(!f.matches(&m));
    }
}
```

- [ ] **Step 3: Run tests and lint**

Run: `cargo test -p dispatched-core filter && cargo clippy -p dispatched-core --all-targets -- -D warnings`
Expected: PASS, 16 filter tests, clean. If `serde_json_path` 0.7's query API differs (`query(&doc).all()`), adapt to its documented API (check `~/.cargo/registry/src/*/serde_json_path-0.7.*/src/lib.rs`) and record the change.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: body filters (contains, regex, JSONPath) and MessageFilter"
```

---
### Task 5: Peek views, scanning, and the new Explorer peek API

**Files:**
- Modify: `crates/dispatched-core/src/namespace.rs` (add `PeekView`)
- Create: `crates/dispatched-core/src/messaging/scan.rs`
- Modify: `crates/dispatched-core/src/messaging/mod.rs`, `src/explorer.rs`, `src/lib.rs`
- Modify callers so they compile:
  - `crates/dispatched-cli/src/main.rs` (peek command)
  - `app/src-tauri/src/commands/browse.rs`
  - `crates/dispatched-core/tests/emulator_m0.rs`

**Interfaces:**
- Produces `namespace::PeekView::{Active, Scheduled, Deferred, DeadLetter, TransferDeadLetter}`, serialised kebab-case, with:
  - `sub_queue(self) -> SubQueue`
  - `accepts(self, MessageState) -> bool`
  - `as_str(self) -> &'static str`
  - `parse(&str) -> Result<Self>`, which accepts `active|scheduled|deferred|dlq|dead-letter|transfer-dlq|transfer-dead-letter`.
- Produces in `messaging::scan`:
  - `DEFAULT_SCAN_LIMIT: u32 = 10_000`
  - `#[async_trait] trait PeekSource: Send { async fn peek_batch(&mut self, address: &str, from: i64, max: u32) -> Result<Vec<PeekedMessage>>; }`, implemented for `ServiceBusConnection`.
  - `PeekOptions { from_sequence: i64, max: u32, scan_limit: u32, filter: MessageFilter }`, with `new(max)` and the builders `from_sequence`, `scan_limit`, `filter`.
  - `PeekPage { messages: Vec<PeekedMessage>, scanned: u32, next_sequence: Option<i64>, exhausted: bool }` (Serialize).
  - `async fn scan_messages(src: &mut (impl PeekSource + ?Sized), address: &str, view: PeekView, opts: &PeekOptions) -> Result<PeekPage>`
- Changes `Explorer::peek` to `peek(&self, entity: &EntityPath, view: PeekView, opts: &PeekOptions) -> Result<PeekPage>`.

- [ ] **Step 1: Add PeekView** (in `namespace.rs`, after `SubQueue`)

```rust
/// What a user wants to see. Scheduled and deferred messages live in the
/// entity itself and are told apart by their state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeekView {
    /// Active messages.
    Active,
    /// Scheduled (not yet enqueued) messages.
    Scheduled,
    /// Deferred messages.
    Deferred,
    /// The dead-letter queue (all states).
    DeadLetter,
    /// The transfer dead-letter queue (all states).
    TransferDeadLetter,
}

impl PeekView {
    /// Queue to read from.
    pub fn sub_queue(self) -> SubQueue {
        match self {
            PeekView::Active | PeekView::Scheduled | PeekView::Deferred => SubQueue::Active,
            PeekView::DeadLetter => SubQueue::DeadLetter,
            PeekView::TransferDeadLetter => SubQueue::TransferDeadLetter,
        }
    }

    /// Whether a message in `state` belongs to this view.
    pub fn accepts(self, state: crate::messaging::MessageState) -> bool {
        use crate::messaging::MessageState as S;
        match self {
            PeekView::Active => state == S::Active,
            PeekView::Scheduled => state == S::Scheduled,
            PeekView::Deferred => state == S::Deferred,
            PeekView::DeadLetter | PeekView::TransferDeadLetter => true,
        }
    }

    /// Stable name.
    pub fn as_str(self) -> &'static str {
        match self {
            PeekView::Active => "active",
            PeekView::Scheduled => "scheduled",
            PeekView::Deferred => "deferred",
            PeekView::DeadLetter => "dead-letter",
            PeekView::TransferDeadLetter => "transfer-dead-letter",
        }
    }

    /// Parse a user-facing name.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Ok(PeekView::Active),
            "scheduled" => Ok(PeekView::Scheduled),
            "deferred" => Ok(PeekView::Deferred),
            "dlq" | "dead-letter" | "deadletter" => Ok(PeekView::DeadLetter),
            "transfer-dlq" | "transfer-dead-letter" => Ok(PeekView::TransferDeadLetter),
            other => Err(Error::Invalid(format!(
                "unknown view `{other}`; use active, scheduled, deferred, dlq or transfer-dlq"
            ))),
        }
    }
}
```

Add to the `namespace.rs` tests:

```rust
    #[test]
    fn peek_views_map_to_sub_queues_and_states() {
        use crate::messaging::MessageState as S;
        assert_eq!(PeekView::Scheduled.sub_queue(), SubQueue::Active);
        assert_eq!(PeekView::parse("DLQ").unwrap(), PeekView::DeadLetter);
        assert!(PeekView::Scheduled.accepts(S::Scheduled));
        assert!(!PeekView::Active.accepts(S::Deferred));
        assert!(PeekView::DeadLetter.accepts(S::Deferred));
        assert!(PeekView::parse("nope").unwrap_err().to_string().contains("unknown view"));
    }
```

- [ ] **Step 2: Write `messaging/scan.rs`, tests first**

```rust
//! Peek through an entity, keeping only messages that match a view and filter.

use super::connection::ServiceBusConnection;
use super::message::PeekedMessage;
use super::peek::{self, MAX_PER_REQUEST};
use crate::Result;
use crate::filter::MessageFilter;
use crate::namespace::PeekView;
use async_trait::async_trait;
use serde::Serialize;

/// Messages examined per peek call unless the caller says otherwise.
pub const DEFAULT_SCAN_LIMIT: u32 = 10_000;

/// Anything that can return a batch of peeked messages.
#[async_trait]
pub trait PeekSource: Send {
    /// Peek up to `max` messages from `address` starting at sequence `from`.
    async fn peek_batch(&mut self, address: &str, from: i64, max: u32) -> Result<Vec<PeekedMessage>>;
}

#[async_trait]
impl PeekSource for ServiceBusConnection {
    async fn peek_batch(&mut self, address: &str, from: i64, max: u32) -> Result<Vec<PeekedMessage>> {
        peek::peek(self, address, from, max).await
    }
}

/// What to peek.
#[derive(Debug, Clone)]
pub struct PeekOptions {
    /// First sequence number to look at.
    pub from_sequence: i64,
    /// Maximum matching messages to return.
    pub max: u32,
    /// Maximum messages to examine (matching or not).
    pub scan_limit: u32,
    /// Property/body filter.
    pub filter: MessageFilter,
}

impl PeekOptions {
    /// Return up to `max` matches from the start, scanning at most [`DEFAULT_SCAN_LIMIT`].
    pub fn new(max: u32) -> Self {
        Self { from_sequence: 0, max, scan_limit: DEFAULT_SCAN_LIMIT, filter: MessageFilter::default() }
    }
    /// Start at `seq`.
    pub fn from_sequence(mut self, seq: i64) -> Self {
        self.from_sequence = seq;
        self
    }
    /// Examine at most `limit` messages.
    pub fn scan_limit(mut self, limit: u32) -> Self {
        self.scan_limit = limit.max(1);
        self
    }
    /// Apply `filter`.
    pub fn filter(mut self, filter: MessageFilter) -> Self {
        self.filter = filter;
        self
    }
}

/// One page of results.
#[derive(Debug, Clone, Serialize)]
pub struct PeekPage {
    /// Matching messages, in sequence order.
    pub messages: Vec<PeekedMessage>,
    /// Messages examined.
    pub scanned: u32,
    /// Where to continue, or `None` when the end of the entity was reached.
    pub next_sequence: Option<i64>,
    /// The end of the entity was reached.
    pub exhausted: bool,
}

/// Peek through `address`, keeping messages accepted by `view` and `opts.filter`.
/// Stops at `opts.max` matches, `opts.scan_limit` examined messages, an empty
/// batch, or a batch whose sequence numbers do not advance.
pub async fn scan_messages(
    src: &mut (impl PeekSource + ?Sized),
    address: &str,
    view: PeekView,
    opts: &PeekOptions,
) -> Result<PeekPage> {
    let mut out: Vec<PeekedMessage> = Vec::new();
    let mut scanned: u32 = 0;
    let mut next = opts.from_sequence;
    let mut exhausted = false;
    'outer: while (out.len() as u32) < opts.max && scanned < opts.scan_limit {
        let want = (opts.scan_limit - scanned).min(MAX_PER_REQUEST);
        let batch = src.peek_batch(address, next, want).await?;
        if batch.is_empty() {
            exhausted = true;
            break;
        }
        let mut advanced = false;
        for m in batch {
            let wanted = view.accepts(m.state) && opts.filter.matches(&m);
            if wanted && out.len() as u32 >= opts.max {
                // Leave this match for the next page.
                next = m.sequence_number;
                break 'outer;
            }
            scanned += 1;
            if m.sequence_number >= next {
                next = m.sequence_number + 1;
                advanced = true;
            }
            if wanted {
                out.push(m);
            }
            if scanned >= opts.scan_limit {
                break 'outer;
            }
        }
        if !advanced {
            break;
        }
    }
    Ok(PeekPage { messages: out, scanned, next_sequence: if exhausted { None } else { Some(next) }, exhausted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::PropertyFilter;
    use crate::messaging::{MessageState, test_message};

    struct Fake {
        messages: Vec<PeekedMessage>,
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Fake {
        async fn peek_batch(&mut self, _address: &str, from: i64, max: u32) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok(self.messages.iter().filter(|m| m.sequence_number >= from).take(max as usize).cloned().collect())
        }
    }

    fn entity(n: i64, scheduled_every: i64) -> Fake {
        let messages = (1..=n)
            .map(|i| {
                let mut m = test_message(i);
                if scheduled_every > 0 && i % scheduled_every == 0 {
                    m.state = MessageState::Scheduled;
                }
                m
            })
            .collect();
        Fake { messages, calls: 0 }
    }

    #[tokio::test]
    async fn views_keep_only_their_state() {
        let mut src = entity(10, 5);
        let page = scan_messages(&mut src, "q", PeekView::Scheduled, &PeekOptions::new(50)).await.unwrap();
        assert_eq!(page.messages.iter().map(|m| m.sequence_number).collect::<Vec<_>>(), [5, 10]);
        assert!(page.exhausted);
        assert_eq!(page.next_sequence, None);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(50)).await.unwrap();
        assert_eq!(page.messages.len(), 8);
    }

    #[tokio::test]
    async fn max_leaves_the_next_match_for_the_continuation() {
        let mut src = entity(10, 0);
        let first = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(3)).await.unwrap();
        assert_eq!(first.messages.iter().map(|m| m.sequence_number).collect::<Vec<_>>(), [1, 2, 3]);
        let next = first.next_sequence.unwrap();
        let second = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(3).from_sequence(next)).await.unwrap();
        assert_eq!(second.messages.iter().map(|m| m.sequence_number).collect::<Vec<_>>(), [4, 5, 6]);
    }

    #[tokio::test]
    async fn scan_stops_at_limit_with_continuation() {
        let mut src = entity(100_000, 0);
        let opts = PeekOptions::new(10).scan_limit(1_000);
        let page = scan_messages(&mut src, "q", PeekView::Scheduled, &opts).await.unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(page.scanned, 1_000);
        assert_eq!(page.next_sequence, Some(1_001));
        assert!(!page.exhausted);
        assert!(src.calls <= 4, "1000 / 250 batches, got {}", src.calls);
    }

    /// A misbehaving source that ignores `from` and always returns the same
    /// messages without sequence numbers.
    struct Stuck {
        calls: u32,
    }

    #[async_trait]
    impl PeekSource for Stuck {
        async fn peek_batch(&mut self, _address: &str, _from: i64, _max: u32) -> Result<Vec<PeekedMessage>> {
            self.calls += 1;
            Ok((0..5).map(|_| test_message(-1)).collect())
        }
    }

    #[tokio::test]
    async fn scan_stops_when_sequence_does_not_advance() {
        let mut src = Stuck { calls: 0 };
        let page = scan_messages(&mut src, "q", PeekView::DeadLetter, &PeekOptions::new(100)).await.unwrap();
        assert_eq!(src.calls, 1, "must not re-read the same range");
        assert_eq!(page.messages.len(), 5);
        assert!(!page.exhausted);
    }

    #[tokio::test]
    async fn filters_apply_during_the_scan() {
        let mut src = entity(20, 0);
        for m in src.messages.iter_mut() {
            m.delivery_count = (m.sequence_number % 3) as u32;
        }
        let filter = MessageFilter {
            property: Some(PropertyFilter::parse("sys.DeliveryCount = 2").unwrap()),
            body: vec![],
        };
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(100).filter(filter)).await.unwrap();
        assert!(page.messages.iter().all(|m| m.delivery_count == 2));
        assert_eq!(page.messages.len(), 7);
    }

    #[tokio::test]
    async fn empty_entity_is_exhausted_immediately() {
        let mut src = entity(0, 0);
        let page = scan_messages(&mut src, "q", PeekView::Active, &PeekOptions::new(10)).await.unwrap();
        assert!(page.exhausted && page.messages.is_empty() && page.scanned == 0);
    }
}
```

Update `messaging/mod.rs`:
```rust
//! AMQP data plane: connection, CBS, management link, message model, peek.

mod connection;
mod management;
mod message;
pub mod peek;
pub mod scan;
#[cfg(feature = "test-support")]
pub mod testing;

pub use connection::ServiceBusConnection;
pub use management::{ManagementLink, MgmtResponse};
pub use message::{MessageBody, MessageState, PeekedMessage, PropValue, decode_bytes, decode_message};
#[cfg(test)]
pub(crate) use message::test_message;
pub use scan::{DEFAULT_SCAN_LIMIT, PeekOptions, PeekPage, PeekSource, scan_messages};
```

- [ ] **Step 3: Change `Explorer::peek`**

Replace `peek` and delete `peek_all` in `explorer.rs`:

```rust
    /// Peek `view` of `entity` according to `opts`.
    pub async fn peek(&self, entity: &EntityPath, view: PeekView, opts: &PeekOptions) -> Result<PeekPage> {
        let address = entity.address(view.sub_queue());
        let mut guard = self.amqp.lock().await;
        if guard.is_none() {
            *guard = Some(ServiceBusConnection::open(&self.ns).await?);
        }
        let conn = guard.as_mut().expect("opened above");
        let result = scan_messages(conn, &address, view, opts).await;
        if result.as_ref().is_err_and(breaks_connection) {
            if let Some(conn) = guard.take() {
                let _ = conn.close().await;
            }
        }
        result
    }
```

Fix the imports: `use crate::messaging::{PeekOptions, PeekPage, ServiceBusConnection, scan_messages};` and `use crate::namespace::{EntityPath, Namespace, PeekView};`. Remove the now-unused `peek` and `SubQueue` imports.

Add to the `lib.rs` re-exports: `pub use messaging::{MessageBody, MessageState, PeekOptions, PeekPage, PeekedMessage, PropValue};`, and add `PeekView` to the `namespace` re-export.

- [ ] **Step 4: Update callers**

- **CLI** (`crates/dispatched-cli/src/main.rs`, `Command::Peek` handler):
  - Map the flags to a view: `--dlq` → `PeekView::DeadLetter`, `--transfer-dlq` → `PeekView::TransferDeadLetter`, otherwise `PeekView::Active`.
  - Call `explorer.peek(&path, view, &PeekOptions::new(top).from_sequence(from_seq)).await?` and print `page.messages`.
  - Task 13 adds the remaining flags.
- **App** (`app/src-tauri/src/commands/browse.rs`): keep the command signature, `sub_queue: SubQueue`. Map `SubQueue::Active` → `PeekView::Active`, `SubQueue::DeadLetter` → `PeekView::DeadLetter`, `SubQueue::TransferDeadLetter` → `PeekView::TransferDeadLetter`. Return `page.messages`, built with `PeekOptions::new(max.min(500)).from_sequence(from_sequence)`.
- **Test** (`crates/dispatched-core/tests/emulator_m0.rs`): replace each `explorer.peek(&e, SubQueue::X, from, max)` with `explorer.peek(&e, PeekView::X, &PeekOptions::new(max).from_sequence(from)).await.unwrap().messages`. That is `PeekView::Active` and `PeekView::DeadLetter`.

- [ ] **Step 5: Run tests**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings && (cd app && npm run build) && cargo clippy -p dispatched-app -- -D warnings`
Expected: PASS (6 new scan tests and 1 namespace test) and clean everywhere.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A && git commit -m "Core: peek views (active/scheduled/deferred/DLQ/transfer DLQ) with bounded, filterable scanning"
```

---

### Task 6: Export (JSON / NDJSON, schema v1)

**Files:**
- Create: `crates/dispatched-core/src/export.rs`
- Modify: `crates/dispatched-core/src/lib.rs` (`pub mod export;` and `pub use export::{EXPORT_SCHEMA, ExportFormat, ExportedMessage, write_messages};`)

**Interfaces:**
- Produces:
  - `EXPORT_SCHEMA = "dispatched.message/v1"`
  - `ExportedMessage<'a> { schema, #[flatten] message: &'a PeekedMessage, body_text: Option<String> }`. `body_text` is set only for binary bodies that are valid UTF-8.
  - `ExportFormat::{Json, Ndjson}`, with `parse(&str) -> Result<Self>`.
  - `write_messages(writer: impl std::io::Write, messages: &[PeekedMessage], format: ExportFormat) -> Result<usize>`
- The CLI uses `ExportedMessage` for every JSON message output, so the schema is the same everywhere. M2's import reads this format.

- [ ] **Step 1: Write `export.rs` with tests**

```rust
//! Stable JSON shape for messages leaving Dispatched (CLI output, files).

use crate::messaging::{MessageBody, PeekedMessage};
use crate::{Error, Result};
use serde::Serialize;
use std::io::Write;

/// Schema identifier written into every exported message.
pub const EXPORT_SCHEMA: &str = "dispatched.message/v1";

/// A message in export form.
#[derive(Debug, Serialize)]
pub struct ExportedMessage<'a> {
    /// Always [`EXPORT_SCHEMA`].
    pub schema: &'static str,
    /// The message fields.
    #[serde(flatten)]
    pub message: &'a PeekedMessage,
    /// The body as text when it is UTF-8 binary (convenience; `body` stays authoritative).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
}

impl<'a> ExportedMessage<'a> {
    /// Wrap `message`.
    pub fn new(message: &'a PeekedMessage) -> Self {
        let body_text = match &message.body {
            MessageBody::Binary(_) => message.body.as_text(),
            _ => None,
        };
        Self { schema: EXPORT_SCHEMA, message, body_text }
    }
}

/// Output layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One pretty-printed JSON array.
    Json,
    /// One JSON object per line.
    Ndjson,
}

impl ExportFormat {
    /// Parse `json` or `ndjson`.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(ExportFormat::Json),
            "ndjson" | "jsonl" => Ok(ExportFormat::Ndjson),
            other => Err(Error::Invalid(format!("unknown export format `{other}`; use json or ndjson"))),
        }
    }
}

fn io(e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("cannot write export: {e}"))
}

/// Write `messages` to `w`. Returns the number written.
pub fn write_messages(mut w: impl Write, messages: &[PeekedMessage], format: ExportFormat) -> Result<usize> {
    let exported: Vec<ExportedMessage<'_>> = messages.iter().map(ExportedMessage::new).collect();
    match format {
        ExportFormat::Json => {
            serde_json::to_writer_pretty(&mut w, &exported).map_err(io)?;
            writeln!(w).map_err(io)?;
        }
        ExportFormat::Ndjson => {
            for m in &exported {
                serde_json::to_writer(&mut w, m).map_err(io)?;
                writeln!(w).map_err(io)?;
            }
        }
    }
    w.flush().map_err(io)?;
    Ok(exported.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::test_message;

    fn two() -> Vec<PeekedMessage> {
        let mut a = test_message(1);
        a.body = MessageBody::Binary(b"{\"x\":1}".to_vec());
        let mut b = test_message(2);
        b.body = MessageBody::Binary(vec![0xff, 0x00]);
        vec![a, b]
    }

    #[test]
    fn json_array_carries_schema_and_text_convenience() {
        let mut out = Vec::new();
        assert_eq!(write_messages(&mut out, &two(), ExportFormat::Json).unwrap(), 2);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v[0]["schema"], EXPORT_SCHEMA);
        assert_eq!(v[0]["sequence_number"], 1);
        assert_eq!(v[0]["body_text"], "{\"x\":1}");
        assert_eq!(v[0]["body"]["kind"], "binary");
        assert!(v[1].get("body_text").is_none());
    }

    #[test]
    fn ndjson_is_one_object_per_line() {
        let mut out = Vec::new();
        write_messages(&mut out, &two(), ExportFormat::Ndjson).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["schema"], EXPORT_SCHEMA);
        }
    }

    #[test]
    fn format_names() {
        assert_eq!(ExportFormat::parse("NDJSON").unwrap(), ExportFormat::Ndjson);
        assert!(ExportFormat::parse("csv").is_err());
    }
}
```

- [ ] **Step 2: Run tests and commit**

Run: `cargo test -p dispatched-core export && cargo clippy -p dispatched-core --all-targets -- -D warnings`
Expected: PASS, 3 tests.

```bash
cargo fmt --all && git add -A && git commit -m "Core: message export (schema dispatched.message/v1, JSON and NDJSON)"
```

---

### Task 7: Long sessions — token re-put, link cache cap, reconnect after auth errors

**Files:**
- Create: `crates/dispatched-core/src/messaging/tokens_ledger.rs`
- Modify: `crates/dispatched-core/src/messaging/connection.rs`, `messaging/mod.rs` (`mod tokens_ledger;`), `src/explorer.rs`, `src/tokens.rs`

**Interfaces:**
- Produces in `tokens_ledger.rs` (`pub(crate)`):
  - `REFRESH_MARGIN: time::Duration = 5 min`.
  - `TokenLedger`, which is `Default`, with `record(&mut self, audience: &str, expires_on: OffsetDateTime)`, `needs_put(&self, audience: &str, now: OffsetDateTime) -> bool` and `forget(&mut self, audience: &str)`.
  - `LinkLru::new(capacity)` with `touch(&mut self, address: &str) -> Option<String>`. `touch` returns the evicted address when it is over capacity.
- Produces `ServiceBusConnection::ensure_authorized(&mut self, address: &str) -> Result<()>`, which re-puts a token if it is missing or within the margin. `management(address)` calls it for `address` and `{address}/$management` on every use and keeps at most 64 links (LRU).
- Produces `explorer::retry_after(err: &Error, attempt: u32) -> bool`, which is true only for `Error::Auth(_)` on attempt 0. `Explorer::peek` drops the connection and retries once when it is true.
- Produces `SasKeySource::with_ttl(self, ttl: std::time::Duration) -> Self`.

- [ ] **Step 1: Write `tokens_ledger.rs` with tests**

```rust
//! When to re-put CBS tokens, and which management links to keep.

use std::collections::{HashMap, VecDeque};
use time::OffsetDateTime;

/// Re-put a token when it has less than this left.
pub(crate) const REFRESH_MARGIN: time::Duration = time::Duration::minutes(5);

/// Token expiry per CBS audience.
#[derive(Debug, Default)]
pub(crate) struct TokenLedger {
    expiries: HashMap<String, OffsetDateTime>,
}

impl TokenLedger {
    pub(crate) fn record(&mut self, audience: &str, expires_on: OffsetDateTime) {
        self.expiries.insert(audience.to_string(), expires_on);
    }

    pub(crate) fn needs_put(&self, audience: &str, now: OffsetDateTime) -> bool {
        self.expiries.get(audience).is_none_or(|exp| *exp - now <= REFRESH_MARGIN)
    }

    pub(crate) fn forget(&mut self, audience: &str) {
        self.expiries.remove(audience);
    }
}

/// Least-recently-used set of management-link addresses.
#[derive(Debug)]
pub(crate) struct LinkLru {
    capacity: usize,
    order: VecDeque<String>,
}

impl LinkLru {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), order: VecDeque::new() }
    }

    /// Mark `address` as used. Returns an address to evict when over capacity.
    pub(crate) fn touch(&mut self, address: &str) -> Option<String> {
        if let Some(pos) = self.order.iter().position(|a| a == address) {
            self.order.remove(pos);
        }
        self.order.push_back(address.to_string());
        if self.order.len() > self.capacity { self.order.pop_front() } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_refreshes_within_margin() {
        let now = OffsetDateTime::now_utc();
        let mut l = TokenLedger::default();
        assert!(l.needs_put("a", now), "unknown audience needs a token");
        l.record("a", now + time::Duration::minutes(60));
        assert!(!l.needs_put("a", now));
        assert!(l.needs_put("a", now + time::Duration::minutes(56)));
        l.forget("a");
        assert!(l.needs_put("a", now));
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut lru = LinkLru::new(2);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("b"), None);
        assert_eq!(lru.touch("a"), None);
        assert_eq!(lru.touch("c").as_deref(), Some("b"));
    }
}
```

- [ ] **Step 2: Use the ledger and the LRU in `connection.rs`**

Add to the fields: `ledger: TokenLedger, lru: LinkLru,`. Initialise them in `open` with `ledger: TokenLedger::default(), lru: LinkLru::new(MAX_MANAGEMENT_LINKS),` and add `const MAX_MANAGEMENT_LINKS: usize = 64;`. Import `super::tokens_ledger::{LinkLru, TokenLedger}` and `time::OffsetDateTime`.

In `authorize`, after a successful put, record the expiry. Capture `token.expires_on` before moving the token:

```rust
        let expires_on = token.expires_on;
        // ... existing put ...
        .map_err(|e| map_put_token_error(&audience, address, e))?;
        self.ledger.record(&audience, expires_on);
        Ok(())
```

Add:

```rust
    /// Put a token for `address` unless a fresh one is already in place.
    pub async fn ensure_authorized(&mut self, address: &str) -> Result<()> {
        let audience = self.ns.amqp_audience(address);
        if self.ledger.needs_put(&audience, OffsetDateTime::now_utc()) {
            self.authorize(address).await?;
        }
        Ok(())
    }
```

Replace `management`:

```rust
    pub async fn management(&mut self, address: &str) -> Result<&mut ManagementLink> {
        self.ensure_authorized(address).await?;
        self.ensure_authorized(&format!("{address}/$management")).await?;
        if !self.management.contains_key(address) {
            let link = ManagementLink::attach(&mut self.session, address).await?;
            self.management.insert(address.to_string(), link);
        }
        if let Some(evicted) = self.lru.touch(address)
            && let Some(old) = self.management.remove(&evicted)
        {
            old.close().await;
            self.ledger.forget(&self.ns.amqp_audience(&evicted));
            self.ledger.forget(&self.ns.amqp_audience(&format!("{evicted}/$management")));
        }
        Ok(self.management.get_mut(address).expect("inserted above"))
    }
```

- [ ] **Step 3: Retry once after an auth error** (`explorer.rs`)

```rust
/// Whether a failed operation should be retried on a fresh connection.
/// An `Auth` error on the first attempt usually means a token expired
/// mid-session; a new connection re-puts fresh tokens.
pub fn retry_after(err: &Error, attempt: u32) -> bool {
    attempt == 0 && matches!(err, Error::Auth(_))
}
```

Rewrite `Explorer::peek` as a two-attempt loop:

```rust
    pub async fn peek(&self, entity: &EntityPath, view: PeekView, opts: &PeekOptions) -> Result<PeekPage> {
        let address = entity.address(view.sub_queue());
        let mut guard = self.amqp.lock().await;
        let mut attempt = 0;
        loop {
            if guard.is_none() {
                *guard = Some(ServiceBusConnection::open(&self.ns).await?);
            }
            let conn = guard.as_mut().expect("opened above");
            let result = scan_messages(conn, &address, view, opts).await;
            match result {
                Err(e) if retry_after(&e, attempt) || breaks_connection(&e) => {
                    if let Some(conn) = guard.take() {
                        let _ = conn.close().await;
                    }
                    if retry_after(&e, attempt) {
                        attempt += 1;
                        continue;
                    }
                    return Err(e);
                }
                other => return other,
            }
        }
    }
```

Add the test:

```rust
    #[test]
    fn explorer_retries_once_after_auth_error() {
        assert!(retry_after(&Error::Auth("expired".into()), 0));
        assert!(!retry_after(&Error::Auth("expired".into()), 1));
        assert!(!retry_after(&Error::Network("x".into()), 0));
        assert!(!retry_after(&Error::Forbidden { needed: None, detail: "x".into() }, 0));
    }
```

- [ ] **Step 4: Configurable SAS TTL** (`tokens.rs`)

Add a `ttl: Duration` field to `SasKeySource`, initialised to `SAS_TTL` in `new`. Add:

```rust
    /// Override the token lifetime (tests and short-lived tokens).
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }
```

and pass `self.ttl` to `sas_token`. Add a test that `SasKeySource::new(...).with_ttl(Duration::from_secs(60))` yields `expires_on` within 60 s of now (±5 s).

- [ ] **Step 5: Run tests and commit**

Run: `cargo test -p dispatched-core && cargo clippy --all-targets -- -D warnings && cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings`
Expected: PASS, clean.

```bash
cargo fmt --all && git add -A && git commit -m "Core: re-put CBS tokens before expiry, cap management links (LRU 64), retry once after auth errors"
```

---

### Task 8: AMQP over WebSockets and HTTP proxies

**Files:**
- Create: `crates/dispatched-core/src/messaging/transport.rs`
- Modify: `crates/dispatched-core/src/namespace.rs` (`Transport`, `ProxyConfig`, fields and builders on `Namespace`)
- Modify: `crates/dispatched-core/src/messaging/connection.rs` (open through `transport::open_connection`)
- Modify: `crates/dispatched-core/src/mgmt/mod.rs` (`ManagementClient::new` honours the namespace proxy)
- Modify: `crates/dispatched-core/Cargo.toml` and root `Cargo.toml` (`fe2o3-amqp-ws = { version = "0.18", features = ["rustls-tls-webpki-roots"] }`), `messaging/mod.rs` (`mod transport;`), `lib.rs` (re-export `Transport`, `ProxyConfig`)

**Interfaces:**
- Produces `namespace::Transport::{Amqp, WebSockets}`, serialised lowercase (`"amqp"`, `"websockets"`), with `parse(&str) -> Result<Self>`.
- Produces `namespace::ProxyConfig`, which is Clone with a redacting `Debug`:
  - `parse(url: &str) -> Result<Self>`. It accepts `http://[user:pass@]host:port`.
  - `from_env() -> Option<Self>`, which reads `HTTPS_PROXY` then `https_proxy`. An invalid value is ignored with a `tracing::warn!`.
  - Accessors `host() -> &str`, `port() -> u16`, `basic_auth() -> Option<(String, SecretString)>` and `url_for_reqwest() -> String`. The last one includes credentials, so it must never be logged.
- Produces on `Namespace`:
  - `with_transport(self, Transport) -> Result<Self>`, which returns `Error::Unsupported` for WebSockets on the emulator.
  - `with_proxy(self, Option<ProxyConfig>) -> Result<Self>`. `Some` on the emulator returns `Unsupported`. `Some` forces WebSockets.
  - `transport() -> Transport` and `proxy() -> Option<&ProxyConfig>`.
- Produces in `messaging::transport` (`pub(crate)`):
  - `proxy_connect<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, host: &str, port: u16, auth: Option<(&str, &str)>, timeout: std::time::Duration) -> Result<()>`
  - `open_connection(ns: &Namespace, container_id: String) -> Result<ConnectionHandle<()>>`

- [ ] **Step 1: Write the proxy CONNECT handshake with tests** (`transport.rs`)

```rust
//! Opening the AMQP connection: TLS on 5671, plain on the emulator, or
//! WebSockets on 443 (optionally through an HTTP CONNECT proxy).

use super::management::SETUP_TIMEOUT;
use crate::namespace::{Namespace, Transport};
use crate::{Error, Result};
use base64::Engine;
use fe2o3_amqp::Connection;
use fe2o3_amqp::connection::ConnectionHandle;
use fe2o3_amqp::sasl_profile::SaslProfile;
use secrecy::ExposeSecret;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

const PROXY_TIMEOUT: Duration = Duration::from_secs(15);

/// Ask an HTTP proxy to open a tunnel to `host:port`.
pub(crate) async fn proxy_connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    host: &str,
    port: u16,
    auth: Option<(&str, &str)>,
    timeout: Duration,
) -> Result<()> {
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some((user, pass)) = auth {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    let exchange = async {
        stream.write_all(request.as_bytes()).await.map_err(|e| Error::Network(format!("proxy write: {e}")))?;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let n = stream.read(&mut byte).await.map_err(|e| Error::Network(format!("proxy read: {e}")))?;
            if n == 0 {
                return Err(Error::Network("the proxy closed the connection during CONNECT".into()));
            }
            head.push(byte[0]);
            if head.len() > 8192 {
                return Err(Error::Network("the proxy sent an oversized response to CONNECT".into()));
            }
        }
        let text = String::from_utf8_lossy(&head);
        let status_line = text.lines().next().unwrap_or_default();
        let mut parts = status_line.split_whitespace();
        let (Some(version), Some(code)) = (parts.next(), parts.next()) else {
            return Err(Error::Network("the proxy sent an invalid response to CONNECT".into()));
        };
        if !version.starts_with("HTTP/1.") {
            return Err(Error::Network("the proxy sent an invalid response to CONNECT".into()));
        }
        match code {
            "200" => Ok(()),
            "407" => Err(Error::Network(
                "the proxy requires authentication (407); put credentials in the proxy URL (http://user:pass@host:port)".into(),
            )),
            other => Err(Error::Network(format!("the proxy refused the tunnel to {host}:{port} (HTTP {other})"))),
        }
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| Error::Timeout(format!("the proxy did not answer CONNECT within {}s", timeout.as_secs())))?
}

/// Open the AMQP connection for `ns` using its transport.
pub(crate) async fn open_connection(ns: &Namespace, container_id: String) -> Result<ConnectionHandle<()>> {
    let builder = Connection::builder()
        .container_id(container_id)
        .hostname(Some(ns.host()))
        .sasl_profile(SaslProfile::Anonymous)
        .max_frame_size(65_536u32)
        .idle_time_out(60_000u32);
    match ns.transport() {
        Transport::Amqp => {
            let url = ns.amqp_url();
            let builder = if ns.is_emulator() { builder } else { builder.alt_tls_establishment(true) };
            tokio::time::timeout(SETUP_TIMEOUT, builder.open(url.as_str()))
                .await
                .map_err(|_| Error::Timeout(format!("connecting to {url} timed out")))?
                .map_err(|e| Error::Network(format!("cannot connect to {url}: {e}")))
        }
        Transport::WebSockets => {
            let ws_url = format!("wss://{}/$servicebus/websocket/", ns.host());
            let connect = async {
                let ws = match ns.proxy() {
                    None => fe2o3_amqp_ws::WebSocketStream::connect(ws_url.as_str())
                        .await
                        .map_err(|e| Error::Network(format!("cannot open {ws_url}: {e}")))?,
                    Some(proxy) => {
                        let mut tcp = TcpStream::connect((proxy.host(), proxy.port()))
                            .await
                            .map_err(|e| Error::Network(format!("cannot reach proxy {}:{}: {e}", proxy.host(), proxy.port())))?;
                        let auth = proxy.basic_auth();
                        let auth_ref = auth.as_ref().map(|(u, p)| (u.as_str(), p.expose_secret()));
                        proxy_connect(&mut tcp, ns.host(), 443, auth_ref, PROXY_TIMEOUT).await?;
                        fe2o3_amqp_ws::WebSocketStream::connect_tls_with_stream(ws_url.as_str(), tcp)
                            .await
                            .map_err(|e| Error::Network(format!("cannot open {ws_url} through the proxy: {e}")))?
                    }
                };
                builder
                    .open_with_stream(ws)
                    .await
                    .map_err(|e| Error::Network(format!("AMQP over WebSockets to {}: {e}", ns.host())))
            };
            tokio::time::timeout(SETUP_TIMEOUT, connect)
                .await
                .map_err(|_| Error::Timeout(format!("connecting to {ws_url} timed out")))?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn proxy_replying(reply: &'static [u8]) -> (TcpStream, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 1024];
            let n = s.read(&mut buf).await.unwrap();
            if !reply.is_empty() {
                s.write_all(reply).await.unwrap();
            } else {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            buf.truncate(n);
            buf
        });
        (TcpStream::connect(addr).await.unwrap(), server)
    }

    #[tokio::test]
    async fn proxy_200_opens_tunnel_and_sends_auth() {
        let (mut s, server) = proxy_replying(b"HTTP/1.1 200 Connection established\r\n\r\n").await;
        proxy_connect(&mut s, "ns.servicebus.windows.net", 443, Some(("u", "p")), Duration::from_secs(2)).await.unwrap();
        let req = String::from_utf8(server.await.unwrap()).unwrap();
        assert!(req.starts_with("CONNECT ns.servicebus.windows.net:443 HTTP/1.1\r\n"));
        assert!(req.contains("Proxy-Authorization: Basic dTpw\r\n"));
    }

    #[tokio::test]
    async fn proxy_407_is_named() {
        let (mut s, _server) = proxy_replying(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await;
        let e = proxy_connect(&mut s, "h", 443, None, Duration::from_secs(2)).await.unwrap_err();
        assert!(matches!(&e, Error::Network(m) if m.contains("407")), "{e}");
    }

    #[tokio::test]
    async fn proxy_garbage_is_network_error() {
        let (mut s, _server) = proxy_replying(b"SSH-2.0-OpenSSH\r\n\r\n").await;
        assert!(matches!(proxy_connect(&mut s, "h", 443, None, Duration::from_secs(2)).await, Err(Error::Network(_))));
    }

    #[tokio::test]
    async fn proxy_timeout() {
        let (mut s, _server) = proxy_replying(b"").await;
        let e = proxy_connect(&mut s, "h", 443, None, Duration::from_millis(200)).await.unwrap_err();
        assert!(matches!(e, Error::Timeout(_)));
    }
}
```

The `fe2o3-amqp-ws` 0.18 API may differ from the code above: `connect` may return a tuple, and `connect_tls_with_stream` may take a different argument order. Check `~/.cargo/registry/src/*/fe2o3-amqp-ws-0.18.*/src/` and adapt, keeping the semantics. Research (2026-10-13) found that fe2o3-amqp-ws hard-codes the subprotocol `"amqp"`. Service Bus is expected to accept it, and Task 15 verifies this live.

- [ ] **Step 2: Add `Transport` and `ProxyConfig` to `namespace.rs`** (with tests)

```rust
/// How the AMQP connection is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// AMQP over TLS on 5671 (plain 5672 on the emulator).
    #[default]
    Amqp,
    /// AMQP over secure WebSockets on 443.
    WebSockets,
}

impl Transport {
    /// Parse `amqp` or `websockets`.
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "amqp" => Ok(Transport::Amqp),
            "websockets" | "ws" | "wss" => Ok(Transport::WebSockets),
            other => Err(Error::Invalid(format!("unknown transport `{other}`; use amqp or websockets"))),
        }
    }
}

/// An HTTP proxy for WebSockets and HTTPS calls.
#[derive(Clone)]
pub struct ProxyConfig {
    host: String,
    port: u16,
    user: Option<String>,
    password: Option<secrecy::SecretString>,
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("auth", &self.user.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl ProxyConfig {
    /// Parse `http://[user:pass@]host:port`.
    pub fn parse(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url).map_err(|_| Error::Invalid("proxy must look like http://host:port".into()))?;
        if parsed.scheme() != "http" {
            return Err(Error::Invalid("only http:// proxies are supported (the tunnel itself is TLS)".into()));
        }
        let host = parsed.host_str().filter(|h| !h.is_empty()).ok_or_else(|| Error::Invalid("proxy URL has no host".into()))?;
        let port = parsed.port_or_known_default().unwrap_or(80);
        let user = (!parsed.username().is_empty()).then(|| {
            percent_encoding::percent_decode_str(parsed.username()).decode_utf8_lossy().into_owned()
        });
        let password = parsed
            .password()
            .map(|p| secrecy::SecretString::from(percent_encoding::percent_decode_str(p).decode_utf8_lossy().into_owned()));
        Ok(Self { host: host.to_string(), port, user, password })
    }

    /// `HTTPS_PROXY` / `https_proxy`, if set and valid.
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")).ok()?;
        match Self::parse(&value) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!("ignoring HTTPS_PROXY: {e}");
                None
            }
        }
    }

    /// Proxy host.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Proxy port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Basic-auth credentials, if any.
    pub fn basic_auth(&self) -> Option<(String, secrecy::SecretString)> {
        Some((self.user.clone()?, self.password.clone().unwrap_or_else(|| secrecy::SecretString::from(""))))
    }

    /// URL for reqwest (contains credentials — never log it).
    pub fn url_for_reqwest(&self) -> String {
        use secrecy::ExposeSecret;
        let enc = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
        match (&self.user, &self.password) {
            (Some(u), Some(p)) => format!("http://{}:{}@{}:{}", enc(u), enc(p.expose_secret()), self.host, self.port),
            (Some(u), None) => format!("http://{}@{}:{}", enc(u), self.host, self.port),
            _ => format!("http://{}:{}", self.host, self.port),
        }
    }
}
```

On `Namespace`:
- Add the fields `transport: Transport` and `proxy: Option<ProxyConfig>`. Initialise them to `Transport::Amqp` / `None` in both constructors.
- Add `transport` to the `Debug` output, plus `.field("proxy", &self.proxy)`.
- Add:

```rust
    /// Choose the transport. WebSockets are not available on the emulator.
    pub fn with_transport(mut self, transport: Transport) -> Result<Self> {
        if self.is_emulator() && transport == Transport::WebSockets {
            return Err(Error::Unsupported { capability: "AMQP over WebSockets on the Service Bus Emulator".into() });
        }
        if self.proxy.is_some() && transport == Transport::Amqp {
            return Err(Error::Invalid("a proxy requires the websockets transport".into()));
        }
        self.transport = transport;
        Ok(self)
    }

    /// Route through an HTTP proxy (forces WebSockets).
    pub fn with_proxy(mut self, proxy: Option<ProxyConfig>) -> Result<Self> {
        if let Some(p) = proxy {
            if self.is_emulator() {
                return Err(Error::Unsupported { capability: "a proxy with the Service Bus Emulator".into() });
            }
            self.proxy = Some(p);
            self.transport = Transport::WebSockets;
        }
        Ok(self)
    }

    /// Transport in use.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Proxy, if any.
    pub fn proxy(&self) -> Option<&ProxyConfig> {
        self.proxy.as_ref()
    }
```

`namespace.rs` tests to add:

```rust
    #[test]
    fn emulator_rejects_websockets_and_proxy() {
        let ns = emulator_ns();
        assert!(matches!(ns.clone().with_transport(Transport::WebSockets), Err(Error::Unsupported { .. })));
        let p = ProxyConfig::parse("http://proxy:3128").unwrap();
        assert!(matches!(ns.with_proxy(Some(p)), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn proxy_forces_websockets_and_redacts() {
        let cs = ConnectionString::parse("Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v").unwrap();
        let p = ProxyConfig::parse("http://alice:s3cr%40t@proxy.corp:8080").unwrap();
        assert_eq!(p.basic_auth().unwrap().0, "alice");
        let ns = Namespace::from_connection_string(&cs).unwrap().with_proxy(Some(p)).unwrap();
        assert_eq!(ns.transport(), Transport::WebSockets);
        let dbg = format!("{ns:?}");
        assert!(!dbg.contains("s3cr"), "{dbg}");
        assert!(ProxyConfig::parse("https://proxy:443").is_err());
        assert!(ProxyConfig::parse("socks5://p:1").is_err());
    }
```

- [ ] **Step 3: Wire the transport into the connection and the management client**

- **`connection.rs::open`:** replace the builder/open block with:
  ```rust
  let mut connection = super::transport::open_connection(ns, format!("dispatched-{}", uuid::Uuid::new_v4().simple())).await?;
  ```
  Remove the now-unused imports.
- **`mgmt/mod.rs` `ManagementClient::new(ns)`:** when `ns.proxy()` is `Some(p)`, build the reqwest client with `.proxy(reqwest::Proxy::all(p.url_for_reqwest()).map_err(|e| Error::Invalid(format!("proxy: {e}")))?)`. Keep the existing timeouts. `new` currently returns `Self`. Change it to return `Result<Self>` only if that's necessary, and if so update `Explorer::new` to return `Result<Self>`. Otherwise use `.unwrap_or_else` and fall back to no proxy with a `tracing::warn!`. **Pick the `Result` route**, and update every caller of `Explorer::new` (CLI, app, tests) to use `?`/`unwrap`.

- [ ] **Step 4: Run tests and commit**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings && cargo clippy -p dispatched-app -- -D warnings`
Expected: PASS, with 4 transport tests and 2 namespace tests new. Clean.

```bash
cargo fmt --all && git add -A && git commit -m "Core: AMQP over WebSockets and HTTP CONNECT proxies (forces WebSockets; emulator unsupported)"
```

---

### Task 9: Approximate counts where the service reports none

**Files:**
- Modify: `crates/dispatched-core/src/mgmt/models.rs` (`EntityCounts.approximate`)
- Modify: `crates/dispatched-core/src/messaging/scan.rs` (add `count_messages`)
- Modify: `crates/dispatched-core/src/explorer.rs` (`count_by_peek`, and `list_queues`/`list_subscriptions` fill missing counts)

**Interfaces:**
- `EntityCounts` gains `#[serde(default)] pub approximate: bool`. It is `false` when counts come from the service.
- Produces `scan::StateCounts { active: u64, deferred: u64, scheduled: u64, total: u64, truncated: bool }` and `scan::count_messages(src, address, cap: u32) -> Result<StateCounts>`.
- Produces `Explorer::count_by_peek(&self, entity: &EntityPath, cap: u32) -> Result<EntityCounts>`:
  - `active` is active + deferred (Service Bus counts deferred messages as active).
  - It also fills `scheduled`, `dead_letter` and `transfer_dead_letter`.
  - `transfer` is 0, and `approximate` is true.
- `Explorer::list_queues()` and `list_subscriptions(topic)`: an entity whose `counts` is `None` gets `count_by_peek(entity, 10_000)`. If counting fails (for example a missing Listen right), `counts` stays `None`. Counting runs at most 50 entities per call. The rest keep `None` and a `tracing::info!` notes the cap.

- [ ] **Step 1: Write the count tests first** (`scan.rs` tests)

```rust
    #[tokio::test]
    async fn count_messages_tallies_states_and_respects_cap() {
        let mut src = entity(10, 5);
        src.messages[0].state = MessageState::Deferred;
        let c = count_messages(&mut src, "q", 1_000).await.unwrap();
        assert_eq!((c.active, c.deferred, c.scheduled, c.total, c.truncated), (7, 1, 2, 10, false));
        let capped = count_messages(&mut src, "q", 4).await.unwrap();
        assert_eq!(capped.total, 4);
        assert!(capped.truncated);
    }
```

- [ ] **Step 2: Implement `count_messages`** (`scan.rs`)

```rust
/// Message tallies by state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateCounts {
    /// Active.
    pub active: u64,
    /// Deferred.
    pub deferred: u64,
    /// Scheduled.
    pub scheduled: u64,
    /// All examined.
    pub total: u64,
    /// The cap was reached before the end.
    pub truncated: bool,
}

/// Count messages in `address` by peeking (at most `cap`).
pub async fn count_messages(src: &mut (impl PeekSource + ?Sized), address: &str, cap: u32) -> Result<StateCounts> {
    let mut counts = StateCounts::default();
    let mut next = 0i64;
    loop {
        if counts.total >= u64::from(cap) {
            counts.truncated = true;
            break;
        }
        let want = (u64::from(cap) - counts.total).min(u64::from(MAX_PER_REQUEST)) as u32;
        let batch = src.peek_batch(address, next, want).await?;
        if batch.is_empty() {
            break;
        }
        let mut advanced = false;
        for m in &batch {
            counts.total += 1;
            match m.state {
                crate::messaging::MessageState::Active => counts.active += 1,
                crate::messaging::MessageState::Deferred => counts.deferred += 1,
                crate::messaging::MessageState::Scheduled => counts.scheduled += 1,
            }
            if m.sequence_number >= next {
                next = m.sequence_number + 1;
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    Ok(counts)
}
```

Re-export `StateCounts` and `count_messages` from `messaging/mod.rs`.

- [ ] **Step 3: `EntityCounts.approximate` and the Explorer**

In `mgmt/models.rs`, add to `EntityCounts`:

```rust
    /// Counted by peeking (the service did not report counts); may be capped.
    #[serde(default)]
    pub approximate: bool,
```

and set `approximate: false` in `counts()`.

In `explorer.rs`, add:

```rust
    const COUNT_CAP: u32 = 10_000;
    const MAX_COUNTED_ENTITIES: usize = 50;

    /// Count messages by peeking (for namespaces such as the emulator that report no counts).
    pub async fn count_by_peek(&self, entity: &EntityPath, cap: u32) -> Result<EntityCounts> {
        let mut guard = self.amqp.lock().await;
        if guard.is_none() {
            *guard = Some(ServiceBusConnection::open(&self.ns).await?);
        }
        let conn = guard.as_mut().expect("opened above");
        let main = count_messages(conn, &entity.address(SubQueue::Active), cap).await?;
        let dlq = count_messages(conn, &entity.address(SubQueue::DeadLetter), cap).await?;
        let tdlq = count_messages(conn, &entity.address(SubQueue::TransferDeadLetter), cap).await?;
        Ok(EntityCounts {
            active: main.active + main.deferred,
            dead_letter: dlq.total,
            scheduled: main.scheduled,
            transfer: 0,
            transfer_dead_letter: tdlq.total,
            approximate: true,
        })
    }
```

The constants go in a separate `impl` block or at module level. Clippy requires associated constants to sit inside `impl Explorer`, so put them there.

Then fill missing counts in `list_queues`:

```rust
    pub async fn list_queues(&self) -> Result<Vec<QueueInfo>> {
        let mut queues = self.mgmt.list_queues().await?;
        let mut counted = 0;
        for q in queues.iter_mut().filter(|q| q.counts.is_none()) {
            if counted == Self::MAX_COUNTED_ENTITIES {
                tracing::info!("approximate counts limited to the first {} queues", Self::MAX_COUNTED_ENTITIES);
                break;
            }
            counted += 1;
            q.counts = self.count_by_peek(&EntityPath::queue(&q.name), Self::COUNT_CAP).await.ok();
        }
        Ok(queues)
    }
```

Do the same in `list_subscriptions(topic)` with `EntityPath::subscription(topic, &s.name)`. Fix the imports (`SubQueue`, `EntityCounts`, `count_messages`).

The transfer-DLQ peek can fail on entities without transfer support. If it errors with `NotFound`, treat it as 0. Use a small match:

```rust
let tdlq = match count_messages(...).await { Ok(c) => c.total, Err(Error::NotFound(_)) => 0, Err(e) => return Err(e) };
```

and adjust the struct literal accordingly.

- [ ] **Step 4: Run tests and commit**

Run: `cargo test -p dispatched-core && cargo clippy --all-targets -- -D warnings && cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings`
Expected: PASS. The existing mgmt tests stay green, because the JSON gains `approximate: false`.

```bash
cargo fmt --all && git add -A && git commit -m "Core: approximate counts by peeking where the service reports none (emulator)"
```

---
### Task 10: Saved profiles (SQLite + keychain)

**Files:**
- Create: `crates/dispatched-core/src/profiles/mod.rs`, `crates/dispatched-core/src/profiles/store.rs`
- Modify: `crates/dispatched-core/Cargo.toml` and root `Cargo.toml` (`rusqlite = { version = "0.40", features = ["bundled"] }`, and `tempfile = "3"` as a dev-dependency), `src/lib.rs` (`pub mod profiles;`)

**Interfaces:**
- Produces `profiles::ProfileAuth`, tagged `"kind"` and kebab-case:
  - `ConnectionString`
  - `Entra { sign_in_id: String, tenant: Option<String> }`
- Produces `profiles::Profile`, which is Clone, Debug and Serialize. Its fields:
  - `id`, `name`, `host` (Strings)
  - `auth: ProfileAuth`
  - `emulator: bool`, `production: bool`
  - `color: Option<String>`
  - `transport: Transport`
  - `proxy: Option<String>`, stored as `http://host:port` without credentials
  - `pinned_entities: Vec<String>`
- `Profile` constructors:
  - `Profile::connection_string(name: &str, cs: &ConnectionString) -> Self` takes the host and emulator flag from the connection string.
  - `Profile::entra(name: &str, host: &str, sign_in_id: &str, tenant: Option<&str>) -> Self`.
- Produces `profiles::secret_key(profile_id: &str) -> String`, which returns `dispatched:profile:{id}:connection-string`.
- Produces `profiles::namespace_for(profile: &Profile, secrets: &dyn CredentialStore, entra: &Arc<EntraSession>, sign_in: Option<&SignIn>) -> Result<Namespace>`, which applies the transport, then the profile proxy, then the env proxy.
- Produces `profiles::with_env_proxy(ns: Namespace) -> Result<Namespace>`. It applies `ProxyConfig::from_env()` unless the namespace is the emulator or already has a proxy.
- Produces `profiles::ProfileStore`:
  - opening: `open(path: &Path) -> Result<Self>`, `open_in_memory() -> Result<Self>`
  - reading: `list()`, `get(name)` (case-insensitive)
  - adding: `insert(&Profile)`, plus `add_connection_string(name, cs_text, secrets, production: bool) -> Result<Profile>`, which validates the string, inserts the row, then stores the secret, and rolls back on failure
  - changing: `update(&Profile)`, `delete(name, secrets) -> Result<bool>`, which also removes the secret
  - defaults: `set_default(Option<&str>)`, `default_profile()`
  - sign-ins: `save_sign_in(&SignIn)`, `sign_in(id)`, `set_current_sign_in(Option<&str>)`, `current_sign_in() -> Result<Option<SignIn>>`, `remove_sign_in(id)`
  - migration: `migrate_cli_state(path: &Path) -> Result<bool>`
- Every database error reads `profile database error at <path>: <detail> (delete the file to start over)`.

- [ ] **Step 1: Write the store tests first** (bottom of `profiles/store.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use horizon_azure_kit::secrets::MemoryStore;
    use secrecy::ExposeSecret;

    const CS: &str = "Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=TopSecretKey123=";

    #[test]
    fn names_are_case_insensitive_unique() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        store.add_connection_string("Prod", CS, &secrets, true).unwrap();
        let e = store.add_connection_string("prod", CS, &secrets, false).unwrap_err();
        assert!(e.to_string().contains("already exists"), "{e}");
        assert_eq!(store.get("PROD").unwrap().unwrap().name, "Prod");
    }

    #[test]
    fn secret_never_written_to_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dispatched.db");
        let secrets = MemoryStore::new();
        let p = {
            let store = ProfileStore::open(&path).unwrap();
            store.add_connection_string("dev", CS, &secrets, false).unwrap()
        };
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("TopSecretKey123"));
        assert_eq!(secrets.get(&secret_key(&p.id)).unwrap().unwrap().expose_secret(), CS);
    }

    #[test]
    fn delete_removes_secret() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let p = store.add_connection_string("dev", CS, &secrets, false).unwrap();
        assert!(store.delete("DEV", &secrets).unwrap());
        assert!(secrets.get(&secret_key(&p.id)).unwrap().is_none());
        assert!(store.get("dev").unwrap().is_none());
        assert!(!store.delete("dev", &secrets).unwrap());
    }

    #[test]
    fn invalid_connection_string_is_not_stored() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        assert!(store.add_connection_string("bad", "Endpoint=nope", &secrets, false).is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn default_profile_and_pins_roundtrip() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut p = store.add_connection_string("dev", CS, &secrets, false).unwrap();
        p.pinned_entities = vec!["orders".into()];
        p.transport = Transport::WebSockets;
        store.update(&p).unwrap();
        store.set_default(Some("dev")).unwrap();
        let d = store.default_profile().unwrap().unwrap();
        assert_eq!(d.pinned_entities, ["orders"]);
        assert_eq!(d.transport, Transport::WebSockets);
        store.set_default(None).unwrap();
        assert!(store.default_profile().unwrap().is_none());
    }

    #[test]
    fn migrates_cli_state_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("cli-state.json");
        std::fs::write(&state, r#"{"sign_in":{"id":"oid-1","user_principal":"a@x.io","tenant_id":"t","display_name":null,"cloud":"public"}}"#).unwrap();
        let store = ProfileStore::open_in_memory().unwrap();
        assert!(store.migrate_cli_state(&state).unwrap());
        assert!(!state.exists(), "old state file is removed");
        assert_eq!(store.current_sign_in().unwrap().unwrap().user_principal, "a@x.io");
        assert!(!store.migrate_cli_state(&state).unwrap());
    }

    #[test]
    fn corrupt_db_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dispatched.db");
        std::fs::write(&path, b"this is not sqlite").unwrap();
        let e = ProfileStore::open(&path).unwrap_err().to_string();
        assert!(e.contains("profile database error") && e.contains("delete the file"), "{e}");
    }
}
```

- [ ] **Step 2: Implement `profiles/store.rs` above the tests**

```rust
//! SQLite storage for profiles, sign-ins and settings. Secrets never go here.

use super::{Profile, ProfileAuth, secret_key};
use crate::connstr::ConnectionString;
use crate::namespace::Transport;
use crate::{Error, Result};
use horizon_azure_kit::auth::SignIn;
use horizon_azure_kit::secrets::CredentialStore;
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SCHEMA_VERSION: i64 = 1;

/// Profile database.
pub struct ProfileStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl ProfileStore {
    /// Open (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| db_err(path, e))?;
        }
        let conn = Connection::open(path).map_err(|e| db_err(path, e))?;
        let store = Self { conn: Mutex::new(conn), path: path.to_path_buf() };
        store.migrate()?;
        Ok(store)
    }

    /// In-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        let path = PathBuf::from(":memory:");
        let conn = Connection::open_in_memory().map_err(|e| db_err(&path, e))?;
        let store = Self { conn: Mutex::new(conn), path };
        store.migrate()?;
        Ok(store)
    }

    fn err(&self, e: impl std::fmt::Display) -> Error {
        db_err(&self.path, e)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| self.err(e))?;
        if version > SCHEMA_VERSION {
            return Err(self.err(format!("schema version {version} is newer than this Dispatched supports")));
        }
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE profiles (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                    host TEXT NOT NULL,
                    auth TEXT NOT NULL,
                    emulator INTEGER NOT NULL DEFAULT 0,
                    production INTEGER NOT NULL DEFAULT 0,
                    color TEXT,
                    transport TEXT NOT NULL DEFAULT 'amqp',
                    proxy TEXT,
                    pinned_entities TEXT NOT NULL DEFAULT '[]'
                );
                CREATE TABLE sign_ins (id TEXT PRIMARY KEY, record TEXT NOT NULL);
                CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                PRAGMA user_version = 1;",
            )
            .map_err(|e| self.err(e))?;
        }
        Ok(())
    }

    /// All profiles, by name.
    pub fn list(&self) -> Result<Vec<Profile>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let mut stmt = conn.prepare(&format!("{SELECT_PROFILE} ORDER BY name COLLATE NOCASE")).map_err(|e| self.err(e))?;
        let rows = stmt.query_map([], row_to_profile).map_err(|e| self.err(e))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(|e| self.err(e))
    }

    /// Profile by name (case-insensitive).
    pub fn get(&self, name: &str) -> Result<Option<Profile>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(&format!("{SELECT_PROFILE} WHERE name = ?1 COLLATE NOCASE"), params![name], row_to_profile)
            .optional()
            .map_err(|e| self.err(e))
    }

    /// Insert a new profile; the name must be unused (case-insensitive).
    pub fn insert(&self, p: &Profile) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let result = conn.execute(
            "INSERT INTO profiles (id, name, host, auth, emulator, production, color, transport, proxy, pinned_entities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                p.id, p.name, p.host, auth_json(&p.auth), p.emulator, p.production, p.color,
                transport_text(p.transport), p.proxy, serde_json::to_string(&p.pinned_entities).expect("pins serialize")
            ],
        );
        match result {
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                Err(Error::Invalid(format!("a profile named `{}` already exists", p.name)))
            }
            Err(e) => Err(self.err(e)),
            Ok(_) => Ok(()),
        }
    }

    /// Replace an existing profile (matched by id).
    pub fn update(&self, p: &Profile) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute(
            "UPDATE profiles SET name = ?2, host = ?3, auth = ?4, emulator = ?5, production = ?6, color = ?7,
             transport = ?8, proxy = ?9, pinned_entities = ?10 WHERE id = ?1",
            params![
                p.id, p.name, p.host, auth_json(&p.auth), p.emulator, p.production, p.color,
                transport_text(p.transport), p.proxy, serde_json::to_string(&p.pinned_entities).expect("pins serialize")
            ],
        )
        .map_err(|e| self.err(e))?;
        Ok(())
    }

    /// Validate `cs_text`, store a connection-string profile and its secret.
    pub fn add_connection_string(&self, name: &str, cs_text: &str, secrets: &dyn CredentialStore, production: bool) -> Result<Profile> {
        let cs = ConnectionString::parse(cs_text)?;
        let mut profile = Profile::connection_string(name, &cs);
        profile.production = production;
        self.insert(&profile)?;
        if let Err(e) = secrets.put(&secret_key(&profile.id), &SecretString::from(cs_text.to_string())) {
            let conn = self.conn.lock().expect("profile db poisoned");
            let _ = conn.execute("DELETE FROM profiles WHERE id = ?1", params![profile.id]);
            return Err(e.into());
        }
        Ok(profile)
    }

    /// Delete a profile and its secret. Returns whether it existed.
    pub fn delete(&self, name: &str, secrets: &dyn CredentialStore) -> Result<bool> {
        let Some(p) = self.get(name)? else { return Ok(false) };
        secrets.delete(&secret_key(&p.id))?;
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute("DELETE FROM profiles WHERE id = ?1", params![p.id]).map_err(|e| self.err(e))?;
        let default: Option<String> = conn
            .query_row("SELECT value FROM settings WHERE key = 'default_profile'", [], |r| r.get(0))
            .optional()
            .map_err(|e| self.err(e))?;
        if default.as_deref() == Some(p.id.as_str()) {
            conn.execute("DELETE FROM settings WHERE key = 'default_profile'", []).map_err(|e| self.err(e))?;
        }
        Ok(true)
    }

    fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        match value {
            Some(v) => conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, v],
            ),
            None => conn.execute("DELETE FROM settings WHERE key = ?1", params![key]),
        }
        .map_err(|e| self.err(e))?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
            .map_err(|e| self.err(e))
    }

    /// Set (or clear) the default profile.
    pub fn set_default(&self, name: Option<&str>) -> Result<()> {
        match name {
            None => self.set_setting("default_profile", None),
            Some(n) => {
                let p = self.get(n)?.ok_or_else(|| Error::NotFound(format!("profile `{n}`")))?;
                self.set_setting("default_profile", Some(&p.id))
            }
        }
    }

    /// The default profile, if set.
    pub fn default_profile(&self) -> Result<Option<Profile>> {
        let Some(id) = self.setting("default_profile")? else { return Ok(None) };
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(&format!("{SELECT_PROFILE} WHERE id = ?1"), params![id], row_to_profile)
            .optional()
            .map_err(|e| self.err(e))
    }

    /// Remember a sign-in (no secrets).
    pub fn save_sign_in(&self, s: &SignIn) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute(
            "INSERT INTO sign_ins (id, record) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET record = excluded.record",
            params![s.id, serde_json::to_string(s).expect("SignIn serializes")],
        )
        .map_err(|e| self.err(e))?;
        Ok(())
    }

    /// A remembered sign-in.
    pub fn sign_in(&self, id: &str) -> Result<Option<SignIn>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        let record: Option<String> = conn
            .query_row("SELECT record FROM sign_ins WHERE id = ?1", params![id], |r| r.get(0))
            .optional()
            .map_err(|e| self.err(e))?;
        record.map(|r| serde_json::from_str(&r).map_err(|e| self.err(e))).transpose()
    }

    /// Forget a sign-in record.
    pub fn remove_sign_in(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.execute("DELETE FROM sign_ins WHERE id = ?1", params![id]).map_err(|e| self.err(e))?;
        Ok(())
    }

    /// Set (or clear) the current sign-in.
    pub fn set_current_sign_in(&self, id: Option<&str>) -> Result<()> {
        self.set_setting("current_sign_in", id)
    }

    /// The current sign-in, if any.
    pub fn current_sign_in(&self) -> Result<Option<SignIn>> {
        match self.setting("current_sign_in")? {
            Some(id) => self.sign_in(&id),
            None => Ok(None),
        }
    }

    /// Import the M0 `cli-state.json` (current sign-in), then delete it.
    /// Returns whether anything was migrated.
    pub fn migrate_cli_state(&self, path: &Path) -> Result<bool> {
        #[derive(serde::Deserialize)]
        struct OldState {
            sign_in: Option<SignIn>,
        }
        let Ok(text) = std::fs::read_to_string(path) else { return Ok(false) };
        let old: OldState = match serde_json::from_str(&text) {
            Ok(s) => s,
            Err(_) => OldState { sign_in: None },
        };
        if let Some(s) = &old.sign_in {
            self.save_sign_in(s)?;
            self.set_current_sign_in(Some(&s.id))?;
        }
        let _ = std::fs::remove_file(path);
        Ok(old.sign_in.is_some())
    }
}

const SELECT_PROFILE: &str =
    "SELECT id, name, host, auth, emulator, production, color, transport, proxy, pinned_entities FROM profiles";

fn row_to_profile(r: &rusqlite::Row<'_>) -> rusqlite::Result<Profile> {
    let auth_text: String = r.get(3)?;
    let transport_text: String = r.get(7)?;
    let pins_text: String = r.get(9)?;
    Ok(Profile {
        id: r.get(0)?,
        name: r.get(1)?,
        host: r.get(2)?,
        auth: serde_json::from_str(&auth_text).unwrap_or(ProfileAuth::ConnectionString),
        emulator: r.get(4)?,
        production: r.get(5)?,
        color: r.get(6)?,
        transport: Transport::parse(&transport_text).unwrap_or_default(),
        proxy: r.get(8)?,
        pinned_entities: serde_json::from_str(&pins_text).unwrap_or_default(),
    })
}

fn auth_json(a: &ProfileAuth) -> String {
    serde_json::to_string(a).expect("ProfileAuth serializes")
}

fn transport_text(t: Transport) -> &'static str {
    match t {
        Transport::Amqp => "amqp",
        Transport::WebSockets => "websockets",
    }
}

fn db_err(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("profile database error at {}: {e} (delete the file to start over)", path.display()))
}
```

- [ ] **Step 3: Implement `profiles/mod.rs`, with tests**

```rust
//! Saved connections. Metadata in SQLite; secrets in the OS keychain.

pub mod store;

pub use store::ProfileStore;

use crate::connstr::ConnectionString;
use crate::namespace::{Namespace, ProxyConfig, Transport};
use crate::tokens::EntraSource;
use crate::{Error, Result};
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::CredentialStore;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// How a profile authenticates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProfileAuth {
    /// A connection string kept in the OS keychain.
    ConnectionString,
    /// A signed-in Entra identity.
    Entra {
        /// Sign-in id (see `SignIn::id`).
        sign_in_id: String,
        /// Tenant to request tokens in (home tenant if `None`).
        tenant: Option<String>,
    },
}

/// A saved connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
    /// Stable id.
    pub id: String,
    /// User-visible name (unique, case-insensitive).
    pub name: String,
    /// Namespace host.
    pub host: String,
    /// Authentication.
    pub auth: ProfileAuth,
    /// Service Bus Emulator.
    pub emulator: bool,
    /// Production: destructive operations need typed confirmation (M2).
    pub production: bool,
    /// Optional colour label.
    pub color: Option<String>,
    /// AMQP transport.
    pub transport: Transport,
    /// `http://host:port` (credentials only via `HTTPS_PROXY`).
    pub proxy: Option<String>,
    /// Entities to show when the identity cannot list the namespace.
    pub pinned_entities: Vec<String>,
}

impl Profile {
    fn base(name: &str, host: &str, auth: ProfileAuth, emulator: bool) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            host: host.to_ascii_lowercase(),
            auth,
            emulator,
            production: false,
            color: None,
            transport: Transport::Amqp,
            proxy: None,
            pinned_entities: Vec::new(),
        }
    }

    /// A connection-string profile (secret stored separately).
    pub fn connection_string(name: &str, cs: &ConnectionString) -> Self {
        let mut p = Self::base(name, &cs.host, ProfileAuth::ConnectionString, cs.use_development_emulator);
        if let Some(e) = &cs.entity_path {
            p.pinned_entities.push(e.clone());
        }
        p
    }

    /// An Entra profile.
    pub fn entra(name: &str, host: &str, sign_in_id: &str, tenant: Option<&str>) -> Self {
        Self::base(
            name,
            host,
            ProfileAuth::Entra { sign_in_id: sign_in_id.to_string(), tenant: tenant.map(str::to_string) },
            false,
        )
    }
}

/// Keychain key of a profile's connection string.
pub fn secret_key(profile_id: &str) -> String {
    format!("dispatched:profile:{profile_id}:connection-string")
}

/// Apply `HTTPS_PROXY` unless the namespace is the emulator or already proxied.
pub fn with_env_proxy(ns: Namespace) -> Result<Namespace> {
    if ns.is_emulator() || ns.proxy().is_some() {
        return Ok(ns);
    }
    ns.with_proxy(ProxyConfig::from_env())
}

/// Build the namespace a profile points at.
pub fn namespace_for(
    profile: &Profile,
    secrets: &dyn CredentialStore,
    entra: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> Result<Namespace> {
    let ns = match &profile.auth {
        ProfileAuth::ConnectionString => {
            let secret = secrets.get(&secret_key(&profile.id))?.ok_or_else(|| {
                Error::Invalid(format!(
                    "profile `{}` has no stored connection string; remove and add it again",
                    profile.name
                ))
            })?;
            Namespace::from_connection_string(&ConnectionString::parse(secret.expose_secret())?)?
        }
        ProfileAuth::Entra { sign_in_id, tenant } => {
            let sign_in = sign_in.filter(|s| &s.id == sign_in_id).ok_or_else(|| {
                Error::NotSignedIn(format!("profile `{}` needs the account it was created with", profile.name))
            })?;
            Namespace::new(
                profile.host.clone(),
                Arc::new(EntraSource::new(entra.clone(), sign_in.clone(), tenant.clone())),
            )
        }
    };
    let ns = ns.with_transport(profile.transport)?;
    let proxy = profile.proxy.as_deref().map(ProxyConfig::parse).transpose()?;
    with_env_proxy(ns.with_proxy(proxy)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_azure_kit::AzureCloud;
    use horizon_azure_kit::secrets::MemoryStore;

    fn session() -> Arc<EntraSession> {
        Arc::new(EntraSession::new(AzureCloud::Public, Arc::new(MemoryStore::new()), "test"))
    }

    #[test]
    fn connection_string_profile_resolves_to_its_namespace() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let cs = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";
        let p = store.add_connection_string("emu", cs, &secrets, false).unwrap();
        assert!(p.emulator);
        let ns = namespace_for(&p, &secrets, &session(), None).unwrap();
        assert!(ns.is_emulator());
        assert_eq!(ns.host(), "localhost");
    }

    #[test]
    fn entra_profile_without_its_sign_in_says_so() {
        let p = Profile::entra("cloud", "ns.servicebus.windows.net", "oid-1", None);
        let e = namespace_for(&p, &MemoryStore::new(), &session(), None).unwrap_err();
        assert!(matches!(e, Error::NotSignedIn(_)));
    }

    #[test]
    fn entity_path_becomes_a_pin() {
        let cs = ConnectionString::parse("Endpoint=sb://ns.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=orders").unwrap();
        assert_eq!(Profile::connection_string("x", &cs).pinned_entities, ["orders"]);
    }
}
```

- [ ] **Step 4: Run tests and commit**

Run: `cargo test -p dispatched-core profiles && cargo clippy -p dispatched-core --all-targets -- -D warnings`
Expected: PASS (7 store and 3 mod tests). If rusqlite 0.40 renamed `ErrorCode::ConstraintViolation` or `OptionalExtension`, adapt to its API and keep the semantics.

```bash
cargo fmt --all && git add -A && git commit -m "Core: saved profiles (SQLite metadata, keychain secrets, sign-ins, cli-state migration)"
```

---

### Task 11: Connection test report

**Files:**
- Create: `crates/dispatched-core/src/diagnostics.rs`
- Modify: `crates/dispatched-core/src/lib.rs` (`pub mod diagnostics;`)

**Interfaces:**
- Produces:
  - `diagnostics::error_kind(&Error) -> &'static str`, using the same kinds as the desktop `CommandError`: `auth`, `not_signed_in`, `forbidden`, `not_found`, `throttled`, `timeout`, `cancelled`, `unsupported`, `invalid`, `network`, `protocol`.
  - `diagnostics::Check`, tagged `"status"` and kebab-case: `Ok { detail }`, `Failed { kind, message }`, `Skipped { reason }`.
  - `diagnostics::ConnectionReport { host, emulator, transport, auth, management: Check, messaging: Check }` (Serialize).
  - `#[async_trait] diagnostics::Probe { fn namespace(&self) -> &Namespace; async fn list_queue_names(&self) -> Result<Vec<String>>; async fn peek_one(&self, entity: &EntityPath) -> Result<usize>; }`, implemented for `Explorer`.
  - `diagnostics::test_connection(probe: &(impl Probe + ?Sized), entity: Option<&EntityPath>) -> ConnectionReport`
- Messaging probe target: the given entity, else the namespace `EntityPath`, else the first listed queue, else `Skipped`.

- [ ] **Step 1: Write `diagnostics.rs` with tests**

```rust
//! "Test connection": which auth path is used and which rights work.

use crate::explorer::Explorer;
use crate::messaging::PeekOptions;
use crate::namespace::{EntityPath, Namespace, PeekView, Transport};
use crate::{Error, Result};
use async_trait::async_trait;
use serde::Serialize;

/// Stable error category (shared with the desktop app and JSON output).
pub fn error_kind(e: &Error) -> &'static str {
    match e {
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
    }
}

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Check {
    /// Worked.
    Ok {
        /// What was verified.
        detail: String,
    },
    /// Failed.
    Failed {
        /// Error category.
        kind: &'static str,
        /// Error text.
        message: String,
    },
    /// Not attempted.
    Skipped {
        /// Why.
        reason: String,
    },
}

/// Result of testing a connection.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionReport {
    /// Namespace host.
    pub host: String,
    /// Emulator.
    pub emulator: bool,
    /// Transport.
    pub transport: Transport,
    /// Auth path, e.g. "SAS key RootManageSharedAccessKey" or "Entra ID a@x.io".
    pub auth: String,
    /// Listing entities (needs Manage).
    pub management: Check,
    /// Peeking an entity (needs Listen).
    pub messaging: Check,
}

/// What `test_connection` needs (implemented by `Explorer`; faked in tests).
#[async_trait]
pub trait Probe: Sync {
    /// The namespace under test.
    fn namespace(&self) -> &Namespace;
    /// Names of queues.
    async fn list_queue_names(&self) -> Result<Vec<String>>;
    /// Peek one active message; returns how many were seen.
    async fn peek_one(&self, entity: &EntityPath) -> Result<usize>;
}

#[async_trait]
impl Probe for Explorer {
    fn namespace(&self) -> &Namespace {
        Explorer::namespace(self)
    }
    async fn list_queue_names(&self) -> Result<Vec<String>> {
        Ok(self.management_client().list_queues().await?.into_iter().map(|q| q.name).collect())
    }
    async fn peek_one(&self, entity: &EntityPath) -> Result<usize> {
        Ok(self.peek(entity, PeekView::Active, &PeekOptions::new(1)).await?.messages.len())
    }
}

fn failed(e: &Error) -> Check {
    Check::Failed { kind: error_kind(e), message: e.to_string() }
}

/// Run the checks.
pub async fn test_connection(probe: &(impl Probe + ?Sized), entity: Option<&EntityPath>) -> ConnectionReport {
    let ns = probe.namespace();
    let listed = probe.list_queue_names().await;
    let management = match &listed {
        Ok(names) => Check::Ok { detail: format!("listed {} queue(s) (Manage)", names.len()) },
        Err(e) => failed(e),
    };
    let target = entity
        .cloned()
        .or_else(|| ns.entity_path().and_then(|p| EntityPath::parse(p).ok()))
        .or_else(|| listed.as_ref().ok().and_then(|n| n.first()).map(|q| EntityPath::queue(q)));
    let messaging = match target {
        None => Check::Skipped { reason: "no entity to probe; pass one".into() },
        Some(e) => match probe.peek_one(&e).await {
            Ok(n) => Check::Ok { detail: format!("peeked {e} ({n} message(s) visible; Listen)") },
            Err(err) => failed(&err),
        },
    };
    ConnectionReport {
        host: ns.host().to_string(),
        emulator: ns.is_emulator(),
        transport: ns.transport(),
        auth: ns.tokens().describe(),
        management,
        messaging,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConnectionString;

    struct Fake {
        ns: Namespace,
        list: std::result::Result<Vec<String>, &'static str>,
        peek: std::result::Result<usize, &'static str>,
    }

    #[async_trait]
    impl Probe for Fake {
        fn namespace(&self) -> &Namespace {
            &self.ns
        }
        async fn list_queue_names(&self) -> Result<Vec<String>> {
            self.list.clone().map_err(|d| Error::Forbidden { needed: Some("Manage,EntityRead".into()), detail: d.into() })
        }
        async fn peek_one(&self, _e: &EntityPath) -> Result<usize> {
            self.peek.map_err(|d| Error::Forbidden { needed: Some("Listen".into()), detail: d.into() })
        }
    }

    fn ns(cs: &str) -> Namespace {
        Namespace::from_connection_string(&ConnectionString::parse(cs).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn full_rights_probe_first_queue() {
        let f = Fake { ns: ns("Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=Root;SharedAccessKey=k"), list: Ok(vec!["orders".into()]), peek: Ok(1) };
        let r = test_connection(&f, None).await;
        assert!(matches!(r.management, Check::Ok { .. }));
        assert!(matches!(&r.messaging, Check::Ok { detail } if detail.contains("orders")));
        assert_eq!(r.auth, "SAS key Root");
    }

    #[tokio::test]
    async fn listen_only_key_uses_entity_path() {
        let f = Fake {
            ns: ns("Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=listen;SharedAccessKey=k;EntityPath=orders"),
            list: Err("claims required"),
            peek: Ok(0),
        };
        let r = test_connection(&f, None).await;
        assert!(matches!(r.management, Check::Failed { kind: "forbidden", .. }));
        assert!(matches!(r.messaging, Check::Ok { .. }));
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["management"]["status"], "failed");
    }

    #[tokio::test]
    async fn nothing_to_probe_is_skipped() {
        let f = Fake { ns: ns("Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=s;SharedAccessKey=k"), list: Err("no"), peek: Ok(0) };
        assert!(matches!(test_connection(&f, None).await.messaging, Check::Skipped { .. }));
    }
}
```

In `explorer.rs`, add `pub fn management_client(&self) -> &ManagementClient { &self.mgmt }`. This lets the probe list without the approximate-count work.

- [ ] **Step 2: Run tests and commit**

Run: `cargo test -p dispatched-core diagnostics && cargo clippy -p dispatched-core --all-targets -- -D warnings`
Expected: PASS, 3 tests.

```bash
cargo fmt --all && git add -A && git commit -m "Core: connection test report (auth path, Manage and Listen checks)"
```

---

### Task 12: CLI — profiles, `--profile`, multi-tenant sign-in, restructure

**Files:**
- Create: `crates/dispatched-cli/src/commands/mod.rs`, `commands/auth.rs`, `commands/profile.rs`, `commands/browse.rs`, `crates/dispatched-cli/src/context.rs`
- Modify: `crates/dispatched-cli/src/main.rs` (keep only `Cli` parsing, `Command` and dispatch, `format_error`), `src/resolve.rs`, `src/output.rs`
- Delete: `crates/dispatched-cli/src/state.rs` (its functionality moves into `context.rs` + `ProfileStore`)
- Modify: `crates/dispatched-core/src/discovery.rs` (add `tenant_id: Option<String>` to `DiscoveredNamespace`, from the subscription)

**Interfaces:**
- Global flags:
  - `--connection-string` (env `DISPATCHED_CONNECTION_STRING`)
  - `--namespace` (env `DISPATCHED_NAMESPACE`)
  - `--profile` (env `DISPATCHED_PROFILE`)
  - `--format table|json`
  - `--connection-string`, `--namespace` and `--profile` are mutually exclusive (`clap` `conflicts_with_all`).
- Resolution order (`resolve.rs`): `--connection-string`, then `--namespace` (with the current sign-in), then `--profile`, then the default profile. With none of these it errors, naming all four options. Each result passes through `profiles::with_env_proxy` (the profile route already applies it).
- New commands:
  - `profile ls`, `profile show <name>`
  - `profile add <name> (--connection-string-env VAR | --connection-string-stdin | --namespace NS [--tenant T]) [--production] [--color C] [--transport amqp|websockets] [--proxy http://host:port]`
  - `profile rm <name>`, `profile default [<name>|--clear]`, `profile pin <name> <entity>`, `profile unpin <name> <entity>`
  - `profile test [<name>] [--entity E]`
  - `namespaces [--tenant T | --all-tenants]`
  - `whoami` honours `--format json`
- `context.rs`:
  - `Ctx { store: ProfileStore, secrets: Arc<OsKeychain>, session: Arc<EntraSession> }`
  - `Ctx::open() -> anyhow::Result<Ctx>` opens `{config_dir}/dispatched.db` (honouring `DISPATCHED_CONFIG_DIR`) and runs `migrate_cli_state` on `{config_dir}/cli-state.json`.
  - `Ctx::current_sign_in()` prunes stale sign-ins, as `state::prune_stale_sign_in` did.
- `output::sanitize` also maps U+200B–U+200F, U+061C and U+FEFF to a space.

- [ ] **Step 1: Write the failing tests**

`crates/dispatched-cli/src/resolve.rs` tests, replacing the existing ones:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::profiles::ProfileStore;
    use horizon_azure_kit::secrets::MemoryStore;

    fn ctx() -> (ProfileStore, MemoryStore) {
        (ProfileStore::open_in_memory().unwrap(), MemoryStore::new())
    }

    const EMU: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

    #[test]
    fn short_names_expand_to_public_cloud_host() {
        assert_eq!(namespace_host("MyNs"), "myns.servicebus.windows.net");
        assert_eq!(namespace_host("x.servicebus.chinacloudapi.cn"), "x.servicebus.chinacloudapi.cn");
    }

    #[test]
    fn connection_string_wins() {
        let (store, secrets) = ctx();
        let sel = Selection { connection_string: Some(EMU), namespace: None, profile: None };
        assert!(resolve(&sel, &store, &secrets, &test_session(), None).unwrap().is_emulator());
    }

    #[test]
    fn named_profile_then_default_profile() {
        let (store, secrets) = ctx();
        store.add_connection_string("emu", EMU, &secrets, false).unwrap();
        let sel = Selection { connection_string: None, namespace: None, profile: Some("EMU") };
        assert!(resolve(&sel, &store, &secrets, &test_session(), None).unwrap().is_emulator());
        let none = Selection { connection_string: None, namespace: None, profile: None };
        let e = resolve(&none, &store, &secrets, &test_session(), None).unwrap_err().to_string();
        assert!(e.contains("--profile") && e.contains("--connection-string") && e.contains("profile default"), "{e}");
        store.set_default(Some("emu")).unwrap();
        assert!(resolve(&none, &store, &secrets, &test_session(), None).unwrap().is_emulator());
    }

    #[test]
    fn unknown_profile_is_named() {
        let (store, secrets) = ctx();
        let sel = Selection { connection_string: None, namespace: None, profile: Some("ghost") };
        assert!(resolve(&sel, &store, &secrets, &test_session(), None).unwrap_err().to_string().contains("ghost"));
    }

    #[test]
    fn namespace_without_sign_in_explains_what_to_do() {
        let (store, secrets) = ctx();
        let sel = Selection { connection_string: None, namespace: Some("myns"), profile: None };
        assert!(resolve(&sel, &store, &secrets, &test_session(), None).unwrap_err().to_string().contains("dispatched login"));
    }
}
```

`output.rs` adds:

```rust
    #[test]
    fn invisible_formatting_characters_are_neutralised() {
        for c in ['\u{200B}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{FEFF}'] {
            assert_eq!(sanitize(&format!("a{c}b")), "a b");
        }
    }
```

- [ ] **Step 2: Implement `resolve.rs`**

```rust
//! Turn CLI flags (or saved profiles) into a `Namespace`.

use anyhow::{Context, bail};
use dispatched_core::profiles::{self, ProfileStore};
use dispatched_core::tokens::EntraSource;
use dispatched_core::{ConnectionString, Namespace};
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::CredentialStore;
use std::sync::Arc;

/// Which namespace the user selected.
pub struct Selection<'a> {
    pub connection_string: Option<&'a str>,
    pub namespace: Option<&'a str>,
    pub profile: Option<&'a str>,
}

pub fn namespace_host(input: &str) -> String {
    let input = input.trim().to_ascii_lowercase();
    if input.contains('.') { input } else { format!("{input}.servicebus.windows.net") }
}

pub fn resolve(
    sel: &Selection<'_>,
    store: &ProfileStore,
    secrets: &dyn CredentialStore,
    session: &Arc<EntraSession>,
    sign_in: Option<&SignIn>,
) -> anyhow::Result<Namespace> {
    if let Some(cs) = sel.connection_string {
        return Ok(profiles::with_env_proxy(Namespace::from_connection_string(&ConnectionString::parse(cs)?)?)?);
    }
    if let Some(ns) = sel.namespace {
        let sign_in = sign_in.cloned().context("not signed in: run `dispatched login`, or pass --connection-string or --profile")?;
        let tokens = Arc::new(EntraSource::new(session.clone(), sign_in, None));
        return Ok(profiles::with_env_proxy(Namespace::new(namespace_host(ns), tokens))?);
    }
    let profile = match sel.profile {
        Some(name) => store.get(name)?.with_context(|| format!("no profile named `{name}`; see `dispatched profile ls`"))?,
        None => match store.default_profile()? {
            Some(p) => p,
            None => bail!(
                "no namespace selected: pass --profile, --connection-string (or DISPATCHED_CONNECTION_STRING), \
                 or --namespace after `dispatched login`; or set one with `dispatched profile default <name>`"
            ),
        },
    };
    Ok(profiles::namespace_for(&profile, secrets, session, sign_in)?)
}

#[cfg(test)]
pub(crate) fn test_session() -> Arc<EntraSession> {
    Arc::new(EntraSession::new(
        horizon_azure_kit::AzureCloud::Public,
        Arc::new(horizon_azure_kit::secrets::MemoryStore::new()),
        "test",
    ))
}
```

- [ ] **Step 3: Implement `context.rs`** (replaces `state.rs`)

```rust
//! Process-wide CLI context: profile database, keychain, Entra session.

use anyhow::Context as _;
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::OsKeychain;
use std::path::PathBuf;
use std::sync::Arc;

pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
pub const KEY_PREFIX: &str = "dispatched";

pub fn config_dir() -> PathBuf {
    std::env::var_os("DISPATCHED_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("dispatched"))
}

pub struct Ctx {
    pub store: ProfileStore,
    pub secrets: Arc<OsKeychain>,
    pub session: Arc<EntraSession>,
}

impl Ctx {
    pub fn open() -> anyhow::Result<Self> {
        let dir = config_dir();
        let store = ProfileStore::open(&dir.join("dispatched.db"))?;
        store.migrate_cli_state(&dir.join("cli-state.json"))?;
        let secrets = Arc::new(OsKeychain::new(KEYCHAIN_SERVICE));
        let session = Arc::new(EntraSession::new(AzureCloud::Public, secrets.clone(), KEY_PREFIX));
        Ok(Self { store, secrets, session })
    }

    /// The current sign-in, cleared if its refresh token is gone.
    pub fn current_sign_in(&self) -> anyhow::Result<Option<SignIn>> {
        let Some(s) = self.store.current_sign_in()? else { return Ok(None) };
        if self.session.is_signed_in(&s).context("checking the stored sign-in")? {
            Ok(Some(s))
        } else {
            self.store.set_current_sign_in(None)?;
            Ok(None)
        }
    }
}
```

- [ ] **Step 4: Restructure `main.rs` and add the command modules**

**`main.rs`** keeps:
- the `Cli` struct, with global `connection_string`, `namespace` and `profile` flags (all `global = true`) and `conflicts_with_all` between them,
- the `Format` enum,
- `Command` with the subcommands `Login`, `Logout`, `Whoami`, `Namespaces { tenant: Option<String>, all_tenants: bool }`, `Ls`, `Peek`, `Export` (Task 13), and `Profile { #[command(subcommand)] cmd: ProfileCmd }`,
- `main`, `format_error` (unchanged) and a `run` that opens `Ctx` and dispatches:
  - `commands::auth::{login, logout, whoami, namespaces}`
  - `commands::profile::run`
  - `commands::browse::{ls, peek, export}`

Move the existing handler bodies into those functions without changing their behaviour, except that:
- `state::*` calls become `ctx.store` and `Ctx` calls: `save_sign_in` + `set_current_sign_in` on login, `remove_sign_in` + `set_current_sign_in(None)` on logout.
- `whoami` with `--format json` prints `{"signed_in": bool, "user": .., "tenant": ..}`.
- `namespaces`:
  - With `--tenant T`, get the ARM token with `token_for(&sign_in, Some(T), &AzureCloud::Public.arm_scope())`.
  - With `--all-tenants`, list tenants through `ArmClient::list_tenants` using the home token. For each tenant, get a token and discover. A tenant whose token fails (MFA, Conditional Access) prints `warning: skipped tenant <id>: <error>` on stderr. Merge the results.
  - The table adds a `TENANT` column. JSON is `{"namespaces": [...], "skipped": [...], "skipped_tenants": [{"tenant_id", "reason"}]}`.
  - In `crates/dispatched-core/src/discovery.rs`, add `pub tenant_id: Option<String>` to `DiscoveredNamespace`, set from `sub.tenant_id.clone()` in `map`. Update its test to assert it.

**`commands/profile.rs`:**

```rust
//! `dispatched profile …`

use crate::context::Ctx;
use crate::output::{sanitize, table};
use crate::resolve::namespace_host;
use crate::Format;
use anyhow::{Context, bail};
use clap::Subcommand;
use dispatched_core::diagnostics::{Check, test_connection};
use dispatched_core::namespace::{ProxyConfig, Transport};
use dispatched_core::profiles::{Profile, namespace_for};
use dispatched_core::{EntityPath, Explorer};
use std::io::Read;

#[derive(Subcommand)]
pub enum ProfileCmd {
    /// List saved profiles.
    Ls,
    /// Show one profile.
    Show { name: String },
    /// Save a profile.
    Add {
        name: String,
        /// Read the connection string from this environment variable.
        #[arg(long, conflicts_with_all = ["connection_string_stdin", "namespace"])]
        connection_string_env: Option<String>,
        /// Read the connection string from standard input.
        #[arg(long, conflicts_with = "namespace")]
        connection_string_stdin: bool,
        /// Namespace (name or host) used with the current Entra sign-in.
        #[arg(long)]
        namespace: Option<String>,
        /// Tenant for Entra tokens (default: the sign-in's home tenant).
        #[arg(long, requires = "namespace")]
        tenant: Option<String>,
        /// Mark as production (destructive operations need typed confirmation).
        #[arg(long)]
        production: bool,
        /// Colour label.
        #[arg(long)]
        color: Option<String>,
        /// amqp or websockets.
        #[arg(long)]
        transport: Option<String>,
        /// http://host:port (credentials only via HTTPS_PROXY).
        #[arg(long)]
        proxy: Option<String>,
    },
    /// Delete a profile and its stored secret.
    Rm { name: String },
    /// Set or clear the default profile.
    Default {
        name: Option<String>,
        #[arg(long, conflicts_with = "name")]
        clear: bool,
    },
    /// Remember an entity to show when the identity cannot list the namespace.
    Pin { name: String, entity: String },
    /// Forget a pinned entity.
    Unpin { name: String, entity: String },
    /// Check which auth path is used and which rights work.
    Test {
        name: Option<String>,
        #[arg(long)]
        entity: Option<String>,
    },
}

fn auth_label(p: &Profile) -> String {
    match &p.auth {
        dispatched_core::profiles::ProfileAuth::ConnectionString => "connection string".into(),
        dispatched_core::profiles::ProfileAuth::Entra { tenant, .. } => {
            format!("Entra{}", tenant.as_deref().map(|t| format!(" ({t})")).unwrap_or_default())
        }
    }
}

pub async fn run(ctx: &Ctx, cmd: ProfileCmd, format: Format) -> anyhow::Result<()> {
    match cmd {
        ProfileCmd::Ls => {
            let profiles = ctx.store.list()?;
            let default_id = ctx.store.default_profile()?.map(|p| p.id);
            if let Format::Json = format {
                println!("{}", serde_json::to_string_pretty(&profiles)?);
                return Ok(());
            }
            let rows: Vec<Vec<String>> = profiles
                .iter()
                .map(|p| {
                    vec![
                        format!("{}{}", if default_id.as_deref() == Some(p.id.as_str()) { "* " } else { "" }, p.name),
                        p.host.clone(),
                        auth_label(p),
                        if p.production { "PROD".into() } else { String::new() },
                        match p.transport { Transport::Amqp => "amqp".into(), Transport::WebSockets => "websockets".into() },
                    ]
                })
                .collect();
            println!("{}", table(&["PROFILE", "HOST", "AUTH", "", "TRANSPORT"], &rows));
        }
        ProfileCmd::Show { name } => {
            let p = ctx.store.get(&name)?.with_context(|| format!("no profile named `{name}`"))?;
            println!("{}", serde_json::to_string_pretty(&p)?);
        }
        ProfileCmd::Add { name, connection_string_env, connection_string_stdin, namespace, tenant, production, color, transport, proxy } => {
            let mut profile = if let Some(var) = connection_string_env {
                let cs = std::env::var(&var).with_context(|| format!("environment variable `{var}` is not set"))?;
                ctx.store.add_connection_string(&name, &cs, ctx.secrets.as_ref(), production)?
            } else if connection_string_stdin {
                let mut cs = String::new();
                std::io::stdin().read_to_string(&mut cs)?;
                ctx.store.add_connection_string(&name, cs.trim(), ctx.secrets.as_ref(), production)?
            } else if let Some(ns) = namespace {
                let sign_in = ctx.current_sign_in()?.context("not signed in: run `dispatched login` first")?;
                ctx.store.save_sign_in(&sign_in)?;
                let mut p = Profile::entra(&name, &namespace_host(&ns), &sign_in.id, tenant.as_deref());
                p.production = production;
                ctx.store.insert(&p)?;
                p
            } else {
                bail!("pass --connection-string-env VAR, --connection-string-stdin, or --namespace (after `dispatched login`)");
            };
            if let Some(t) = transport {
                profile.transport = Transport::parse(&t)?;
            }
            if let Some(p) = proxy {
                let parsed = ProxyConfig::parse(&p)?;
                if parsed.basic_auth().is_some() {
                    bail!("profiles store only http://host:port; put proxy credentials in HTTPS_PROXY");
                }
                profile.proxy = Some(format!("http://{}:{}", parsed.host(), parsed.port()));
                profile.transport = Transport::WebSockets;
            }
            profile.color = color;
            ctx.store.update(&profile)?;
            eprintln!("Saved profile `{}` ({}).", profile.name, profile.host);
        }
        ProfileCmd::Rm { name } => {
            if ctx.store.delete(&name, ctx.secrets.as_ref())? {
                eprintln!("Removed profile `{name}` and its stored secret.");
            } else {
                bail!("no profile named `{name}`");
            }
        }
        ProfileCmd::Default { name, clear } => {
            if clear {
                ctx.store.set_default(None)?;
                eprintln!("Cleared the default profile.");
            } else if let Some(n) = name {
                ctx.store.set_default(Some(&n))?;
                eprintln!("Default profile: {n}");
            } else {
                match ctx.store.default_profile()? {
                    Some(p) => println!("{}", p.name),
                    None => println!("(none)"),
                }
            }
        }
        ProfileCmd::Pin { name, entity } => {
            let mut p = ctx.store.get(&name)?.with_context(|| format!("no profile named `{name}`"))?;
            EntityPath::parse(&entity)?;
            if !p.pinned_entities.iter().any(|e| e.eq_ignore_ascii_case(&entity)) {
                p.pinned_entities.push(entity);
            }
            ctx.store.update(&p)?;
        }
        ProfileCmd::Unpin { name, entity } => {
            let mut p = ctx.store.get(&name)?.with_context(|| format!("no profile named `{name}`"))?;
            p.pinned_entities.retain(|e| !e.eq_ignore_ascii_case(&entity));
            ctx.store.update(&p)?;
        }
        ProfileCmd::Test { name, entity } => {
            let p = match name {
                Some(n) => ctx.store.get(&n)?.with_context(|| format!("no profile named `{n}`"))?,
                None => ctx.store.default_profile()?.context("no default profile; pass a name")?,
            };
            let sign_in = ctx.current_sign_in()?;
            let ns = namespace_for(&p, ctx.secrets.as_ref(), &ctx.session, sign_in.as_ref())?;
            let explorer = Explorer::new(ns)?;
            let entity = entity.or_else(|| p.pinned_entities.first().cloned()).map(|e| EntityPath::parse(&e)).transpose()?;
            let report = test_connection(&explorer, entity.as_ref()).await;
            explorer.close().await;
            if let Format::Json = format {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(());
            }
            let show = |c: &Check| match c {
                Check::Ok { detail } => format!("ok — {detail}"),
                Check::Failed { message, .. } => format!("FAILED — {}", sanitize(message)),
                Check::Skipped { reason } => format!("skipped — {reason}"),
            };
            println!("namespace   {}{}", report.host, if report.emulator { " (emulator)" } else { "" });
            println!("auth        {}", report.auth);
            println!("management  {}", show(&report.management));
            println!("messaging   {}", show(&report.messaging));
        }
    }
    Ok(())
}
```

**`commands/auth.rs`** and **`commands/browse.rs`** hold the moved handlers (`login`, `logout`, `whoami`, `namespaces`; `ls`, `peek`), using `resolve::resolve` with a `Selection` built from the global flags. **`commands/mod.rs`** contains `pub mod auth; pub mod browse; pub mod profile;`.

**`output.rs` `sanitize`:** extend the match to `'\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}' | '\u{200B}'..='\u{200F}' | '\u{061C}' | '\u{FEFF}' => ' '`.

- [ ] **Step 5: Run tests, a help smoke test, and commit**

Run: `cargo test -p dispatched -p dispatched-core && cargo clippy --all-targets -- -D warnings && cargo run -q -p dispatched -- profile --help && cargo run -q -p dispatched -- namespaces --help`
Expected: tests PASS, clean, and the help output lists the new subcommands and flags.

Then a manual check against the keychain (macOS) with a throwaway config dir:

```bash
export DISPATCHED_CONFIG_DIR=$(mktemp -d)
export EMU='Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;'
cargo run -q -p dispatched -- profile add emu --connection-string-env EMU
cargo run -q -p dispatched -- profile default emu
cargo run -q -p dispatched -- profile ls
cargo run -q -p dispatched -- profile rm emu
rm -rf "$DISPATCHED_CONFIG_DIR"; unset DISPATCHED_CONFIG_DIR EMU
```

Expected: the profile is listed with a `*`, then removed. The keychain entry for it is gone, and `security find-generic-password -s io.horizon-tech.dispatched -a "dispatched:profile:..."` finds nothing.

```bash
cargo fmt --all && git add -A && git commit -m "CLI: profiles (add/ls/rm/default/pin/test), --profile, multi-tenant namespaces, split commands"
```

---

### Task 13: CLI — peek views, filters, export, entity fallbacks, approximate counts

**Files:**
- Modify: `crates/dispatched-cli/src/commands/browse.rs`, `crates/dispatched-cli/src/main.rs` (the `Peek` and `Export` args)

**Interfaces:**
- `peek [ENTITY] [--view active|scheduled|deferred|dlq|transfer-dlq] [--dlq] [--transfer-dlq] [--top N=20] [--from-seq N=0] [--scan-limit N=10000] [--filter EXPR] [--body-contains TEXT] [--body-regex RE] [--body-json PATH [--body-json-eq VALUE]]`
  - `--dlq` and `--transfer-dlq` stay as aliases. They conflict with `--view`.
  - `ENTITY` defaults to the namespace `EntityPath`, then to the profile's single pinned entity. Otherwise it errors with `entity required: pass <queue> or <topic>/subscriptions/<sub>`.
  - Table output adds a `STATE` column for the scheduled and deferred views.
  - When a page is not exhausted, stderr gets `(more: continue with --from-seq <n>)`.
  - JSON output is `{"messages": [ExportedMessage...], "scanned": n, "next_sequence": n|null}`.
- `export [ENTITY] [same view/filter flags] [--top N=1000] --out PATH|- [--format json|ndjson] [--force]`
  - It refuses to overwrite an existing file without `--force`.
  - It pages until `--top` matches are written or the entity is exhausted. Each page's `next_sequence` feeds the next request.
  - It prints `Exported N message(s) to PATH` to stderr.
- `ls`:
  - Counts with `approximate: true` render as `~N`. A footnote line on stderr reads `~ counted by peeking (the service reports no counts)`.
  - On `Error::Forbidden` from listing, it falls back to known entities: the namespace `EntityPath`, plus the profile's pins when a profile is used. These print with a note `listing needs Manage,EntityRead; showing known entities` and counts `?`. With no known entities, the original error is returned.

- [ ] **Step 1: Write the filter-building helper with tests** (in `browse.rs`)

```rust
pub struct FilterArgs<'a> {
    pub filter: Option<&'a str>,
    pub body_contains: Option<&'a str>,
    pub body_regex: Option<&'a str>,
    pub body_json: Option<&'a str>,
    pub body_json_eq: Option<&'a str>,
}

pub fn build_filter(a: &FilterArgs<'_>) -> anyhow::Result<MessageFilter> {
    let mut f = MessageFilter::default();
    if let Some(expr) = a.filter {
        f.property = Some(PropertyFilter::parse(expr)?);
    }
    if let Some(t) = a.body_contains {
        f.body.push(BodyFilter::contains(t));
    }
    if let Some(r) = a.body_regex {
        f.body.push(BodyFilter::regex(r)?);
    }
    match (a.body_json, a.body_json_eq) {
        (Some(p), eq) => f.body.push(BodyFilter::json(p, eq)?),
        (None, Some(_)) => anyhow::bail!("--body-json-eq needs --body-json PATH"),
        (None, None) => {}
    }
    Ok(f)
}

pub fn pick_view(view: Option<&str>, dlq: bool, transfer_dlq: bool) -> anyhow::Result<PeekView> {
    Ok(match (view, dlq, transfer_dlq) {
        (Some(v), false, false) => PeekView::parse(v)?,
        (None, true, false) => PeekView::DeadLetter,
        (None, false, true) => PeekView::TransferDeadLetter,
        (None, false, false) => PeekView::Active,
        _ => anyhow::bail!("use only one of --view, --dlq, --transfer-dlq"),
    })
}

pub fn default_entity(explicit: Option<&str>, ns: &Namespace, pins: &[String]) -> anyhow::Result<EntityPath> {
    if let Some(e) = explicit {
        return Ok(EntityPath::parse(e)?);
    }
    if let Some(e) = ns.entity_path() {
        return Ok(EntityPath::parse(e)?);
    }
    if let [only] = pins {
        return Ok(EntityPath::parse(only)?);
    }
    anyhow::bail!("entity required: pass <queue> or <topic>/subscriptions/<sub>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_flags_combine() {
        let f = build_filter(&FilterArgs {
            filter: Some("sys.DeliveryCount > 1"),
            body_contains: Some("x"),
            body_regex: None,
            body_json: Some("$.a"),
            body_json_eq: Some("1"),
        })
        .unwrap();
        assert!(f.property.is_some());
        assert_eq!(f.body.len(), 2);
        assert!(build_filter(&FilterArgs { filter: None, body_contains: None, body_regex: None, body_json: None, body_json_eq: Some("1") }).is_err());
        assert!(build_filter(&FilterArgs { filter: Some("a ="), body_contains: None, body_regex: None, body_json: None, body_json_eq: None }).unwrap_err().to_string().contains("column"));
    }

    #[test]
    fn views_and_aliases() {
        assert_eq!(pick_view(Some("scheduled"), false, false).unwrap(), PeekView::Scheduled);
        assert_eq!(pick_view(None, true, false).unwrap(), PeekView::DeadLetter);
        assert!(pick_view(Some("active"), true, false).is_err());
    }

    #[test]
    fn entity_defaults() {
        let cs = dispatched_core::ConnectionString::parse("Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v;EntityPath=orders").unwrap();
        let ns = Namespace::from_connection_string(&cs).unwrap();
        assert_eq!(default_entity(None, &ns, &[]).unwrap().base(), "orders");
        let cs2 = dispatched_core::ConnectionString::parse("Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=v").unwrap();
        let ns2 = Namespace::from_connection_string(&cs2).unwrap();
        assert_eq!(default_entity(None, &ns2, &["events/subscriptions/all".into()]).unwrap().base(), "events/Subscriptions/all");
        assert!(default_entity(None, &ns2, &[]).unwrap_err().to_string().contains("entity required"));
    }
}
```

- [ ] **Step 2: Implement `peek`, `export` and the `ls` changes in `browse.rs`**

The resolution helpers return the namespace plus the profile pins. `resolve` returns a `Namespace`. For pins, `browse.rs` looks up the profile itself when `--profile` or the default profile was the source:

```rust
fn pins_for(ctx: &Ctx, sel: &Selection<'_>) -> anyhow::Result<Vec<String>> {
    if sel.connection_string.is_some() || sel.namespace.is_some() {
        return Ok(Vec::new());
    }
    let p = match sel.profile {
        Some(n) => ctx.store.get(n)?,
        None => ctx.store.default_profile()?,
    };
    Ok(p.map(|p| p.pinned_entities).unwrap_or_default())
}
```

`peek`:
1. Resolve `ns` and `pins`.
2. `let explorer = Explorer::new(ns)?;`
3. `let entity = default_entity(arg, explorer.namespace(), &pins)?;`
4. `let opts = PeekOptions::new(top).from_sequence(from_seq).scan_limit(scan_limit).filter(build_filter(..)?);`
5. `let page = explorer.peek(&entity, view, &opts).await;`, then `explorer.close().await;`, then `let page = page?;`
6. Print:
   - **JSON:** `serde_json::json!({ "messages": page.messages.iter().map(ExportedMessage::new).collect::<Vec<_>>(), "scanned": page.scanned, "next_sequence": page.next_sequence })`.
   - **Table:** the existing columns, plus `STATE` when `view` is `Scheduled` or `Deferred`, plus `REASON` for the two DLQ views. If `page.next_sequence.is_some() && !page.exhausted`, print `eprintln!("(more: continue with --from-seq {n}; scanned {s})")`. When there are no messages, print `(no messages)` on stderr, as before.

`export`:
1. Open the output. For `-`, use `std::io::stdout().lock()`. Otherwise, if the path exists and `!force`, `bail!("{path} exists; pass --force to overwrite")`, then use `std::fs::File::create(path)?`.
2. Loop:
   ```rust
   let mut all = Vec::new();
   let mut from = from_seq;
   loop {
       let page = explorer.peek(&entity, view, &opts.clone().from_sequence(from)).await?;
       all.extend(page.messages);
       if all.len() as u32 >= top || page.exhausted {
           break;
       }
       match page.next_sequence {
           Some(n) => from = n,
           None => break,
       }
   }
   all.truncate(top as usize);
   ```
   with `opts` max set to `top.min(1000)`.
3. Call `write_messages(writer, &all, format)`, close the explorer, and print `Exported {n} message(s) to {path}` to stderr. Use `stdout` in the message when the output is `-`.

`ls`:
- Match on `explorer.list_queues().await`:
  - `Err(dispatched_core::Error::Forbidden { .. })` when the known entities (`ns.entity_path()` plus pins) are non-empty: print the note and render those entities with `?` counts.
  - Any other `Err(e)`: return it.
- Cell rendering: `fn count_cell(c: Option<u64>, approximate: bool) -> String` returns `~N` when approximate. Track whether any `~` was printed, and if so print the footnote on stderr.

- [ ] **Step 3: Run tests and commit**

Run: `cargo test -p dispatched && cargo clippy --all-targets -- -D warnings && cargo run -q -p dispatched -- peek --help && cargo run -q -p dispatched -- export --help`
Expected: PASS (3 new tests), clean, and help lists the flags.

```bash
cargo fmt --all && git add -A && git commit -m "CLI: peek views and filters, export, entity defaults, ls fallbacks and approximate counts"
```

---

### Task 14: Emulator coverage for M1a, plus harness and CI updates

**Files:**
- Modify: `crates/dispatched-core/src/messaging/testing.rs` (add `send_scheduled`, `defer_next`, and timeouts on every network step)
- Create: `crates/dispatched-core/tests/emulator_m1.rs`
- Modify: `crates/dispatched-core/Cargo.toml` (`[[test]] name = "emulator_m1"`, `required-features = ["test-support"]`)
- Modify: `testing/emulator/docker-compose.arm64.yml` (pin `mcr.microsoft.com/azure-sql-edge:1.0.7`)
- Modify: `.github/workflows/ci.yml` (the emulator job runs both tests and requires two `test result: ok. 1 passed` lines)

**Interfaces:**
- Produces in `testing`:
  - `send_scheduled(conn, address, body: &str, props: &[(&str, &str)], enqueue_at: OffsetDateTime) -> Result<()>`. It sets the message annotation `x-opt-scheduled-enqueue-time`.
  - `defer_next(conn, address, wait: Duration) -> Result<bool>`. It receives in peek-lock and settles with `Modified { delivery_failed: None, undeliverable_here: Some(true), message_annotations: None }`, which is a defer.
  - Every attach, send, receive-settle and close in `testing.rs` is wrapped in `tokio::time::timeout(Duration::from_secs(30), …)`, mapped to `Error::Timeout`.

- [ ] **Step 1: Add the helpers**

`send_scheduled` builds its message like `send_text`, adding:

```rust
.message_annotations(
    MessageAnnotations::builder()
        .insert("x-opt-scheduled-enqueue-time", Value::Timestamp(Timestamp::from(enqueue_at)))
        .build(),
)
```

`defer_next` is like `dead_letter_next` but settles with:

```rust
receiver.modify(&delivery, Modified { delivery_failed: None, undeliverable_here: Some(true), message_annotations: None })
```

Add the imports for `MessageAnnotations`, `Modified` and `Timestamp`. Wrap each awaited fe2o3 call in the 30-second timeout.

- [ ] **Step 2: Write `tests/emulator_m1.rs`**

```rust
//! M1a emulator coverage: views, filters, export, approximate counts.
//! Run: `testing/emulator/emulator.sh up && DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m1 -- --nocapture`

use dispatched_core::filter::{BodyFilter, MessageFilter, PropertyFilter};
use dispatched_core::messaging::{ServiceBusConnection, testing};
use dispatched_core::{ConnectionString, EntityPath, ExportFormat, Explorer, MessageState, Namespace, PeekOptions, PeekView, write_messages};
use std::time::Duration;

const EMULATOR: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

fn enabled() -> bool {
    std::env::var("DISPATCHED_EMULATOR").as_deref() == Ok("1")
}

#[tokio::test(flavor = "multi_thread")]
async fn m1a_views_filters_export_counts() {
    if !enabled() {
        // CI greps for this line to fail the job if the gate was skipped.
        eprintln!("skipped: set DISPATCHED_EMULATOR=1 and start testing/emulator/emulator.sh up");
        return;
    }
    let ns = Namespace::from_connection_string(&ConnectionString::parse(EMULATOR).unwrap()).unwrap();
    let run = uuid::Uuid::new_v4().simple().to_string();
    let mut conn = ServiceBusConnection::open(&ns).await.unwrap();
    for addr in ["orders", "orders/$DeadLetterQueue"] {
        testing::drain(&mut conn, addr, Duration::from_secs(2)).await.unwrap();
    }
    // Seed: 4 plain messages i=0..3 and 1 scheduled (in 30 min). The queue was
    // drained first, so `defer_next` defers the oldest receivable one: i=0.
    for i in 0..4 {
        testing::send_text(&mut conn, "orders", &format!("{{\"run\":\"{run}\",\"i\":{i},\"kind\":\"plain\"}}"), &[("run", &run), ("i", &i.to_string())]).await.unwrap();
    }
    let later = time::OffsetDateTime::now_utc() + time::Duration::minutes(30);
    testing::send_scheduled(&mut conn, "orders", &format!("{{\"run\":\"{run}\",\"kind\":\"scheduled\"}}"), &[("run", &run)], later).await.unwrap();
    assert!(testing::defer_next(&mut conn, "orders", Duration::from_secs(10)).await.unwrap());
    conn.close().await.unwrap();

    let explorer = Explorer::new(ns).unwrap();
    let e = EntityPath::queue("orders");
    let ours = |f: &str| MessageFilter { property: Some(PropertyFilter::parse(&format!("run = '{run}'{f}")).unwrap()), body: vec![] };

    let scheduled = explorer.peek(&e, PeekView::Scheduled, &PeekOptions::new(10).filter(ours(""))).await.unwrap();
    assert_eq!(scheduled.messages.len(), 1);
    assert_eq!(scheduled.messages[0].state, MessageState::Scheduled);

    let deferred = explorer.peek(&e, PeekView::Deferred, &PeekOptions::new(10).filter(ours(""))).await.unwrap();
    assert_eq!(deferred.messages.len(), 1);
    assert_eq!(deferred.messages[0].state, MessageState::Deferred);
    assert!(deferred.messages[0].body.as_text().unwrap().contains("\"i\":0"), "the oldest message was deferred");

    let active = explorer.peek(&e, PeekView::Active, &PeekOptions::new(10).filter(ours(""))).await.unwrap();
    assert_eq!(active.messages.len(), 3, "i=1..3 stay active");
    assert!(active.messages.iter().all(|m| m.state == MessageState::Active));

    let by_prop = explorer.peek(&e, PeekView::Active, &PeekOptions::new(10).filter(ours(" AND i = 2"))).await.unwrap();
    assert_eq!(by_prop.messages.len(), 1);

    let mut by_body = ours("");
    by_body.body.push(BodyFilter::json("$.kind", Some("plain")).unwrap());
    let plain = explorer.peek(&e, PeekView::Active, &PeekOptions::new(10).filter(by_body)).await.unwrap();
    assert_eq!(plain.messages.len(), 3);

    let mut out = Vec::new();
    let n = write_messages(&mut out, &active.messages, ExportFormat::Ndjson).unwrap();
    assert_eq!(n, 3);
    assert_eq!(String::from_utf8(out).unwrap().lines().count(), 3);

    let queues = explorer.list_queues().await.unwrap();
    let orders = queues.iter().find(|q| q.name == "orders").unwrap();
    let counts = orders.counts.as_ref().expect("approximate counts on the emulator");
    assert!(counts.approximate);
    assert!(counts.active >= 4, "active includes deferred: {counts:?}");
    assert!(counts.scheduled >= 1);
    eprintln!("GATE-NOTE m1a counts for orders: {counts:?}");
    explorer.close().await;
}
```

Add `time` to `[dev-dependencies]` if the test cannot reach it transitively. `time = { workspace = true }` works.

- [ ] **Step 3: Harness and CI**

- **arm64 override:** in `testing/emulator/docker-compose.arm64.yml`, set the image to `mcr.microsoft.com/azure-sql-edge:1.0.7`.
- **CI test step:** in `.github/workflows/ci.yml`, change the emulator job's test step to run `cargo test -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1 -- --nocapture 2>&1 | tee emulator-gate.log`.
- **CI pass guard:** replace the single positive check with:
  ```bash
  if [ "$(grep -c 'test result: ok. 1 passed' emulator-gate.log)" -ne 2 ]; then
    echo "::error::Both emulator gate tests must report 'test result: ok. 1 passed'."
    exit 1
  fi
  ```
  Keep the `skipped:` guard.

- [ ] **Step 4: Run against the emulator locally**

Run (the user approved the emulator EULA in M0):

```bash
df -h /System/Volumes/Data
testing/emulator/emulator.sh up
DISPATCHED_EMULATOR=1 cargo test -p dispatched-core --features test-support --test emulator_m0 --test emulator_m1 -- --nocapture
testing/emulator/emulator.sh down
```

Expected: both PASS. Record both `GATE-NOTE` lines.

If `defer_next` fails on the emulator, record the error verbatim and stop to report BLOCKED. Do not weaken the test. The modify-with-undeliverable-here mapping is the documented AMQP defer.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && cargo clippy -p dispatched-core --all-targets --features test-support -- -D warnings && git add -A && git commit -m "Emulator coverage for views, filters, export and approximate counts; harness pins; CI runs both gates"
```

---

### Task 15: Live verification, publishing, cleanup

**Files:**
- Create: `dispatched/docs/live-verification-m1a.md`
- Modify: `dispatched/.github/workflows/ci.yml` and `release.yml` (kit pins `v0.1.0` → `v0.2.0`)

- [ ] **Step 1: Ask the user before creating Azure resources**

Ask: "To verify WebSockets, multi-tenant discovery and profiles against real Azure, I'll create a temporary resource group `rg-dispatched-m1a-test` with a Basic namespace in the Horizon subscription (az profile `horizon`), then delete it afterwards. OK?" Only on yes:
1. Run `AZURE_CONFIG_DIR=$HOME/.azure-profiles/horizon az account show --query "{tenant:tenantId, sub:name}" -o table` and confirm it is tenant `75863b0b-…`.
2. Create the resource group, a Basic namespace and the queue `orders` (`--max-delivery-count 1`).
3. Assign "Azure Service Bus Data Owner" on the namespace to the signed-in user.

- [ ] **Step 2: Live checks**

Run these with the release CLI built from `m1a`. The user is still signed in from M0. If `whoami` says otherwise, run `dispatched login`.

```bash
D=target/debug/dispatched
$D whoami --format json
$D namespaces --all-tenants
$D profile add live --namespace <ns>
$D profile test live
$D --profile live ls
$D --profile live peek orders --view scheduled
$D --profile live peek orders --filter "sys.DeliveryCount >= 0" --format json | head -20
$D --profile live export orders --out /dev/null --format ndjson
$D profile add live-ws --namespace <ns> --transport websockets
$D --profile live-ws peek orders     # AMQP over WebSockets
```

Before peeking, seed with the REST seeding script approach from M0: send 3, schedule 1 via `BrokerProperties.ScheduledEnqueueTimeUtc`, and dead-letter 1 through max delivery 1.

Expected:
- WebSockets peek works. This proves Service Bus accepts the `amqp` subprotocol. If it fails, record the exact error. The fallback is a small adapter that sends the `AMQPWSB10` subprotocol, and it becomes a follow-up task.
- `profile test` shows Manage ok and Listen ok.

- [ ] **Step 3: Write `docs/live-verification-m1a.md`**

Use the same table layout as `live-verification-m0.md`. Rows:
- WebSockets (cloud)
- proxy (`not run` unless the user has one)
- multi-tenant discovery
- profiles (add/test/default/rm, keychain entry removed)
- views (scheduled, deferred on the emulator)
- filters
- export
- approximate counts (emulator)
- token refresh: "unit-tested; not run live (needs >1 h)"

Add a short decision line and the follow-ups. Commit it.

- [ ] **Step 4: Ask before publishing**

Ask: "Merge `m1a` → `main` in both repos, tag the kit `v0.2.0`, bump Dispatched's CI/release kit pins to `v0.2.0`, and push both?" Only on yes:

```bash
cd ../horizon-azure-kit && git checkout main && git merge --ff-only m1a && git tag -a v0.2.0 -m "horizon-azure-kit v0.2.0" && git push origin main v0.2.0
cd ../dispatched && sed -i '' 's/v0\.1\.0/v0.2.0/g' .github/workflows/ci.yml .github/workflows/release.yml && git commit -qam "CI/release: pin kit v0.2.0" && git checkout main && git merge --ff-only m1a && git push origin main
```

Watch both CI runs with `gh run watch … --exit-status`. Every job, including both emulator gates, must be green.

- [ ] **Step 5: Clean up**

After the user confirms the deletion:
1. `AZURE_CONFIG_DIR=$HOME/.azure-profiles/horizon az group delete -n rg-dispatched-m1a-test --yes`. Remove the role assignment first, as in M0.
2. Run `testing/emulator/emulator.sh down`.
3. Remove the emulator, mssql and azure-sql-edge images this session pulled.
4. Delete scratch files.
5. Remove the throwaway profiles: `dispatched profile rm live` and `dispatched profile rm live-ws`.
