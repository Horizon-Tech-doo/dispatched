# M1b — Design System and App Shell: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the M0 plumbing UI with Dispatched's design system (direction A, "Workbench") and a real app shell: profiles, sign-in, connection, lazy namespace tree with counts, entity tabs with sub-tab rows, ⌘K palette, Jobs/Activity drawer, status bar, settings and the update prompt.

**Architecture:**
- **Rust:** `dispatched-core` gains the few pieces the desktop app shares with the CLI (config paths, keychain names, app settings, profile drafts, known entities). `app/src-tauri` becomes thin commands over core: each `#[tauri::command]` calls a plain `ops::*` function that tests can call directly, and engine calls are reported on an Activity event channel.
- **Frontend:** three folders with one-way dependencies. `app/src/ui/` is the design system (tokens, glyphs, restyled shadcn components, Tree, DataGrid) and imports nothing else from the app. `app/src/engine/` is the only code that talks to Tauri (typed commands, events, TanStack Query hooks, and a mock transport for tests). `app/src/shell/` composes the two into the app.
- **Design process:** impeccable sets up `PRODUCT.md` and the direction contract before the first screen; every screen task ends with an impeccable finish review against the contract and the mockup screenshots; the documenter writes `DESIGN.md` at the end.

**Tech Stack:** Rust 2024 / Tauri 2 (existing), React 19 + TypeScript + Vite 7 (existing), Tailwind CSS 4.3, shadcn/ui structure on `radix-ui` 1.8, `class-variance-authority`, `clsx` + `tailwind-merge` 3, `lucide-react` 0.577, `cmdk` 1.1, TanStack Table 8.21 / Virtual 3.14 / Query 5, `motion` 12, `react-resizable-panels` 3, `@fontsource/jetbrains-mono` 5, Vitest 3 + Testing Library + jsdom 26, ESLint 9 + typescript-eslint 8, Playwright 1.64.

**Spec:** `docs/superpowers/specs/2026-10-15-m1b-design-system-shell-design.md` (M1b). Parent: `docs/superpowers/specs/2026-10-13-dispatched-v1-design.md` §6. Visual reference: artboard "A. Workbench" in the private canvas https://claude.ai/artifact/85qsVo8NmMQagPRXus4MwX; Task 6 saves screenshots of it into `docs/design/` so reviewers can see it.

## Global Constraints

- **Repo:** `/Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched`, branch `m1b` (already created; the spec is committed on it). Never commit to `main`. The kit repo is not changed by this plan.
- **Disk hygiene (hard rule):** build only in the repo's own `target/` and `app/node_modules/`. Never set `CARGO_TARGET_DIR`, never copy the repo to build it. Run `df -h /System/Volumes/Data` before heavy builds (cargo build/test, `tauri build`, Playwright browser install) and stop if under 30 GB free. Playwright report and screenshot folders (`app/playwright-report/`, `app/test-results/`) are gitignored and deleted before the branch is reported done.
- **Toolchain:** Rust 2024, MSRV 1.88, resolver 3 (unchanged). Node 22 (CI uses `actions/setup-node@v4` with 22).
- **npm dependencies, exact major/minor lines** (`npm install --save-exact` is not required; use `^` ranges on these lines and commit `package-lock.json`):
  - runtime: `tailwindcss@^4.3.3`, `@tailwindcss/vite@^4.3.3`, `radix-ui@^1.8.1`, `class-variance-authority@^0.7.1`, `clsx@^2.1.1`, `tailwind-merge@^3.7.0`, `lucide-react@^0.577.0`, `cmdk@^1.1.1`, `@tanstack/react-table@^8.21.3`, `@tanstack/react-virtual@^3.14.19`, `@tanstack/react-query@^5.104.3`, `motion@^12.43.0`, `react-resizable-panels@^3.0.6`, `@fontsource/jetbrains-mono@^5.3.0`, `@tauri-apps/plugin-updater@^2.14.0`, `@tauri-apps/plugin-process@^2.4.0`
  - dev: `vitest@^3.2.7`, `@testing-library/react@^16.3.3`, `@testing-library/user-event@^14.6.8`, `@testing-library/jest-dom@^6.10.0`, `jsdom@^26.1.0`, `eslint@^9.39.5`, `@eslint/js@^9.39.5`, `typescript-eslint@^8.71.2`, `eslint-plugin-react-hooks@^5.2.0`, `globals@^16.5.0`, `@playwright/test@^1.64.1`, `@types/node@^22`
  - Do not add other runtime dependencies (no zustand, no router, no icon packs) without the user's yes.
- **Boundaries:** `app/src/ui/**` imports only itself and npm packages. `app/src/engine/**` imports only itself and npm packages (never `ui` or `shell`). `app/src/shell/**` may import `ui` and `engine`. `app/src/dev/**` may import everything. Only `app/src/engine/ipc.ts` (and, from Task 19, `app/src/engine/updater.ts`) import `@tauri-apps/*`. ESLint enforces these rules (Task 5).
- **Colour:** components use colour roles only (`bg-chrome`, `text-muted`, `border-line`, ...), never hex values or Tailwind palette colours (`bg-zinc-800`). Amber (`accent`) appears only on dead-letter counts above zero, PROD, and errors (always with an icon and text). Selection is `selection`, focus is a 1px `fg` outline, success has no colour.
- **Type:** UI font stack `-apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`; sizes 11/12/13/14px only (base 12px/16px); weights 400 and 600 only. JetBrains Mono (bundled via `@fontsource/jetbrains-mono`, weights 400 and 500, ligatures off) only for IDs, sequence numbers, timestamps in grids, bodies, filter expressions, the activity log and shortcuts. Every count and number column uses tabular numerals.
- **Shape and depth:** 4px grid; `--row` 22px (compact, default) or 26px (comfortable); 3px radius on controls, square panels; no shadows except `shadow-float` on popovers, menus, the palette and side sheets.
- **Motion:** only side sheets, the drawer and popovers animate, 150ms, ease-out `cubic-bezier(0.16, 1, 0.3, 1)`; nothing animates under `prefers-reduced-motion: reduce`.
- **Banned in UI:** gradients, glows, glass/backdrop blur, decorative cards, pills as default shape, eyebrow labels, decorative status dots, emoji, em dashes in UI copy, spinners in the middle of content (use skeleton rows), modal dialogs for forms (use the right-edge `Sheet`).
- **Copy:** plain product language. Errors say what happened and what to do. Buttons name their action. Count formatting carries M0's convention: `?` unknown, `~N` counted by peeking, `~N+` when that count hit its cap; thousands separators via `Intl.NumberFormat("en-US")`.
- **Secrets:** connection strings, keys and tokens never appear in command responses, events, the activity log, settings, the SQLite DB, console logs or test snapshots. They travel exactly once, from the profile sheet to `profile_save`, and go straight to the OS keychain (service `io.horizon-tech.dispatched`, key `dispatched:profile:{profile_id}:connection-string`).
- **IPC shapes:** core types cross IPC with their existing serde names (snake_case fields, e.g. `dead_letter`, `requires_session`). Top-level Tauri command arguments are camelCase in TypeScript (Tauri converts `profile_id` to `profileId`).
- **Keys:** ⌘ on macOS, Ctrl on Windows/Linux, decided by `navigator.platform`/`userAgent` containing `Mac`.
- **impeccable and taste:** every task that changes what a user sees (Tasks 7–19) runs with the `impeccable` and `design-taste-frontend` skills loaded, and ends with the Finish review procedure defined in Task 8 (screenshots via `npm run shots`, then the `impeccable-finish-reviewer` agent against the direction contract from Task 6 and `docs/design/m1b-mockup-a-*.png`). Material findings are fixed before the task is committed as done.
- **Outward-facing actions need the user's explicit yes, every time:** pushing, opening the PR, tagging, releasing. Signing in to Azure and connecting to real namespaces in Task 21 needs the user's yes and is read-only.
- **Out of scope:** everything inside an entity's sub-tabs (M1c), the Dashboard content (M1c), concurrent Entra sign-ins (M1c), anything that changes Service Bus data (M2+), the `@horizon-tech/explorer-ui` extraction (after M1).

**Deviations from the spec, decided while planning (the spec was updated to match):**
- The `settings` table already exists (M1a); core exposes it for app keys under an `app.` prefix instead of adding a migration.
- Sign-in is one long-running `sign_in` command plus `sign_in_cancel` and a `sign-in-prompt` event, instead of start/status polling, because the kit's `login_browser` / `login_device_code` are single awaited calls.
- The shadcn components are written in shadcn's file structure directly from this plan (with `components.json` committed so `npx shadcn add` keeps working later) instead of generating defaults with the CLI and then rewriting them.
- TanStack Table stays on 8.x and `react-resizable-panels` on 3.x (their newer majors changed APIs this plan relies on).

## Review Focus

1. **A profile whose secret is gone or whose sign-in expired** (keychain cleared, DB copied from another machine, refresh token revoked).
   - **Expected:** connecting fails with a sentence that names the profile and the fix ("sign in again" / "replace the connection string"), the selector stays usable, the activity log records it, and nothing panics or hangs.
   - **Tests:** Task 3 `connect_reports_missing_secret_with_the_fix`, Task 4 `connect_with_lost_sign_in_says_sign_in_again`, Task 13 `missing secret shows the fix`, Task 20 Playwright `missing secret shows the fix`.
2. **Huge or odd namespaces:** 2,000+ entities, names with dots/dashes/uppercase, topics with zero subscriptions, counts `null` (service did not report).
   - **Expected:** the tree scrolls smoothly (virtualised), filter matches case-insensitively, empty topics say "No subscriptions", unknown counts render `?` with no layout shift.
   - **Tests:** Task 10 `renders_only_visible_rows_of_5000`, Task 14 `model_handles_empty_topics_and_unknown_counts`, `filter_is_case_insensitive_and_keeps_ancestors`, Task 20 `2,000 queues stay navigable from the keyboard`.
3. **Identity that can't list the namespace** (Listen-only SAS, entity-scoped connection string, Entra without Manage).
   - **Expected:** the tree shows the known entities (EntityPath plus pins) with `?` counts and one explanatory line with the fix; with no known entities, a single empty state explains how to pin one.
   - **Tests:** Task 4 `list_entities_falls_back_to_known_entities_on_forbidden`, `forbidden_with_nothing_known_still_explains_how_to_pin`, Task 14 `limited_listing_shows_known_entities_and_the_reason`.
4. **Secrets leaking through new surfaces** (command responses, activity events, settings, errors on bad input).
   - **Expected:** a connection string or key never appears in any JSON the frontend receives, in any activity entry, or in the DB.
   - **Tests:** Task 2 `draft_secret_never_reaches_the_db`, `bad_connection_string_error_does_not_echo_it`, `draft_debug_redacts_the_connection_string`, Task 3 `save_response_and_activity_never_contain_the_secret`, Task 12 `never returns a stored connection string`.
5. **Restoring saved shell state that no longer matches reality** (a tab for an entity that was deleted, a profile that was removed, a settings value from a newer/older build that doesn't parse).
   - **Expected:** unparsable state is ignored (defaults used), tabs for missing entities open with a "This entity no longer exists" empty state and can be closed, a deleted profile's state is not restored.
   - **Tests:** Task 3 `unparsable_stored_value_reads_as_none`, `delete_profile_clears_its_secret_and_shell_state`, Task 13 `bad_persisted_state_falls_back_to_defaults`, Task 15 `tab_for_missing_entity_shows_gone_state`.

---

## File Structure

### Rust
```
crates/dispatched-core/src/
├── lib.rs                     + pub mod paths
├── paths.rs                   NEW config_dir(), KEYCHAIN_SERVICE, KEY_PREFIX (moved from the CLI)
├── namespace.rs               + known_entities() (moved from the CLI)
├── profiles/mod.rs            + namespace_host() (moved from the CLI), ProfileDraft, DraftAuth
└── profiles/store.rs          + get_by_id, app_setting/set_app_setting, save_draft, clear_app_settings_with_prefix
crates/dispatched-cli/src/
├── context.rs                 uses dispatched_core::paths
├── resolve.rs                 re-exports dispatched_core::profiles::namespace_host
└── commands/browse.rs         uses dispatched_core::namespace::known_entities
app/src-tauri/
├── Cargo.toml                 + horizon-azure-kit, secrecy, time, open; tokio features
├── tauri.conf.json            macOS overlay title bar, CSP font-src
├── capabilities/default.json  + core:window:allow-start-dragging
└── src/
    ├── lib.rs                 builder: state, plugins, command list
    ├── state.rs               AppState (store, secrets, session, active connection, sign-in task)
    ├── error.rs               CommandError (+ From<anyhow-free kit errors>)
    ├── events.rs              ActivityEntry, JobEvent, SignInPrompt, Emitter trait, TauriEmitter, RecordingEmitter (tests)
    ├── ops/mod.rs             plain async functions the commands call (tested directly)
    ├── ops/profiles.rs        list, save, delete, test
    ├── ops/account.rs         current, sign_in, cancel
    ├── ops/connection.rs      connect, disconnect
    ├── ops/tree.rs            list_entities, list_subscriptions
    ├── ops/settings.rs        get, set
    └── commands.rs            #[tauri::command] wrappers only
```

### Frontend (`app/`)
```
app/
├── package.json, package-lock.json, components.json, eslint.config.js, vitest.config.ts, playwright.config.ts
├── index.html                 root class + font preload
├── PRODUCT.md                 NEW (Task 6, impeccable)
├── DESIGN.md                  NEW (Task 21, impeccable documenter)
├── e2e/                       Playwright specs: smoke, shell flows, review screenshots
└── src/
    ├── main.tsx               mounts <App/>, installs the mock transport in mock builds
    ├── App.tsx                providers + <Shell/> (or <Catalogue/> in dev with #catalogue)
    ├── test/setup.ts          jest-dom, ResizeObserver/matchMedia/pointer stubs
    ├── ui/
    │   ├── styles.css         Tailwind import, @theme mapping, base layer, font faces
    │   ├── tokens.ts          colour roles per theme (source of truth), contrast pairs
    │   ├── theme.ts           resolveTheme, applyTheme, useSystemDark
    │   ├── contrast.ts        WCAG maths
    │   ├── lib/cn.ts, lib/format.ts, lib/platform.ts
    │   ├── glyphs.tsx         Namespace/Queue/Topic/Subscription/Rule glyphs + EntityGlyph
    │   ├── components/        button, icon-button, input (+Textarea), field, select, badge, kbd, skeleton,
    │   │                      empty-state, theme-scope, tabs, tooltip, dropdown-menu, context-menu,
    │   │                      sheet, command, resizable, segmented, label-swatch
    │   ├── tree/Tree.tsx      virtualised keyboard tree
    │   ├── grid/DataGrid.tsx  virtualised keyboard grid
    │   └── index.ts           public exports
    ├── engine/
    │   ├── types.ts           IPC types mirroring Rust
    │   ├── ipc.ts             transport (Tauri or mock), EngineError, inTauri
    │   ├── updater.ts         update check and install (Tauri updater plugin; mockable)
    │   ├── commands.ts        one typed function per command
    │   ├── events.ts          activity, job and sign-in prompt subscriptions
    │   ├── queries.ts         query keys, hooks, mutations
    │   └── mock/              fixtures.ts, transport.ts (scenarios for tests and Playwright)
    ├── shell/
    │   ├── Shell.tsx          layout (panel groups) with tree/workspace/drawer slots
    │   ├── Workbench.tsx      fills the slots
    │   ├── context.tsx, connection.tsx, queryState.ts, Hotkeys.tsx, test-utils.tsx
    │   ├── state.ts           shell reducer, persistence (per profile) and prefs
    │   ├── keys.ts            keymap hook
    │   ├── TopBar.tsx, ConnectionSelector.tsx, StatusBar.tsx, Welcome.tsx
    │   ├── tree/model.ts, tree/NamespaceTree.tsx
    │   ├── tabs/subtabs.ts, tabs/EntityTabs.tsx, tabs/EntityView.tsx, tabs/Workspace.tsx
    │   ├── palette/commands.ts, palette/CommandPalette.tsx
    │   ├── drawer/activity.ts, drawer/Drawer.tsx
    │   └── sheets/SheetHost.tsx, ProfileSheet.tsx, ProfilesSheet.tsx, SignInPanel.tsx, SettingsSheet.tsx, UpdateSheet.tsx, useUpdate.ts
    └── dev/Catalogue.tsx      every component, both themes, both densities (built up in Tasks 8–11, 18)
```
`app/src/api.ts` and the M0 screen are deleted in Task 13 (replaced by `engine/` and `shell/`).

### Task order and dependencies
1–4 Rust (core, then app ops and commands) · 5 frontend toolchain · 6 impeccable setup and mockup screenshots · 7–11 design system (tokens, core components and catalogue, overlays, Tree, DataGrid) · 12 engine · 13–19 shell (foundation, tree, tabs, palette, drawer, connection sheets, settings and updates) · 20 Playwright flows and CI · 21 live verification, DESIGN.md, PR.
Run the tasks in order. Tasks 5–11 do not need the Rust tasks, but Task 12 mirrors the command shapes from Tasks 3–4.

---
### Task 1: Core — shared paths, known entities, namespace hosts, app settings

Move the pieces the desktop app needs out of the CLI into `dispatched-core`, and expose app settings on the existing `settings` table.

**Files:**
- Create: `crates/dispatched-core/src/paths.rs`
- Modify: `crates/dispatched-core/src/lib.rs` (add `pub mod paths;`)
- Modify: `crates/dispatched-core/Cargo.toml` (add `dirs = { workspace = true }` under `[dependencies]`)
- Modify: `crates/dispatched-core/src/namespace.rs` (add `known_entities` + tests)
- Modify: `crates/dispatched-core/src/profiles/mod.rs` (add `namespace_host` + tests)
- Modify: `crates/dispatched-core/src/profiles/store.rs` (add `get_by_id`, `app_setting`, `set_app_setting`, `clear_app_settings_with_prefix` + tests)
- Modify: `crates/dispatched-cli/src/context.rs` (use core paths; drop the moved code and its test)
- Modify: `crates/dispatched-cli/src/resolve.rs` (re-export core `namespace_host`; drop the local function, keep its tests)
- Modify: `crates/dispatched-cli/src/commands/browse.rs` (re-export core `known_entities`; drop the local function and the test moved to core)

**Interfaces:**
- Produces:
  - `dispatched_core::paths::{KEYCHAIN_SERVICE: &str, KEY_PREFIX: &str, DB_FILE: &str, LEGACY_CLI_STATE: &str, config_dir() -> Result<PathBuf>, config_dir_from(Option<OsString>, Option<PathBuf>) -> Result<PathBuf>, open_store() -> Result<ProfileStore>}`
  - `dispatched_core::namespace::known_entities(entity_path: Option<&str>, pins: &[String]) -> Vec<EntityPath>`
  - `dispatched_core::profiles::namespace_host(input: &str) -> Result<String>`
  - `ProfileStore::get_by_id(&self, id: &str) -> Result<Option<Profile>>`
  - `ProfileStore::app_setting(&self, key: &str) -> Result<Option<String>>`
  - `ProfileStore::set_app_setting(&self, key: &str, value: Option<&str>) -> Result<()>`
  - `ProfileStore::clear_app_settings_with_prefix(&self, prefix: &str) -> Result<usize>`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` at the bottom of `crates/dispatched-core/src/profiles/store.rs` (create the module's `use` lines if they are missing: `use super::*; use horizon_azure_kit::secrets::MemoryStore;`):

```rust
    fn entra_profile(store: &ProfileStore, name: &str) -> Profile {
        let p = Profile::entra(name, "myns.servicebus.windows.net", "sign-in-1", None);
        store.insert(&p).unwrap();
        p
    }

    #[test]
    fn get_by_id_finds_and_misses() {
        let store = ProfileStore::open_in_memory().unwrap();
        let p = entra_profile(&store, "Logistics");
        assert_eq!(store.get_by_id(&p.id).unwrap().unwrap().name, "Logistics");
        assert!(store.get_by_id("nope").unwrap().is_none());
    }

    #[test]
    fn app_settings_round_trip_and_clear() {
        let store = ProfileStore::open_in_memory().unwrap();
        assert!(store.app_setting("ui").unwrap().is_none());
        store.set_app_setting("ui", Some(r#"{"theme":"dark"}"#)).unwrap();
        assert_eq!(store.app_setting("ui").unwrap().as_deref(), Some(r#"{"theme":"dark"}"#));
        store.set_app_setting("ui", None).unwrap();
        assert!(store.app_setting("ui").unwrap().is_none());
    }

    #[test]
    fn app_keys_cannot_reach_internal_keys() {
        let store = ProfileStore::open_in_memory().unwrap();
        let p = entra_profile(&store, "Logistics");
        store.set_default(Some("Logistics")).unwrap();
        // The internal key is invisible through the app API...
        assert!(store.app_setting("default_profile").unwrap().is_none());
        // ...and writing the same name through it does not touch the internal one.
        store.set_app_setting("default_profile", Some("x")).unwrap();
        assert_eq!(store.default_profile().unwrap().unwrap().id, p.id);
    }

    #[test]
    fn invalid_app_keys_are_rejected() {
        let store = ProfileStore::open_in_memory().unwrap();
        for bad in ["", "has space", "semi;colon", &"k".repeat(201)] {
            let e = store.set_app_setting(bad, Some("1")).unwrap_err();
            assert!(e.to_string().contains("setting key"), "{bad}: {e}");
        }
    }

    #[test]
    fn clear_app_settings_with_prefix_only_removes_matching() {
        let store = ProfileStore::open_in_memory().unwrap();
        store.set_app_setting("shell:a_1", Some("1")).unwrap();
        store.set_app_setting("shell:a_1.extra", Some("2")).unwrap();
        store.set_app_setting("shell:ab1", Some("3")).unwrap();
        store.set_app_setting("ui", Some("4")).unwrap();
        // `_` must not act as a wildcard.
        assert_eq!(store.clear_app_settings_with_prefix("shell:a_1").unwrap(), 2);
        assert!(store.app_setting("shell:a_1").unwrap().is_none());
        assert_eq!(store.app_setting("shell:ab1").unwrap().as_deref(), Some("3"));
        assert_eq!(store.app_setting("ui").unwrap().as_deref(), Some("4"));
    }
```

Add to the tests module of `crates/dispatched-core/src/profiles/mod.rs`:

```rust
    #[test]
    fn namespace_host_expands_names_and_rejects_urls() {
        assert_eq!(namespace_host("MyNs").unwrap(), "myns.servicebus.windows.net");
        assert_eq!(
            namespace_host(" x.servicebus.chinacloudapi.cn ").unwrap(),
            "x.servicebus.chinacloudapi.cn"
        );
        for bad in ["", "https://x.servicebus.windows.net", "x:5671", ".x", "x..y", "a/b"] {
            let e = namespace_host(bad).unwrap_err().to_string();
            assert!(e.contains("invalid namespace"), "{bad}: {e}");
        }
    }
```

Create `crates/dispatched-core/src/paths.rs` with only its tests for now (the module body follows in Step 3), and add `pub mod paths;` to `lib.rs` after `pub mod namespace;`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_prefers_the_override_and_never_uses_the_cwd() {
        let env = Some(OsString::from("/tmp/dispatched-test"));
        assert_eq!(
            config_dir_from(env, Some("/home/u/.config".into())).unwrap(),
            PathBuf::from("/tmp/dispatched-test")
        );
        assert_eq!(
            config_dir_from(None, Some("/home/u/.config".into())).unwrap(),
            PathBuf::from("/home/u/.config/dispatched")
        );
        assert_eq!(
            config_dir_from(Some(OsString::new()), Some("/c".into())).unwrap(),
            PathBuf::from("/c/dispatched")
        );
        let e = config_dir_from(None, None).unwrap_err().to_string();
        assert!(e.contains("DISPATCHED_CONFIG_DIR"), "{e}");
    }
}
```

Move the CLI test `known_entities_combine_entity_path_and_pins_without_duplicates` (in `crates/dispatched-cli/src/commands/browse.rs`, the `#[test]` starting near line 1399) **unchanged** into the tests module of `crates/dispatched-core/src/namespace.rs`, and delete it from `browse.rs`. Leave `fallback_lists_known_entities_with_unknown_counts` in the CLI.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && df -h /System/Volumes/Data && cargo test -p dispatched-core --lib 2>&1 | tail -20`
Expected: compile errors: `get_by_id`, `app_setting`, `set_app_setting`, `clear_app_settings_with_prefix`, `namespace_host`, `config_dir_from`, `known_entities` not found.

- [ ] **Step 3: Implement**

`crates/dispatched-core/src/paths.rs` (above the tests module):

```rust
//! Where Dispatched keeps its database and secrets. Shared by the CLI and the
//! desktop app so both see the same profiles and sign-ins.

use crate::profiles::ProfileStore;
use crate::{Error, Result};
use std::ffi::OsString;
use std::path::PathBuf;

/// OS keychain service holding profile secrets and Entra refresh tokens.
pub const KEYCHAIN_SERVICE: &str = "io.horizon-tech.dispatched";
/// Key prefix for the kit's Entra session entries in the keychain.
pub const KEY_PREFIX: &str = "dispatched";
/// Database file inside [`config_dir`].
pub const DB_FILE: &str = "dispatched.db";
/// M0's CLI state file, migrated into the database on first open.
pub const LEGACY_CLI_STATE: &str = "cli-state.json";

/// The directory holding `dispatched.db`: `DISPATCHED_CONFIG_DIR`, else the
/// platform config directory. Never falls back to the working directory.
pub fn config_dir() -> Result<PathBuf> {
    config_dir_from(std::env::var_os("DISPATCHED_CONFIG_DIR"), dirs::config_dir())
}

/// [`config_dir`] with its inputs passed in (tests).
pub fn config_dir_from(env: Option<OsString>, platform: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    platform.map(|d| d.join("dispatched")).ok_or_else(|| {
        Error::Io(
            "cannot determine the config directory (no home directory); \
             set DISPATCHED_CONFIG_DIR to a writable directory"
                .into(),
        )
    })
}

/// Open the profile database at its standard location, migrating M0's CLI
/// state file once.
pub fn open_store() -> Result<ProfileStore> {
    let dir = config_dir()?;
    let store = ProfileStore::open(&dir.join(DB_FILE))?;
    store.migrate_cli_state(&dir.join(LEGACY_CLI_STATE))?;
    Ok(store)
}
```

In `crates/dispatched-core/src/namespace.rs`, after `impl EntityPath { ... }`, add `known_entities` with the body moved verbatim from `crates/dispatched-cli/src/commands/browse.rs`:

```rust
/// The entities we can show without listing the namespace: the connection
/// string's EntityPath, then the profile's pins. Duplicates (any case) and
/// unparsable pins are dropped.
pub fn known_entities(entity_path: Option<&str>, pins: &[String]) -> Vec<EntityPath> {
    let mut out: Vec<EntityPath> = Vec::new();
    for raw in entity_path
        .into_iter()
        .chain(pins.iter().map(String::as_str))
    {
        if let Ok(e) = EntityPath::parse(raw)
            && !out.iter().any(|k| k.base().eq_ignore_ascii_case(&e.base()))
        {
            out.push(e);
        }
    }
    out
}
```

In `crates/dispatched-core/src/profiles/mod.rs`, after `secret_key`, add `namespace_host` (moved from `crates/dispatched-cli/src/resolve.rs`, now returning core's `Error::Invalid` with the same text):

```rust
/// A bare namespace name expands to the public-cloud host; a dotted input is
/// taken as the full host. Only hostname characters (letters, digits, `.`,
/// `-`) are accepted, so a URL, port or path is rejected rather than guessed at.
pub fn namespace_host(input: &str) -> Result<String> {
    let input = input.trim().to_ascii_lowercase();
    let valid = !input.is_empty()
        && !input.starts_with(['.', '-'])
        && !input.ends_with(['.', '-'])
        && !input.contains("..")
        && input
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !valid {
        return Err(Error::Invalid(
            "invalid namespace: expected a name like `myns` or a host like \
             `myns.servicebus.windows.net` (letters, digits, `.` and `-` only)"
                .into(),
        ));
    }
    Ok(if input.contains('.') {
        input
    } else {
        format!("{input}.servicebus.windows.net")
    })
}
```

In `crates/dispatched-core/src/profiles/store.rs`, add next to `get`:

```rust
    /// The profile with this id.
    pub fn get_by_id(&self, id: &str) -> Result<Option<Profile>> {
        let conn = self.conn.lock().expect("profile db poisoned");
        conn.query_row(
            &format!("{SELECT_PROFILE} WHERE id = ?1"),
            params![id],
            row_to_profile,
        )
        .optional()
        .map_err(|e| self.row_err(e))
    }
```

and after the private `setting` function:

```rust
    /// An app setting (desktop UI state). App keys live under `app.` so they
    /// can never read or overwrite the store's own keys.
    pub fn app_setting(&self, key: &str) -> Result<Option<String>> {
        self.setting(&app_key(key)?)
    }

    /// Set or clear (`None`) an app setting.
    pub fn set_app_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.set_setting(&app_key(key)?, value)
    }

    /// Remove every app setting whose key starts with `prefix`. Returns how many.
    pub fn clear_app_settings_with_prefix(&self, prefix: &str) -> Result<usize> {
        let full = app_key(prefix)?;
        let conn = self.conn.lock().expect("profile db poisoned");
        // substr instead of LIKE: `_` and `%` in keys must match literally.
        conn.execute(
            "DELETE FROM settings WHERE substr(key, 1, ?2) = ?1",
            params![full, full.chars().count() as i64],
        )
        .map_err(|e| self.err(e))
    }
```

and as a free function near the bottom of the file (outside `impl`):

```rust
const APP_KEY_PREFIX: &str = "app.";
const APP_KEY_MAX: usize = 200;

fn app_key(key: &str) -> Result<String> {
    let ok = !key.is_empty()
        && key.len() <= APP_KEY_MAX
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
    if !ok {
        return Err(Error::Invalid(format!(
            "invalid setting key (use 1-{APP_KEY_MAX} letters, digits, `.`, `_`, `:` or `-`)"
        )));
    }
    Ok(format!("{APP_KEY_PREFIX}{key}"))
}
```

In `crates/dispatched-core/Cargo.toml` `[dependencies]`, add `dirs = { workspace = true }` (alphabetical, after `base64`).

CLI changes:
- `crates/dispatched-cli/src/context.rs`: delete `KEYCHAIN_SERVICE`, `KEY_PREFIX`, `config_dir`, `config_dir_from` and the test `config_dir_prefers_the_override_and_never_uses_the_cwd`; add `pub use dispatched_core::paths::{KEYCHAIN_SERVICE, KEY_PREFIX};`; in `Ctx::open` replace the opener body with `Box::new(|| Ok(dispatched_core::paths::open_store()?))`. Remove now-unused imports (`OsString`, `PathBuf`, `anyhow::Context` if unused).
- `crates/dispatched-cli/src/resolve.rs`: delete the local `namespace_host` function and add `pub use dispatched_core::profiles::namespace_host;` at the top. Its existing tests keep calling `namespace_host(...)` and comparing `to_string()` texts, which are unchanged.
- `crates/dispatched-cli/src/commands/browse.rs`: delete the local `known_entities` function and add `pub use dispatched_core::namespace::known_entities;` with the other imports.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dispatched-core --lib && cargo test -p dispatched && cargo clippy --all-targets -- -D warnings`
Expected: all PASS, no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/dispatched-core crates/dispatched-cli Cargo.lock
git commit -m "Core: shared config paths, known entities, namespace hosts and app settings"
```

---

### Task 2: Core — profile drafts (create and edit from a form)

The profile sheet submits one draft for both create and edit. All validation and the keychain handling live in core so the app crate stays a thin wrapper.

**Files:**
- Modify: `crates/dispatched-core/src/profiles/mod.rs` (add `ProfileDraft`, `DraftAuth`)
- Modify: `crates/dispatched-core/src/profiles/store.rs` (add `save_draft` and helpers + tests)

**Interfaces:**
- Consumes: `namespace_host` (Task 1), `ProfileStore::{insert, update, get_by_id, save_sign_in}`, `Profile::{connection_string, entra}`, `secret_key`, `ConnectionString::parse`, `EntityPath::parse`.
- Produces:
  - `pub struct ProfileDraft { pub id: Option<String>, pub name: String, pub auth: DraftAuth, pub production: bool, pub color: Option<String>, pub transport: Transport, pub proxy: Option<String>, pub pinned_entities: Vec<String> }` — `serde::Deserialize`, snake_case fields, every field but `name` and `auth` defaults.
  - `pub enum DraftAuth { ConnectionString { connection_string: Option<String> }, Entra { namespace: String, tenant: Option<String> } }` — `#[serde(tag = "kind", rename_all = "kebab-case")]`, so JSON is `{"kind":"connection-string","connection_string":"..."}` or `{"kind":"entra","namespace":"myns","tenant":null}`. Its `Debug` redacts the connection string.
  - `ProfileStore::save_draft(&self, draft: &ProfileDraft, secrets: &dyn CredentialStore, current_sign_in: Option<&SignIn>) -> Result<Profile>`

Rules:
- Create (`id: None`), connection string: the string is required ("paste the namespace's connection string"); the profile is inserted, then the secret is stored; if storing fails the row is removed.
- Create, Entra: needs `current_sign_in` (else `Error::NotSignedIn("sign in with Entra ID first, then add the profile")`); host from `namespace_host`; the sign-in is saved.
- Edit (`id: Some`): missing id → `Error::NotFound`. Switching auth kind → `Error::Invalid("a profile's sign-in method can't be changed; add a new profile instead")`. A connection-string edit with `connection_string: None` (or blank) keeps the stored secret; with a value it replaces the secret and updates host and emulator flag; if the row update then fails (for example a duplicate name) the previous secret is restored.
- Both: name trimmed (store validates it); colour and proxy trimmed, blank → `None`; pins trimmed, blanks dropped, each must parse as an `EntityPath`; the connection string's `EntityPath` is merged first and duplicates (any case) are dropped; WebSockets or a proxy on an emulator profile → `Error::Unsupported { capability: "WebSockets and proxies on the Service Bus Emulator" }` before anything is written.

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/dispatched-core/src/profiles/store.rs`:

```rust
    use crate::namespace::Transport;
    use crate::profiles::{DraftAuth, ProfileDraft};
    use horizon_azure_kit::AzureCloud;
    use horizon_azure_kit::auth::SignIn;
    use horizon_azure_kit::secrets::CredentialStore;
    use secrecy::ExposeSecret;

    const CS: &str = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0LWtleQ==";
    const CS2: &str = "Endpoint=sb://other.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=bmV3LWtleQ==";
    const EMU: &str = "Endpoint=sb://localhost;SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=SAS_KEY_VALUE;UseDevelopmentEmulator=true;";

    fn cs_draft(name: &str, cs: Option<&str>) -> ProfileDraft {
        ProfileDraft {
            id: None,
            name: name.into(),
            auth: DraftAuth::ConnectionString { connection_string: cs.map(str::to_string) },
            production: false,
            color: None,
            transport: Transport::Amqp,
            proxy: None,
            pinned_entities: vec![],
        }
    }

    fn sign_in() -> SignIn {
        SignIn {
            id: "oid-1".into(),
            user_principal: "ops@contoso.example".into(),
            tenant_id: "tenant-1".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        }
    }

    fn stored(secrets: &MemoryStore, p: &Profile) -> Option<String> {
        secrets
            .get(&secret_key(&p.id))
            .unwrap()
            .map(|s| s.expose_secret().to_string())
    }

    #[test]
    fn draft_secret_never_reaches_the_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dispatched.db");
        let store = ProfileStore::open(&path).unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Logistics", Some(CS));
        d.production = true;
        d.color = Some("  amber ".into());
        let p = store.save_draft(&d, &secrets, None).unwrap();
        assert_eq!(p.host, "myns.servicebus.windows.net");
        assert!(p.production);
        assert_eq!(p.color.as_deref(), Some("amber"));
        assert_eq!(stored(&secrets, &p).as_deref(), Some(CS));
        drop(store);
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("c2VjcmV0LWtleQ"), "secret written to the DB");
    }

    #[test]
    fn create_needs_a_connection_string() {
        let store = ProfileStore::open_in_memory().unwrap();
        let e = store
            .save_draft(&cs_draft("x", Some("  ")), &MemoryStore::new(), None)
            .unwrap_err();
        assert!(e.to_string().contains("paste the namespace's connection string"), "{e}");
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn bad_connection_string_error_does_not_echo_it() {
        let store = ProfileStore::open_in_memory().unwrap();
        let bad = "Endpoint=sb://x/;SharedAccessKey=TOPSECRET;SharedAccessKeyName";
        let e = store
            .save_draft(&cs_draft("x", Some(bad)), &MemoryStore::new(), None)
            .unwrap_err();
        assert!(!e.to_string().contains("TOPSECRET"), "{e}");
    }

    #[test]
    fn edit_without_secret_keeps_it_and_with_secret_replaces_it() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let p = store.save_draft(&cs_draft("Logistics", Some(CS)), &secrets, None).unwrap();

        let mut edit = cs_draft("Logistics prod", None);
        edit.id = Some(p.id.clone());
        let p2 = store.save_draft(&edit, &secrets, None).unwrap();
        assert_eq!(p2.name, "Logistics prod");
        assert_eq!(stored(&secrets, &p2).as_deref(), Some(CS));

        let mut edit = cs_draft("Logistics prod", Some(CS2));
        edit.id = Some(p.id.clone());
        let p3 = store.save_draft(&edit, &secrets, None).unwrap();
        assert_eq!(p3.host, "other.servicebus.windows.net");
        assert_eq!(stored(&secrets, &p3).as_deref(), Some(CS2));
    }

    #[test]
    fn rename_conflict_restores_the_previous_secret() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        store.save_draft(&cs_draft("A", Some(CS)), &secrets, None).unwrap();
        let b = store.save_draft(&cs_draft("B", Some(CS)), &secrets, None).unwrap();
        let mut edit = cs_draft("a", Some(CS2));
        edit.id = Some(b.id.clone());
        let e = store.save_draft(&edit, &secrets, None).unwrap_err();
        assert!(e.to_string().contains("already exists"), "{e}");
        assert_eq!(stored(&secrets, &b).as_deref(), Some(CS));
        assert_eq!(store.get_by_id(&b.id).unwrap().unwrap().host, "myns.servicebus.windows.net");
    }

    #[test]
    fn edit_cannot_switch_auth_kind_or_target_a_missing_profile() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let p = store.save_draft(&cs_draft("A", Some(CS)), &secrets, None).unwrap();
        let mut edit = cs_draft("A", None);
        edit.id = Some(p.id.clone());
        edit.auth = DraftAuth::Entra { namespace: "myns".into(), tenant: None };
        let e = store.save_draft(&edit, &secrets, Some(&sign_in())).unwrap_err();
        assert!(e.to_string().contains("can't be changed"), "{e}");
        edit.id = Some("missing".into());
        assert!(matches!(
            store.save_draft(&edit, &secrets, Some(&sign_in())).unwrap_err(),
            Error::NotFound(_)
        ));
    }

    #[test]
    fn entra_draft_needs_a_sign_in_and_expands_the_host() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Ops", None);
        d.auth = DraftAuth::Entra { namespace: "MyNs".into(), tenant: Some(" t-2 ".into()) };
        assert!(matches!(
            store.save_draft(&d, &secrets, None).unwrap_err(),
            Error::NotSignedIn(_)
        ));
        let p = store.save_draft(&d, &secrets, Some(&sign_in())).unwrap();
        assert_eq!(p.host, "myns.servicebus.windows.net");
        assert_eq!(
            p.auth,
            ProfileAuth::Entra { sign_in_id: "oid-1".into(), tenant: Some("t-2".into()) }
        );
        assert!(store.sign_in("oid-1").unwrap().is_some());
    }

    #[test]
    fn emulator_draft_rejects_websockets_and_writes_nothing() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Local", Some(EMU));
        d.transport = Transport::WebSockets;
        assert!(matches!(
            store.save_draft(&d, &secrets, None).unwrap_err(),
            Error::Unsupported { .. }
        ));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn pins_are_validated_and_merged_with_the_entity_path() {
        let store = ProfileStore::open_in_memory().unwrap();
        let secrets = MemoryStore::new();
        let mut d = cs_draft("Scoped", Some(&format!("{CS};EntityPath=orders")));
        d.pinned_entities = vec!["ORDERS".into(), " ".into(), "events/subscriptions/audit".into()];
        let p = store.save_draft(&d, &secrets, None).unwrap();
        assert_eq!(p.pinned_entities, vec!["orders", "events/subscriptions/audit"]);
        let mut bad = cs_draft("Bad", Some(CS));
        bad.pinned_entities = vec!["a/b/c/d/e".into()];
        assert!(matches!(store.save_draft(&bad, &secrets, None).unwrap_err(), Error::Invalid(_)));
    }

    #[test]
    fn draft_debug_redacts_the_connection_string() {
        let d = cs_draft("x", Some(CS));
        assert!(!format!("{d:?}").contains("c2VjcmV0"));
    }

    #[test]
    fn draft_deserializes_from_ui_json() {
        let d: ProfileDraft = serde_json::from_str(
            r#"{"name":"Ops","auth":{"kind":"entra","namespace":"myns"},"transport":"websockets"}"#,
        )
        .unwrap();
        assert_eq!(d.transport, Transport::WebSockets);
        assert!(d.id.is_none() && !d.production && d.pinned_entities.is_empty());
    }
```

Note: if `EntityPath::parse("a/b/c/d/e")` turns out to succeed, replace that pin in the test with one `EntityPath::parse` rejects (check `namespace.rs` tests for a rejected example) — the assertion is that an unparsable pin is an `Error::Invalid`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p dispatched-core --lib profiles 2>&1 | tail -20`
Expected: compile errors for `ProfileDraft`, `DraftAuth`, `save_draft`.

- [ ] **Step 3: Implement**

In `crates/dispatched-core/src/profiles/mod.rs` (add `use serde::Deserialize;` if missing):

```rust
/// What a UI submits to create (`id: None`) or edit a profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileDraft {
    /// Existing profile id when editing.
    #[serde(default)]
    pub id: Option<String>,
    /// Display name.
    pub name: String,
    /// How it authenticates.
    pub auth: DraftAuth,
    /// Production profile.
    #[serde(default)]
    pub production: bool,
    /// Optional colour label.
    #[serde(default)]
    pub color: Option<String>,
    /// AMQP transport.
    #[serde(default)]
    pub transport: Transport,
    /// `http://host:port` proxy.
    #[serde(default)]
    pub proxy: Option<String>,
    /// Entities to show when the identity cannot list the namespace.
    #[serde(default)]
    pub pinned_entities: Vec<String>,
}

/// The authentication part of a [`ProfileDraft`].
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DraftAuth {
    /// A connection string; `None` on an edit keeps the stored one.
    ConnectionString {
        /// The connection string (a secret).
        #[serde(default)]
        connection_string: Option<String>,
    },
    /// The current Entra sign-in, for a namespace name or host.
    Entra {
        /// `myns` or `myns.servicebus.windows.net`.
        namespace: String,
        /// Tenant to request tokens in (home tenant if `None`).
        #[serde(default)]
        tenant: Option<String>,
    },
}

impl std::fmt::Debug for DraftAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftAuth::ConnectionString { connection_string } => f
                .debug_struct("ConnectionString")
                .field("connection_string", &connection_string.as_ref().map(|_| "<redacted>"))
                .finish(),
            DraftAuth::Entra { namespace, tenant } => f
                .debug_struct("Entra")
                .field("namespace", namespace)
                .field("tenant", tenant)
                .finish(),
        }
    }
}
```

In `crates/dispatched-core/src/profiles/store.rs` (add `use super::{DraftAuth, ProfileDraft, namespace_host};` and `use crate::namespace::EntityPath;` to the imports):

```rust
    /// Create or edit a profile from a UI draft (see [`ProfileDraft`]).
    pub fn save_draft(
        &self,
        draft: &ProfileDraft,
        secrets: &dyn CredentialStore,
        current_sign_in: Option<&SignIn>,
    ) -> Result<Profile> {
        let pins = clean_pins(&draft.pinned_entities)?;
        match &draft.id {
            None => self.create_from_draft(draft, pins, secrets, current_sign_in),
            Some(id) => self.edit_from_draft(id, draft, pins, secrets),
        }
    }

    fn create_from_draft(
        &self,
        draft: &ProfileDraft,
        pins: Vec<String>,
        secrets: &dyn CredentialStore,
        current_sign_in: Option<&SignIn>,
    ) -> Result<Profile> {
        match &draft.auth {
            DraftAuth::ConnectionString { connection_string } => {
                let text = non_blank(connection_string.as_deref()).ok_or_else(|| {
                    Error::Invalid("paste the namespace's connection string".into())
                })?;
                let cs = ConnectionString::parse(text)?;
                let mut p = Profile::connection_string(&draft.name, &cs);
                let from_cs = std::mem::take(&mut p.pinned_entities);
                apply_draft_options(&mut p, draft, merge_pins(from_cs, pins));
                check_emulator_options(&p)?;
                self.insert(&p)?;
                if let Err(e) = secrets.put(&secret_key(&p.id), &SecretString::from(text.to_string())) {
                    self.delete_row(&p.id);
                    return Err(e.into());
                }
                Ok(p)
            }
            DraftAuth::Entra { namespace, tenant } => {
                let sign_in = current_sign_in.ok_or_else(|| {
                    Error::NotSignedIn("sign in with Entra ID first, then add the profile".into())
                })?;
                let host = namespace_host(namespace)?;
                let mut p = Profile::entra(
                    &draft.name,
                    &host,
                    &sign_in.id,
                    non_blank(tenant.as_deref()),
                );
                apply_draft_options(&mut p, draft, pins);
                self.save_sign_in(sign_in)?;
                self.insert(&p)?;
                Ok(p)
            }
        }
    }

    fn edit_from_draft(
        &self,
        id: &str,
        draft: &ProfileDraft,
        pins: Vec<String>,
        secrets: &dyn CredentialStore,
    ) -> Result<Profile> {
        let mut p = self
            .get_by_id(id)?
            .ok_or_else(|| Error::NotFound(format!("profile {id}")))?;
        match (&draft.auth, &p.auth) {
            (DraftAuth::ConnectionString { connection_string }, ProfileAuth::ConnectionString) => {
                if let Some(text) = non_blank(connection_string.as_deref()) {
                    let cs = ConnectionString::parse(text)?;
                    p.host = cs.host.clone();
                    p.emulator = cs.use_development_emulator;
                    apply_draft_options(&mut p, draft, merge_pins(cs.entity_path.into_iter().collect(), pins));
                    check_emulator_options(&p)?;
                    let key = secret_key(&p.id);
                    let previous = secrets.get(&key)?;
                    secrets.put(&key, &SecretString::from(text.to_string()))?;
                    if let Err(e) = self.update(&p) {
                        let _ = match previous {
                            Some(old) => secrets.put(&key, &old),
                            None => secrets.delete(&key),
                        };
                        return Err(e);
                    }
                    return Ok(p);
                }
            }
            (DraftAuth::Entra { namespace, tenant }, ProfileAuth::Entra { sign_in_id, .. }) => {
                p.host = namespace_host(namespace)?;
                p.auth = ProfileAuth::Entra {
                    sign_in_id: sign_in_id.clone(),
                    tenant: non_blank(tenant.as_deref()).map(str::to_string),
                };
            }
            _ => {
                return Err(Error::Invalid(
                    "a profile's sign-in method can't be changed; add a new profile instead".into(),
                ));
            }
        }
        apply_draft_options(&mut p, draft, pins);
        check_emulator_options(&p)?;
        self.update(&p)?;
        Ok(p)
    }

    /// Remove a profile row only (rollback helper; the secret is handled by the caller).
    fn delete_row(&self, id: &str) {
        let conn = self.conn.lock().expect("profile db poisoned");
        let _ = conn.execute("DELETE FROM profiles WHERE id = ?1", params![id]);
    }
```

Free functions in the same file:

```rust
fn non_blank(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn clean_pins(raw: &[String]) -> Result<Vec<String>> {
    raw.iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| EntityPath::parse(p).map(|_| p.to_string()))
        .collect()
}

fn merge_pins(first: Vec<String>, rest: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in first.into_iter().chain(rest) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&p)) {
            out.push(p);
        }
    }
    out
}

fn apply_draft_options(p: &mut Profile, d: &ProfileDraft, pins: Vec<String>) {
    p.name = d.name.trim().to_string();
    p.production = d.production;
    p.color = non_blank(d.color.as_deref()).map(str::to_string);
    p.transport = d.transport;
    p.proxy = non_blank(d.proxy.as_deref()).map(str::to_string);
    p.pinned_entities = pins;
}

fn check_emulator_options(p: &Profile) -> Result<()> {
    if p.emulator && (p.transport == Transport::WebSockets || p.proxy.is_some()) {
        return Err(Error::Unsupported {
            capability: "WebSockets and proxies on the Service Bus Emulator".into(),
        });
    }
    Ok(())
}
```

(`Transport` is already imported in `store.rs`; add it if not.) Re-export from `crates/dispatched-core/src/lib.rs` is not needed; callers use `dispatched_core::profiles::{ProfileDraft, DraftAuth}`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dispatched-core --lib profiles && cargo clippy -p dispatched-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/dispatched-core
git commit -m "Core: create and edit profiles from a UI draft, secrets only in the keychain"
```

---
### Task 3: App crate — state, events, settings and profile commands

Rebuild `app/src-tauri` around a testable `ops` layer. This task covers app state, the event channels, settings, app info and the profile commands; Task 4 adds sign-in, connection and tree.

**Files:**
- Modify: `app/src-tauri/Cargo.toml`
- Rewrite: `app/src-tauri/src/state.rs`, `app/src-tauri/src/error.rs`, `app/src-tauri/src/lib.rs`
- Create: `app/src-tauri/src/events.rs`, `app/src-tauri/src/ops/mod.rs`, `app/src-tauri/src/ops/settings.rs`, `app/src-tauri/src/ops/profiles.rs`, `app/src-tauri/src/commands.rs`
- Delete: `app/src-tauri/src/commands/` (the M0 `connection.rs`, `browse.rs`, `mod.rs`; `peek` moves into `commands.rs` in Task 4)

**Interfaces:**
- Consumes: Task 1 (`paths::*`, `app_setting`, `set_app_setting`, `clear_app_settings_with_prefix`, `get_by_id`), Task 2 (`ProfileDraft`, `save_draft`), `dispatched_core::diagnostics::{test_connection, ConnectionReport}`, `dispatched_core::profiles::namespace_for`.
- Produces (Rust, used by Task 4):
  - `pub struct AppState { pub store: ProfileStore, pub secrets: Arc<dyn CredentialStore>, pub session: Arc<EntraSession>, pub active: tokio::sync::RwLock<Option<Active>>, pub sign_in_cancel: tokio::sync::Notify, pub startup_error: Option<String> }`
  - `pub struct Active { pub profile_id: String, pub explorer: Arc<Explorer> }` (`Clone`)
  - `AppState::new(store, secrets) -> Self`, `AppState::open() -> Self` (never fails; falls back to an in-memory store and sets `startup_error`), `AppState::active() -> Result<Active, CommandError>` (kind `not_connected`), `AppState::current_sign_in() -> Result<Option<SignIn>, CommandError>`
  - `pub trait Events: Send + Sync { fn activity(&self, e: ActivityEntry); fn job(&self, e: JobEvent); fn sign_in_prompt(&self, p: SignInPrompt); }`, `TauriEvents(AppHandle)`, `#[cfg(test)] Recorder`
  - `pub async fn tracked<T>(ev: &dyn Events, failed: &str, work: impl Future<Output = Result<T, CommandError>>, describe: impl FnOnce(&T) -> String) -> Result<T, CommandError>`
  - `ops::profiles::namespace_for_profile(state: &AppState, p: &Profile) -> Result<Namespace, CommandError>`
- Produces (IPC, used by Task 11):
  - `app_info() -> { version: string, startup_error: string | null }`
  - `settings_get(key: string) -> unknown | null`, `settings_set(key: string, value: unknown | null) -> null`
  - `profiles_list() -> Profile[]`, `profile_save(draft: ProfileDraft) -> Profile`, `profile_delete(id: string) -> null`, `profile_test(id: string) -> ConnectionReport`
  - events `activity: ActivityEntry`, `job: JobEvent`, `sign-in-prompt: SignInPrompt` (shapes below)

- [ ] **Step 1: Update `app/src-tauri/Cargo.toml`**

Replace the `[dependencies]` block and add `[dev-dependencies]`:

```toml
[dependencies]
dispatched-core = { workspace = true }
horizon-azure-kit = { workspace = true }
open = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
time = { workspace = true }
tauri = { version = "2", features = [] }
tauri-plugin-process = "2"
tauri-plugin-updater = "2"
tokio = { version = "1", features = ["sync", "macros", "time"] }

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
```

- [ ] **Step 2: Write the failing tests**

Create `app/src-tauri/src/ops/mod.rs`:

```rust
//! Plain functions behind the Tauri commands. Commands only unwrap Tauri's
//! state and handle, so everything here is testable without a window.

pub mod profiles;
pub mod settings;

#[cfg(test)]
pub(crate) mod test_util {
    use crate::state::AppState;
    use dispatched_core::profiles::ProfileStore;
    use horizon_azure_kit::secrets::MemoryStore;
    use std::sync::Arc;

    pub const CS: &str = "Endpoint=sb://myns.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0LWtleQ==";
    pub const SECRET_PART: &str = "c2VjcmV0LWtleQ";

    pub fn state() -> AppState {
        AppState::new(
            ProfileStore::open_in_memory().unwrap(),
            Arc::new(MemoryStore::new()),
        )
    }
}
```

Create `app/src-tauri/src/ops/settings.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_util::state;
    use serde_json::json;

    #[test]
    fn settings_round_trip_as_json() {
        let s = state();
        assert_eq!(get(&s, "ui").unwrap(), None);
        set(&s, "ui", Some(json!({"theme": "dark", "density": "compact"}))).unwrap();
        assert_eq!(get(&s, "ui").unwrap(), Some(json!({"theme": "dark", "density": "compact"})));
        set(&s, "ui", None).unwrap();
        assert_eq!(get(&s, "ui").unwrap(), None);
    }

    #[test]
    fn unparsable_stored_value_reads_as_none() {
        let s = state();
        s.store.set_app_setting("ui", Some("{not json")).unwrap();
        assert_eq!(get(&s, "ui").unwrap(), None);
    }

    #[test]
    fn oversized_values_are_rejected() {
        let s = state();
        let big = serde_json::Value::String("x".repeat(MAX_SETTING_BYTES + 1));
        let e = set(&s, "ui", Some(big)).unwrap_err();
        assert_eq!(e.kind, "invalid");
    }
}
```

Create `app/src-tauri/src/ops/profiles.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::{CS, SECRET_PART, state};
    use dispatched_core::namespace::Transport;
    use horizon_azure_kit::secrets::CredentialStore;
    use dispatched_core::profiles::{DraftAuth, ProfileDraft};

    fn draft(name: &str, cs: Option<&str>) -> ProfileDraft {
        ProfileDraft {
            id: None,
            name: name.into(),
            auth: DraftAuth::ConnectionString { connection_string: cs.map(str::to_string) },
            production: false,
            color: None,
            transport: Transport::Amqp,
            proxy: None,
            pinned_entities: vec![],
        }
    }

    #[tokio::test]
    async fn save_response_and_activity_never_contain_the_secret() {
        let s = state();
        let ev = Recorder::default();
        let p = save(&s, &ev, draft("Logistics", Some(CS))).await.unwrap();
        assert!(!serde_json::to_string(&p).unwrap().contains(SECRET_PART));
        assert!(!serde_json::to_string(&list(&s).unwrap()).unwrap().contains(SECRET_PART));
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Saved profile Logistics"), "{log}");
        assert!(!log.contains(SECRET_PART), "{log}");
    }

    #[tokio::test]
    async fn bad_connection_string_error_does_not_echo_it() {
        let s = state();
        let ev = Recorder::default();
        let bad = "Endpoint=sb://x/;SharedAccessKey=TOPSECRET;SharedAccessKeyName";
        let e = save(&s, &ev, draft("x", Some(bad))).await.unwrap_err();
        assert!(!e.message.contains("TOPSECRET"));
        let log = serde_json::to_string(&*ev.activity.lock().unwrap()).unwrap();
        assert!(log.contains("Saving the profile failed"), "{log}");
        assert!(!log.contains("TOPSECRET"), "{log}");
    }

    #[tokio::test]
    async fn delete_profile_clears_its_secret_and_shell_state() {
        let s = state();
        let ev = Recorder::default();
        let p = save(&s, &ev, draft("Logistics", Some(CS))).await.unwrap();
        s.store.set_app_setting(&format!("shell:{}", p.id), Some("{}")).unwrap();
        s.store.set_app_setting("ui", Some("{}")).unwrap();
        delete(&s, &ev, &p.id).await.unwrap();
        assert!(list(&s).unwrap().is_empty());
        assert!(s.secrets.get(&dispatched_core::profiles::secret_key(&p.id)).unwrap().is_none());
        assert!(s.store.app_setting(&format!("shell:{}", p.id)).unwrap().is_none());
        assert!(s.store.app_setting("ui").unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_missing_profile_is_not_found() {
        let s = state();
        let e = delete(&s, &Recorder::default(), "nope").await.unwrap_err();
        assert_eq!(e.kind, "not_found");
    }

    #[tokio::test]
    async fn connect_reports_missing_secret_with_the_fix() {
        let s = state();
        // A profile row whose keychain entry is gone (cleared keychain, copied DB).
        let p = dispatched_core::profiles::Profile::connection_string(
            "Orphan",
            &dispatched_core::ConnectionString::parse(CS).unwrap(),
        );
        s.store.insert(&p).unwrap();
        let e = namespace_for_profile(&s, &p).unwrap_err();
        assert_eq!(e.kind, "invalid");
        assert!(e.message.contains("Orphan") && e.message.contains("connection string"), "{}", e.message);
    }
}
```

Create `app/src-tauri/src/events.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CommandError;

    #[tokio::test]
    async fn tracked_records_success_and_failure_with_durations() {
        let ev = Recorder::default();
        let ok: Result<u32, CommandError> = tracked(&ev, "Counting failed", async { Ok(3) }, |n| format!("Counted {n}")).await;
        assert_eq!(ok.unwrap(), 3);
        let err: Result<u32, CommandError> = tracked(
            &ev,
            "Counting failed",
            async { Err(CommandError::new("forbidden", "access denied (needs Listen): no")) },
            |n| format!("Counted {n}"),
        )
        .await;
        assert!(err.is_err());
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Counted 3");
        assert!(log[0].error.is_none());
        assert_eq!(log[1].action, "Counting failed");
        assert_eq!(log[1].error.as_ref().unwrap().kind, "forbidden");
        assert!(time::OffsetDateTime::parse(&log[0].at, &time::format_description::well_known::Rfc3339).is_ok());
    }

    #[test]
    fn events_serialize_in_the_shapes_the_ui_reads() {
        let e = ActivityEntry {
            at: "2026-10-15T09:41:12Z".into(),
            action: "Listed 4 queues and 3 topics".into(),
            duration_ms: 312,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"at":"2026-10-15T09:41:12Z","action":"Listed 4 queues and 3 topics","duration_ms":312,"error":null})
        );
        let j = JobEvent::Progress { id: "j1".into(), done: 5, total: Some(10) };
        assert_eq!(
            serde_json::to_value(&j).unwrap(),
            serde_json::json!({"type":"progress","id":"j1","done":5,"total":10})
        );
        let p = SignInPrompt { kind: "device-code", url: "https://microsoft.com/devicelogin".into(), user_code: Some("ABCD-EFGH".into()), message: "Go to ...".into() };
        assert_eq!(serde_json::to_value(&p).unwrap()["user_code"], "ABCD-EFGH");
    }
}
```

Extend the existing tests in `app/src-tauri/src/error.rs` (keep the two M0 tests):

```rust
    #[test]
    fn kit_errors_map_through_core() {
        let e: CommandError = horizon_azure_kit::KitError::NotSignedIn("expired".into()).into();
        assert_eq!(e.kind, "not_signed_in");
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && df -h /System/Volumes/Data && (cd app && npm run build) && cargo test -p dispatched-app 2>&1 | tail -20`
Expected: compile errors (`AppState::new`, `Recorder`, `tracked`, `save`, `get`, ... not defined).

- [ ] **Step 4: Implement**

`app/src-tauri/src/error.rs` (replace everything above the tests module):

```rust
use dispatched_core::Error;
use serde::Serialize;

/// Error shape returned to the frontend: a stable `kind` the UI branches on
/// and a sentence for people. Never contains secrets (core errors don't).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CommandError {
    pub kind: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        Self {
            kind: dispatched_core::diagnostics::error_kind(&e),
            message: e.to_string(),
        }
    }
}

impl From<horizon_azure_kit::KitError> for CommandError {
    fn from(e: horizon_azure_kit::KitError) -> Self {
        Error::from(e).into()
    }
}
```


`app/src-tauri/src/events.rs` (above its tests):

```rust
//! Event channels from Rust to the UI: activity log, jobs, sign-in prompts.
//! Payloads are built from names, counts and error texts only, never from
//! secrets or message bodies.

use crate::error::CommandError;
use serde::Serialize;
use std::future::Future;
use std::time::Instant;

pub const ACTIVITY: &str = "activity";
pub const JOB: &str = "job";
pub const SIGN_IN_PROMPT: &str = "sign-in-prompt";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivityError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActivityEntry {
    /// RFC 3339, UTC.
    pub at: String,
    pub action: String,
    pub duration_ms: u64,
    pub error: Option<ActivityError>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum JobEvent {
    Started { id: String, title: String, total: Option<u64> },
    Progress { id: String, done: u64, total: Option<u64> },
    Finished { id: String, error: Option<ActivityError> },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SignInPrompt {
    /// `browser` or `device-code`.
    pub kind: &'static str,
    pub url: String,
    pub user_code: Option<String>,
    pub message: String,
}

pub trait Events: Send + Sync {
    fn activity(&self, e: ActivityEntry);
    fn job(&self, e: JobEvent);
    fn sign_in_prompt(&self, p: SignInPrompt);
}

/// Emits to every window.
pub struct TauriEvents(pub tauri::AppHandle);

impl Events for TauriEvents {
    fn activity(&self, e: ActivityEntry) {
        use tauri::Emitter as _;
        let _ = self.0.emit(ACTIVITY, e);
    }
    fn job(&self, e: JobEvent) {
        use tauri::Emitter as _;
        let _ = self.0.emit(JOB, e);
    }
    fn sign_in_prompt(&self, p: SignInPrompt) {
        use tauri::Emitter as _;
        let _ = self.0.emit(SIGN_IN_PROMPT, p);
    }
}

/// Collects events (tests).
#[cfg(test)]
#[derive(Default)]
pub struct Recorder {
    pub activity: std::sync::Mutex<Vec<ActivityEntry>>,
    pub jobs: std::sync::Mutex<Vec<JobEvent>>,
    pub prompts: std::sync::Mutex<Vec<SignInPrompt>>,
}

#[cfg(test)]
impl Events for Recorder {
    fn activity(&self, e: ActivityEntry) {
        self.activity.lock().unwrap().push(e);
    }
    fn job(&self, e: JobEvent) {
        self.jobs.lock().unwrap().push(e);
    }
    fn sign_in_prompt(&self, p: SignInPrompt) {
        self.prompts.lock().unwrap().push(p);
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// Run `work` and record one activity entry: `describe(&value)` on success,
/// `failed` plus the error on failure.
pub async fn tracked<T>(
    ev: &dyn Events,
    failed: &str,
    work: impl Future<Output = Result<T, CommandError>>,
    describe: impl FnOnce(&T) -> String,
) -> Result<T, CommandError> {
    let started = Instant::now();
    let result = work.await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let (action, error) = match &result {
        Ok(v) => (describe(v), None),
        Err(e) => (
            failed.to_string(),
            Some(ActivityError { kind: e.kind.to_string(), message: e.message.clone() }),
        ),
    };
    ev.activity(ActivityEntry { at: now_rfc3339(), action, duration_ms, error });
    result
}
```

`app/src-tauri/src/state.rs`:

```rust
use crate::error::CommandError;
use dispatched_core::Explorer;
use dispatched_core::paths::{KEY_PREFIX, KEYCHAIN_SERVICE};
use dispatched_core::profiles::ProfileStore;
use horizon_azure_kit::AzureCloud;
use horizon_azure_kit::auth::{EntraSession, SignIn};
use horizon_azure_kit::secrets::{CredentialStore, OsKeychain};
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

/// The connected profile and its explorer.
#[derive(Clone)]
pub struct Active {
    pub profile_id: String,
    pub explorer: Arc<Explorer>,
}

/// App-wide state.
pub struct AppState {
    pub store: ProfileStore,
    pub secrets: Arc<dyn CredentialStore>,
    pub session: Arc<EntraSession>,
    pub active: RwLock<Option<Active>>,
    /// Wakes a running `sign_in` so it returns `cancelled`.
    pub sign_in_cancel: Notify,
    /// Why the real database could not be opened, if it couldn't.
    pub startup_error: Option<String>,
}

impl AppState {
    pub fn new(store: ProfileStore, secrets: Arc<dyn CredentialStore>) -> Self {
        let session = Arc::new(EntraSession::new(AzureCloud::Public, secrets.clone(), KEY_PREFIX));
        Self {
            store,
            secrets,
            session,
            active: RwLock::new(None),
            sign_in_cancel: Notify::new(),
            startup_error: None,
        }
    }

    /// State over the real database and the OS keychain. If the database
    /// can't be opened the app still starts, on an empty in-memory store, and
    /// the UI shows `startup_error`.
    pub fn open() -> Self {
        let secrets: Arc<dyn CredentialStore> = Arc::new(OsKeychain::new(KEYCHAIN_SERVICE));
        match dispatched_core::paths::open_store() {
            Ok(store) => Self::new(store, secrets),
            Err(e) => {
                let mut s = Self::new(
                    ProfileStore::open_in_memory().expect("in-memory SQLite opens"),
                    secrets,
                );
                s.startup_error = Some(format!(
                    "Your saved profiles could not be opened ({e}). Changes made now will not be kept."
                ));
                s
            }
        }
    }

    pub async fn active(&self) -> Result<Active, CommandError> {
        self.active
            .read()
            .await
            .clone()
            .ok_or_else(|| CommandError::new("not_connected", "Choose a connection first."))
    }

    /// The current sign-in, cleared if its refresh token is gone.
    pub fn current_sign_in(&self) -> Result<Option<SignIn>, CommandError> {
        let Some(s) = self.store.current_sign_in()? else {
            return Ok(None);
        };
        if self.session.is_signed_in(&s)? {
            Ok(Some(s))
        } else {
            self.store.set_current_sign_in(None)?;
            Ok(None)
        }
    }
}
```

`app/src-tauri/src/ops/settings.rs` (above its tests):

```rust
use crate::error::CommandError;
use crate::state::AppState;
use serde_json::Value;

pub const MAX_SETTING_BYTES: usize = 64 * 1024;

/// A stored app setting. A value that no longer parses reads as `None`, so a
/// change in shape between builds falls back to defaults instead of failing.
pub fn get(state: &AppState, key: &str) -> Result<Option<Value>, CommandError> {
    Ok(state
        .store
        .app_setting(key)?
        .and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn set(state: &AppState, key: &str, value: Option<Value>) -> Result<(), CommandError> {
    let text = value.map(|v| v.to_string());
    if text.as_ref().is_some_and(|t| t.len() > MAX_SETTING_BYTES) {
        return Err(CommandError::new("invalid", "That setting is too large to save."));
    }
    state.store.set_app_setting(key, text.as_deref())?;
    Ok(())
}
```

`app/src-tauri/src/ops/profiles.rs` (above its tests):

```rust
use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::state::AppState;
use dispatched_core::diagnostics::{Check, ConnectionReport, test_connection};
use dispatched_core::profiles::{Profile, ProfileAuth, ProfileDraft, namespace_for};
use dispatched_core::{Error, Explorer, Namespace};

pub fn list(state: &AppState) -> Result<Vec<Profile>, CommandError> {
    Ok(state.store.list()?)
}

pub async fn save(state: &AppState, ev: &dyn Events, draft: ProfileDraft) -> Result<Profile, CommandError> {
    tracked(
        ev,
        "Saving the profile failed",
        async {
            let sign_in = match draft.auth {
                dispatched_core::profiles::DraftAuth::Entra { .. } => state.current_sign_in()?,
                _ => None,
            };
            Ok(state.store.save_draft(&draft, state.secrets.as_ref(), sign_in.as_ref())?)
        },
        |p| format!("Saved profile {}", p.name),
    )
    .await
}

pub async fn delete(state: &AppState, ev: &dyn Events, id: &str) -> Result<(), CommandError> {
    let profile = state
        .store
        .get_by_id(id)?
        .ok_or_else(|| CommandError::new("not_found", "That profile no longer exists."))?;
    let name = profile.name.clone();
    tracked(
        ev,
        "Removing the profile failed",
        async {
            let was_active = state.active.read().await.as_ref().is_some_and(|a| a.profile_id == id);
            if was_active {
                if let Some(old) = state.active.write().await.take() {
                    old.explorer.close().await;
                }
            }
            state.store.delete(&profile.name, state.secrets.as_ref())?;
            state.store.clear_app_settings_with_prefix(&format!("shell:{id}"))?;
            Ok(())
        },
        |_| format!("Removed profile {name}"),
    )
    .await
}

/// The namespace a profile points at, with errors that name the profile and the fix.
pub fn namespace_for_profile(state: &AppState, p: &Profile) -> Result<Namespace, CommandError> {
    let sign_in = match &p.auth {
        ProfileAuth::ConnectionString => None,
        ProfileAuth::Entra { sign_in_id, .. } => {
            let s = state.store.sign_in(sign_in_id)?;
            match s {
                Some(s) if state.session.is_signed_in(&s)? => Some(s),
                _ => {
                    return Err(Error::NotSignedIn(format!(
                        "the sign-in used by profile `{}` has expired; sign in again",
                        p.name
                    ))
                    .into());
                }
            }
        }
    };
    Ok(namespace_for(p, state.secrets.as_ref(), &state.session, sign_in.as_ref())?)
}

pub async fn test(state: &AppState, ev: &dyn Events, id: &str) -> Result<ConnectionReport, CommandError> {
    let p = state
        .store
        .get_by_id(id)?
        .ok_or_else(|| CommandError::new("not_found", "That profile no longer exists."))?;
    tracked(
        ev,
        "Testing the connection failed",
        async {
            let explorer = Explorer::new(namespace_for_profile(state, &p)?)?;
            let report = test_connection(&explorer, None).await;
            explorer.close().await;
            Ok(report)
        },
        |r| format!("Tested {}: management {}, messaging {}", p.name, check_word(&r.management), check_word(&r.messaging)),
    )
    .await
}

fn check_word(c: &Check) -> &'static str {
    match c {
        Check::Ok { .. } => "ok",
        Check::Failed { .. } => "failed",
        Check::Skipped { .. } => "skipped",
    }
}
```

`app/src-tauri/src/commands.rs`:

```rust
//! Tauri commands: unwrap state and handle, call `ops`.

use crate::error::CommandError;
use crate::events::TauriEvents;
use crate::ops;
use crate::state::AppState;
use dispatched_core::diagnostics::ConnectionReport;
use dispatched_core::profiles::{Profile, ProfileDraft};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    startup_error: Option<String>,
}

#[tauri::command]
pub fn app_info(app: AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        startup_error: state.startup_error.clone(),
    }
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>, key: String) -> Result<Option<Value>, CommandError> {
    ops::settings::get(&state, &key)
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, key: String, value: Option<Value>) -> Result<(), CommandError> {
    ops::settings::set(&state, &key, value)
}

#[tauri::command]
pub fn profiles_list(state: State<'_, AppState>) -> Result<Vec<Profile>, CommandError> {
    ops::profiles::list(&state)
}

#[tauri::command]
pub async fn profile_save(app: AppHandle, state: State<'_, AppState>, draft: ProfileDraft) -> Result<Profile, CommandError> {
    ops::profiles::save(&state, &TauriEvents(app), draft).await
}

#[tauri::command]
pub async fn profile_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    ops::profiles::delete(&state, &TauriEvents(app), &id).await
}

#[tauri::command]
pub async fn profile_test(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<ConnectionReport, CommandError> {
    ops::profiles::test(&state, &TauriEvents(app), &id).await
}
```

`app/src-tauri/src/lib.rs`:

```rust
mod commands;
mod error;
mod events;
mod ops;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(state::AppState::open())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::settings_get,
            commands::settings_set,
            commands::profiles_list,
            commands::profile_save,
            commands::profile_delete,
            commands::profile_test,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dispatched");
}
```

Delete `app/src-tauri/src/commands/` (all three files).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p dispatched-app && cargo clippy -p dispatched-app --all-targets -- -D warnings`
Expected: PASS. (`app/dist` must exist; Step 3 built it.)

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri Cargo.lock
git commit -m "App: ops layer with activity events, settings and profile commands"
```

---

### Task 4: App crate — sign-in, connection, tree commands and window chrome

**Files:**
- Create: `app/src-tauri/src/ops/account.rs`, `app/src-tauri/src/ops/connection.rs`, `app/src-tauri/src/ops/tree.rs`
- Modify: `app/src-tauri/src/ops/mod.rs` (add the three modules), `app/src-tauri/src/commands.rs`, `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/tauri.conf.json`, `app/src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: Task 3 (`AppState`, `Active`, `Events`, `tracked`, `namespace_for_profile`), core `known_entities`, `Explorer::{list_queues, list_topics, list_subscriptions, peek, close}`.
- Produces (IPC, used by Task 11):
  - `account_current() -> AccountView | null`, `sign_in(tenant: string | null, deviceCode: boolean) -> AccountView`, `sign_in_cancel() -> null`
    - `AccountView { id: string, user_principal: string, display_name: string | null, tenant_id: string }`
  - `connect(profileId: string) -> ConnectionView`, `disconnect() -> null`
    - `ConnectionView { profile_id: string, profile_name: string, host: string, emulator: boolean, auth: string, transport: "amqp" | "websockets", production: boolean }`
  - `list_entities() -> EntityListing`, `list_subscriptions(topic: string) -> SubscriptionInfo[]`
    - `EntityListing { queues: QueueInfo[], topics: TopicInfo[], limited: string | null, known: { entity: string, subscription: string | null }[] }`
  - `peek(entity, subQueue, fromSequence, max) -> PeekedMessage[]` (kept for M1c, unchanged shape)

- [ ] **Step 1: Write the failing tests**

`app/src-tauri/src/ops/tree.rs` tests (the network calls are not unit-tested; the fallback decision is a pure function):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use dispatched_core::{Error, QueueInfo};

    fn queue(name: &str) -> QueueInfo {
        QueueInfo {
            name: name.into(),
            status: None,
            counts: None,
            size_in_bytes: None,
            requires_session: false,
            max_delivery_count: None,
            lock_duration: None,
            forward_to: None,
        }
    }

    #[test]
    fn a_full_listing_passes_through() {
        let l = listing_from(Ok((vec![queue("orders")], vec![])), None, &[]).unwrap();
        assert_eq!(l.queues.len(), 1);
        assert!(l.limited.is_none() && l.known.is_empty());
    }

    #[test]
    fn list_entities_falls_back_to_known_entities_on_forbidden() {
        let forbidden = Err(Error::Forbidden { needed: Some("Manage,EntityRead".into()), detail: "no".into() });
        let pins = vec!["events/subscriptions/audit".to_string()];
        let l = listing_from(forbidden, Some("orders"), &pins).unwrap();
        assert!(l.queues.is_empty() && l.topics.is_empty());
        assert_eq!(l.known.len(), 2);
        let why = l.limited.unwrap();
        assert!(why.contains("can't list") && why.contains("Manage"), "{why}");
    }

    #[test]
    fn forbidden_with_nothing_known_still_explains_how_to_pin() {
        let forbidden = Err(Error::Forbidden { needed: None, detail: "no".into() });
        let l = listing_from(forbidden, None, &[]).unwrap();
        assert!(l.known.is_empty());
        assert!(l.limited.unwrap().contains("pin"));
    }

    #[test]
    fn other_errors_are_errors() {
        let e = listing_from(Err(Error::Network("reset".into())), None, &[]).unwrap_err();
        assert_eq!(e.kind, "network");
    }
}
```

`app/src-tauri/src/ops/connection.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Recorder;
    use crate::ops::test_util::{CS, state};
    use dispatched_core::profiles::Profile;
    use horizon_azure_kit::AzureCloud;
    use horizon_azure_kit::auth::SignIn;

    #[tokio::test]
    async fn connect_sets_the_active_profile_and_disconnect_clears_it() {
        let s = state();
        let ev = Recorder::default();
        let p = s.store.add_connection_string("Logistics", CS, s.secrets.as_ref(), true).unwrap();
        let view = connect(&s, &ev, &p.id).await.unwrap();
        assert_eq!(view.host, "myns.servicebus.windows.net");
        assert!(view.production);
        assert_eq!(s.active().await.unwrap().profile_id, p.id);
        disconnect(&s, &ev).await.unwrap();
        assert_eq!(s.active().await.unwrap_err().kind, "not_connected");
        let log = ev.activity.lock().unwrap();
        assert_eq!(log[0].action, "Connected to Logistics");
        assert_eq!(log[1].action, "Disconnected");
    }

    #[tokio::test]
    async fn connect_with_lost_sign_in_says_sign_in_again() {
        let s = state();
        let si = SignIn {
            id: "oid-1".into(),
            user_principal: "ops@contoso.example".into(),
            tenant_id: "t".into(),
            display_name: None,
            cloud: AzureCloud::Public,
        };
        s.store.save_sign_in(&si).unwrap();
        let p = Profile::entra("Ops", "myns.servicebus.windows.net", "oid-1", None);
        s.store.insert(&p).unwrap();
        // No refresh token in the (memory) keychain.
        let e = connect(&s, &Recorder::default(), &p.id).await.unwrap_err();
        assert_eq!(e.kind, "not_signed_in");
        assert!(e.message.contains("sign in again"), "{}", e.message);
        assert!(s.active().await.is_err());
    }

    #[tokio::test]
    async fn connect_to_a_missing_profile_is_not_found() {
        let s = state();
        assert_eq!(connect(&s, &Recorder::default(), "nope").await.unwrap_err().kind, "not_found");
    }
}
```

`app/src-tauri/src/ops/account.rs` tests (no network: only the cancel path and `current`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_util::state;

    #[test]
    fn no_current_account_without_a_sign_in() {
        assert!(current(&state()).unwrap().is_none());
    }

    #[tokio::test]
    async fn cancel_ends_a_pending_sign_in() {
        let s = state();
        let ev = crate::events::Recorder::default();
        // A sign-in that never finishes stands in for the browser flow.
        let pending = wait_or_cancel(&s, std::future::pending::<Result<SignIn, CommandError>>());
        let (r, ()) = tokio::join!(pending, async {
            tokio::task::yield_now().await;
            cancel(&s);
        });
        assert_eq!(r.unwrap_err().kind, "cancelled");
        drop(ev);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p dispatched-app 2>&1 | tail -20`
Expected: compile errors for the new modules' functions.

- [ ] **Step 3: Implement**

`app/src-tauri/src/ops/tree.rs` (above its tests):

```rust
use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::state::AppState;
use dispatched_core::namespace::known_entities;
use dispatched_core::{EntityPath, Error, QueueInfo, SubscriptionInfo, TopicInfo};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct EntityListing {
    pub queues: Vec<QueueInfo>,
    pub topics: Vec<TopicInfo>,
    /// Why the listing is limited (the identity can't list the namespace).
    pub limited: Option<String>,
    /// Entities we can still show: the connection string's EntityPath, then pins.
    pub known: Vec<EntityPath>,
}

/// Decide what the tree shows from a listing attempt.
pub fn listing_from(
    result: dispatched_core::Result<(Vec<QueueInfo>, Vec<TopicInfo>)>,
    entity_path: Option<&str>,
    pins: &[String],
) -> Result<EntityListing, CommandError> {
    match result {
        Ok((queues, topics)) => Ok(EntityListing { queues, topics, limited: None, known: vec![] }),
        Err(Error::Forbidden { needed, .. }) => {
            let known = known_entities(entity_path, pins);
            let right = needed.unwrap_or_else(|| "Manage".into());
            let limited = if known.is_empty() {
                format!("This identity can't list the namespace (needs {right}). Edit the profile to pin the entities you use.")
            } else {
                format!("This identity can't list the namespace (needs {right}). Showing pinned entities.")
            };
            Ok(EntityListing { queues: vec![], topics: vec![], limited: Some(limited), known })
        }
        Err(e) => Err(e.into()),
    }
}

pub async fn list_entities(state: &AppState, ev: &dyn Events) -> Result<EntityListing, CommandError> {
    let active = state.active().await?;
    let pins = state
        .store
        .get_by_id(&active.profile_id)?
        .map(|p| p.pinned_entities)
        .unwrap_or_default();
    tracked(
        ev,
        "Listing the namespace failed",
        async {
            let ex = &active.explorer;
            let result = tokio::try_join!(ex.list_queues(), ex.list_topics());
            listing_from(result, ex.namespace().entity_path(), &pins)
        },
        |l| match &l.limited {
            None => format!("Listed {} queues and {} topics", l.queues.len(), l.topics.len()),
            Some(_) => format!("Listing not allowed; showing {} pinned entities", l.known.len()),
        },
    )
    .await
}

pub async fn list_subscriptions(state: &AppState, ev: &dyn Events, topic: &str) -> Result<Vec<SubscriptionInfo>, CommandError> {
    let active = state.active().await?;
    tracked(
        ev,
        &format!("Listing subscriptions of {topic} failed"),
        async { Ok(active.explorer.list_subscriptions(topic).await?) },
        |subs| format!("Listed {} subscriptions of {topic}", subs.len()),
    )
    .await
}
```

`app/src-tauri/src/ops/connection.rs` (above its tests):

```rust
use crate::error::CommandError;
use crate::events::{Events, tracked};
use crate::ops::profiles::namespace_for_profile;
use crate::state::{Active, AppState};
use dispatched_core::{Explorer, TokenSource as _, Transport};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct ConnectionView {
    pub profile_id: String,
    pub profile_name: String,
    pub host: String,
    pub emulator: bool,
    /// e.g. "SAS key reader" or "Entra ID ops@contoso.example".
    pub auth: String,
    pub transport: Transport,
    pub production: bool,
}

pub async fn connect(state: &AppState, ev: &dyn Events, profile_id: &str) -> Result<ConnectionView, CommandError> {
    let p = state
        .store
        .get_by_id(profile_id)?
        .ok_or_else(|| CommandError::new("not_found", "That profile no longer exists."))?;
    let name = p.name.clone();
    tracked(
        ev,
        &format!("Connecting to {name} failed"),
        async {
            let ns = namespace_for_profile(state, &p)?;
            let view = ConnectionView {
                profile_id: p.id.clone(),
                profile_name: p.name.clone(),
                host: ns.host().to_string(),
                emulator: ns.is_emulator(),
                auth: ns.tokens().describe(),
                transport: ns.transport(),
                production: p.production,
            };
            let explorer = Arc::new(Explorer::new(ns)?);
            let previous = state
                .active
                .write()
                .await
                .replace(Active { profile_id: p.id.clone(), explorer });
            if let Some(old) = previous {
                old.explorer.close().await;
            }
            Ok(view)
        },
        |_| format!("Connected to {name}"),
    )
    .await
}

pub async fn disconnect(state: &AppState, ev: &dyn Events) -> Result<(), CommandError> {
    tracked(
        ev,
        "Disconnecting failed",
        async {
            // Take it out first so the write lock is not held while closing.
            let old = state.active.write().await.take();
            if let Some(old) = old {
                old.explorer.close().await;
            }
            Ok(())
        },
        |_| "Disconnected".to_string(),
    )
    .await
}
```

`app/src-tauri/src/ops/account.rs` (above its tests):

```rust
use crate::error::CommandError;
use crate::events::{Events, SignInPrompt, tracked};
use crate::state::AppState;
use horizon_azure_kit::auth::SignIn;
use serde::Serialize;
use std::future::Future;

#[derive(Debug, Serialize)]
pub struct AccountView {
    pub id: String,
    pub user_principal: String,
    pub display_name: Option<String>,
    pub tenant_id: String,
}

impl From<&SignIn> for AccountView {
    fn from(s: &SignIn) -> Self {
        Self {
            id: s.id.clone(),
            user_principal: s.user_principal.clone(),
            display_name: s.display_name.clone(),
            tenant_id: s.tenant_id.clone(),
        }
    }
}

pub fn current(state: &AppState) -> Result<Option<AccountView>, CommandError> {
    Ok(state.current_sign_in()?.as_ref().map(AccountView::from))
}

/// Await `login`, or return `cancelled` when `cancel` is called.
pub(crate) async fn wait_or_cancel(
    state: &AppState,
    login: impl Future<Output = Result<SignIn, CommandError>>,
) -> Result<SignIn, CommandError> {
    tokio::select! {
        r = login => r,
        () = state.sign_in_cancel.notified() => Err(CommandError::new("cancelled", "Sign-in cancelled.")),
    }
}

pub async fn sign_in(
    state: &AppState,
    ev: &dyn Events,
    tenant: Option<String>,
    device_code: bool,
) -> Result<AccountView, CommandError> {
    let tenant = tenant
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "organizations".into());
    tracked(
        ev,
        "Sign-in failed",
        async {
            let session = state.session.clone();
            let login = async {
                let r = if device_code {
                    session
                        .login_device_code(&tenant, |dc| {
                            ev.sign_in_prompt(SignInPrompt {
                                kind: "device-code",
                                url: dc.verification_uri.clone(),
                                user_code: Some(dc.user_code.clone()),
                                message: dc.message.clone(),
                            })
                        })
                        .await
                } else {
                    session
                        .login_browser(&tenant, |url| {
                            ev.sign_in_prompt(SignInPrompt {
                                kind: "browser",
                                url: url.to_string(),
                                user_code: None,
                                message: "Finish signing in in your browser.".into(),
                            });
                            let _ = open::that(url);
                        })
                        .await
                };
                r.map_err(CommandError::from)
            };
            let s = wait_or_cancel(state, login).await?;
            state.store.save_sign_in(&s)?;
            state.store.set_current_sign_in(Some(&s.id))?;
            Ok(AccountView::from(&s))
        },
        |a| format!("Signed in as {}", a.user_principal),
    )
    .await
}

pub fn cancel(state: &AppState) {
    state.sign_in_cancel.notify_waiters();
}
```

Add to `app/src-tauri/src/ops/mod.rs`: `pub mod account; pub mod connection; pub mod tree;`.

Append to `app/src-tauri/src/commands.rs`:

```rust
use crate::ops::account::AccountView;
use crate::ops::connection::ConnectionView;
use crate::ops::tree::EntityListing;
use dispatched_core::{EntityPath, PeekOptions, PeekView, PeekedMessage, SubQueue, SubscriptionInfo};

#[tauri::command]
pub fn account_current(state: State<'_, AppState>) -> Result<Option<AccountView>, CommandError> {
    ops::account::current(&state)
}

#[tauri::command]
pub async fn sign_in(app: AppHandle, state: State<'_, AppState>, tenant: Option<String>, device_code: bool) -> Result<AccountView, CommandError> {
    ops::account::sign_in(&state, &TauriEvents(app), tenant, device_code).await
}

#[tauri::command]
pub fn sign_in_cancel(state: State<'_, AppState>) {
    ops::account::cancel(&state);
}

#[tauri::command]
pub async fn connect(app: AppHandle, state: State<'_, AppState>, profile_id: String) -> Result<ConnectionView, CommandError> {
    ops::connection::connect(&state, &TauriEvents(app), &profile_id).await
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<(), CommandError> {
    ops::connection::disconnect(&state, &TauriEvents(app)).await
}

#[tauri::command]
pub async fn list_entities(app: AppHandle, state: State<'_, AppState>) -> Result<EntityListing, CommandError> {
    ops::tree::list_entities(&state, &TauriEvents(app)).await
}

#[tauri::command]
pub async fn list_subscriptions(app: AppHandle, state: State<'_, AppState>, topic: String) -> Result<Vec<SubscriptionInfo>, CommandError> {
    ops::tree::list_subscriptions(&state, &TauriEvents(app), &topic).await
}

/// Kept from M0 for M1c's message views.
#[tauri::command]
pub async fn peek(
    state: State<'_, AppState>,
    entity: String,
    sub_queue: SubQueue,
    from_sequence: i64,
    max: u32,
) -> Result<Vec<PeekedMessage>, CommandError> {
    let path = EntityPath::parse(&entity)?;
    let view = match sub_queue {
        SubQueue::Active => PeekView::Active,
        SubQueue::DeadLetter => PeekView::DeadLetter,
        SubQueue::TransferDeadLetter => PeekView::TransferDeadLetter,
    };
    let opts = PeekOptions::new(max.min(500)).from_sequence(from_sequence);
    let page = state.active().await?.explorer.peek(&path, view, &opts).await?;
    Ok(page.messages)
}
```

Register them in `lib.rs`'s `generate_handler!` after `commands::profile_test`: `commands::account_current, commands::sign_in, commands::sign_in_cancel, commands::connect, commands::disconnect, commands::list_entities, commands::list_subscriptions, commands::peek,`.

`app/src-tauri/tauri.conf.json`: in `app.windows[0]` add `"titleBarStyle": "Overlay"`, `"hiddenTitle": true`, `"minWidth": 960`, and change the CSP to:

```json
"csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost"
```

(`titleBarStyle`/`hiddenTitle` only affect macOS; Windows and Linux keep the native title bar.)

`app/src-tauri/capabilities/default.json` permissions become:

```json
"permissions": [
  "core:default",
  "core:window:allow-start-dragging",
  "updater:default",
  "process:default"
]
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dispatched-app && cargo clippy -p dispatched-app --all-targets -- -D warnings && cargo test`
Expected: PASS (the full workspace too).

- [ ] **Step 5: Commit**

```bash
git add app/src-tauri Cargo.lock
git commit -m "App: sign-in, connection and tree commands; macOS overlay title bar"
```

---
### Task 5: Frontend toolchain — Tailwind, shadcn config, ESLint boundaries, Vitest, Playwright

**Files:**
- Modify: `app/package.json`, `app/package-lock.json`, `app/vite.config.ts`, `app/tsconfig.json`, `.gitignore`
- Create: `app/components.json`, `app/eslint.config.js`, `app/vitest.config.ts`, `app/playwright.config.ts`, `app/src/test/setup.ts`, `app/src/test/boundaries.test.ts`, `app/src/ui/styles.css` (minimal, completed in Task 7), `app/e2e/smoke.spec.ts`

**Interfaces:**
- Produces: npm scripts `dev`, `build`, `build:mock`, `preview:mock`, `lint`, `typecheck`, `test`, `e2e`, `shots`; the `@/` path alias for `app/src/`; the boundary lint rules every later task must satisfy; `src/test/setup.ts` stubs (ResizeObserver, matchMedia, pointer capture, scrollIntoView) that component tests rely on.

- [ ] **Step 1: Install dependencies**

Run (check disk first):

```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched/app && df -h /System/Volumes/Data
npm install tailwindcss@^4.3.3 @tailwindcss/vite@^4.3.3 radix-ui@^1.8.1 class-variance-authority@^0.7.1 clsx@^2.1.1 tailwind-merge@^3.7.0 lucide-react@^0.577.0 cmdk@^1.1.1 @tanstack/react-table@^8.21.3 @tanstack/react-virtual@^3.14.19 @tanstack/react-query@^5.104.3 motion@^12.43.0 react-resizable-panels@^3.0.6 @fontsource/jetbrains-mono@^5.3.0 @tauri-apps/plugin-updater@^2.14.0 @tauri-apps/plugin-process@^2.4.0
npm install -D vitest@^3.2.7 @testing-library/react@^16.3.3 @testing-library/user-event@^14.6.8 @testing-library/jest-dom@^6.10.0 jsdom@^26.1.0 eslint@^9.39.5 @eslint/js@^9.39.5 typescript-eslint@^8.71.2 eslint-plugin-react-hooks@^5.2.0 globals@^16.5.0 @playwright/test@^1.64.1 @types/node@^22
```

- [ ] **Step 2: Configure scripts, Vite, TypeScript and ignores**

`app/package.json` `scripts`:

```json
"scripts": {
  "dev": "vite",
  "build": "tsc --noEmit && vite build",
  "build:mock": "VITE_DISPATCHED_MOCK=1 vite build --outDir dist-mock",
  "preview:mock": "vite preview --outDir dist-mock --port 4173 --strictPort",
  "lint": "eslint .",
  "typecheck": "tsc --noEmit",
  "test": "vitest run",
  "e2e": "playwright test",
  "shots": "playwright test e2e/shots.spec.ts --project=webkit",
  "tauri": "tauri"
}
```

`app/vite.config.ts`: add the Tailwind plugin and the alias (keep everything else as it is):

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath, URL } from "node:url";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
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

`app/tsconfig.json` `compilerOptions`: add `"baseUrl": "."`, `"paths": { "@/*": ["src/*"] }`, `"types": ["vite/client"]`. Keep `"include": ["src"]`.

`app/vitest.config.ts`:

```ts
import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: "jsdom",
      setupFiles: ["./src/test/setup.ts"],
      include: ["src/**/*.test.{ts,tsx}"],
      css: false,
    },
  }),
);
```

`app/src/test/setup.ts`:

```ts
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => cleanup());

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;

window.matchMedia ??= ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addEventListener: () => {},
  removeEventListener: () => {},
  addListener: () => {},
  removeListener: () => {},
  dispatchEvent: () => false,
})) as typeof window.matchMedia;

Element.prototype.hasPointerCapture ??= () => false;
Element.prototype.setPointerCapture ??= () => {};
Element.prototype.releasePointerCapture ??= () => {};
Element.prototype.scrollIntoView ??= () => {};
```

`.gitignore` (repo root): add

```
/app/dist-mock/
/app/playwright-report/
/app/test-results/
```

`app/components.json` (lets `npx shadcn add` place future components correctly):

```json
{
  "$schema": "https://ui.shadcn.com/schema.json",
  "style": "new-york",
  "rsc": false,
  "tsx": true,
  "tailwind": { "config": "", "css": "src/ui/styles.css", "baseColor": "neutral", "cssVariables": true, "prefix": "" },
  "aliases": { "components": "@/ui/components", "utils": "@/ui/lib/cn", "ui": "@/ui/components", "lib": "@/ui/lib", "hooks": "@/ui/hooks" },
  "iconLibrary": "lucide"
}
```

`app/src/ui/styles.css` (Task 7 completes it):

```css
@import "tailwindcss";
```

Import it at the top of `app/src/main.tsx`: `import "./ui/styles.css";`.

- [ ] **Step 3: Write the boundary test (failing: no ESLint config yet)**

`app/src/test/boundaries.test.ts`:

```ts
// @vitest-environment node
import { ESLint } from "eslint";
import { describe, expect, it } from "vitest";

const eslint = new ESLint({ cwd: process.cwd() });

async function restricted(filePath: string, code: string): Promise<string[]> {
  const [result] = await eslint.lintText(code, { filePath });
  return result.messages.filter((m) => m.ruleId === "no-restricted-imports").map((m) => m.message);
}

describe("import boundaries", () => {
  it("ui cannot import app code", async () => {
    expect(await restricted("src/ui/components/x.tsx", `import { a } from "../../engine/commands";\nexport const b = a;`)).toHaveLength(1);
    expect(await restricted("src/ui/x.ts", `import { a } from "../shell/Shell";\nexport const b = a;`)).toHaveLength(1);
    expect(await restricted("src/ui/x.ts", `import { a } from "@/engine/commands";\nexport const b = a;`)).toHaveLength(1);
  });

  it("ui can import itself and packages", async () => {
    expect(await restricted("src/ui/components/x.tsx", `import { cn } from "../lib/cn";\nimport { clsx } from "clsx";\nexport const b = [cn, clsx];`)).toHaveLength(0);
  });

  it("engine cannot import ui or shell", async () => {
    expect(await restricted("src/engine/queries.ts", `import { a } from "../ui";\nexport const b = a;`)).toHaveLength(1);
    expect(await restricted("src/engine/queries.ts", `import { a } from "@/shell/state";\nexport const b = a;`)).toHaveLength(1);
  });

  it("only engine/ipc.ts imports Tauri", async () => {
    expect(await restricted("src/shell/x.ts", `import { invoke } from "@tauri-apps/api/core";\nexport const b = invoke;`)).toHaveLength(1);
    expect(await restricted("src/engine/ipc.ts", `import { invoke } from "@tauri-apps/api/core";\nexport const b = invoke;`)).toHaveLength(0);
  });

  it("shell cannot import the dev catalogue", async () => {
    expect(await restricted("src/shell/x.ts", `import { Catalogue } from "../dev/Catalogue";\nexport const b = Catalogue;`)).toHaveLength(1);
  });
});
```

Run: `npx vitest run src/test/boundaries.test.ts`
Expected: FAIL (no ESLint config, so no `no-restricted-imports` messages).

- [ ] **Step 4: Write `app/eslint.config.js`**

```js
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

const tauri = { group: ["@tauri-apps/*"], message: "Only src/engine/ipc.ts talks to Tauri." };
const appCode = (dirs) => dirs.flatMap((d) => [`**/${d}`, `**/${d}/**`, `@/${d}`, `@/${d}/**`]);
const restrict = (...patterns) => ["error", { patterns }];

export default tseslint.config(
  { ignores: ["dist", "dist-mock", "playwright-report", "test-results", "src-tauri", "node_modules"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: { ...globals.browser } },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "no-restricted-imports": restrict(tauri),
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
  {
    files: ["src/ui/**"],
    rules: {
      "no-restricted-imports": restrict(tauri, {
        group: appCode(["engine", "shell", "dev"]),
        message: "src/ui must not import app code; it becomes @horizon-tech/explorer-ui after M1.",
      }),
    },
  },
  {
    files: ["src/engine/**"],
    rules: {
      "no-restricted-imports": restrict(tauri, {
        group: appCode(["ui", "shell", "dev"]),
        message: "src/engine must not import UI code.",
      }),
    },
  },
  { files: ["src/engine/ipc.ts"], rules: { "no-restricted-imports": "off" } },
  {
    files: ["src/shell/**"],
    rules: {
      "no-restricted-imports": restrict(tauri, {
        group: appCode(["dev"]),
        message: "The dev catalogue is not part of the shell.",
      }),
    },
  },
  { files: ["e2e/**", "*.config.{js,ts}"], languageOptions: { globals: { ...globals.node } } },
);
```

Run: `npx vitest run src/test/boundaries.test.ts && npm run lint`
Expected: the boundary test PASSES. `npm run lint` may report problems in the M0 `src/App.tsx` / `src/api.ts`; fix only what is needed to pass (both files are replaced in Tasks 12–13).

- [ ] **Step 5: Playwright config and smoke test**

`app/playwright.config.ts`:

```ts
import { defineConfig, devices } from "@playwright/test";

const viewport = { width: 1440, height: 900 };

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : [["list"]],
  use: { baseURL: "http://localhost:4173", trace: "retain-on-failure" },
  webServer: {
    command: "npm run build:mock && npm run preview:mock",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport } },
    { name: "webkit", use: { ...devices["Desktop Safari"], viewport } },
  ],
});
```

`app/e2e/smoke.spec.ts`:

```ts
import { expect, test } from "@playwright/test";

test("the app loads", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle("Dispatched");
});
```

Run (check disk first; the browsers go to `~/Library/Caches/ms-playwright`):

```bash
df -h /System/Volumes/Data && npx playwright install chromium webkit && npm run e2e
```

Expected: PASS in both projects.

- [ ] **Step 6: Typecheck, test and commit**

Run: `npm run typecheck && npm test && npm run lint`
Expected: PASS.

```bash
git add app/package.json app/package-lock.json app/vite.config.ts app/tsconfig.json app/components.json app/eslint.config.js app/vitest.config.ts app/playwright.config.ts app/src/test app/src/ui/styles.css app/src/main.tsx app/e2e .gitignore
git commit -m "App: frontend toolchain with Tailwind, ESLint boundaries, Vitest and Playwright"
```

---

### Task 6: impeccable setup — product context, direction contract, mockup references

This task writes no product code. It gives every later UI task the same reference: `PRODUCT.md`, the direction contract for direction A, and screenshots of mockup A.

**Files:**
- Create: `docs/design/m1b-mockup-a-dark.png`, `docs/design/m1b-mockup-a-light.png`
- Create: `app/PRODUCT.md`
- Create: the surface brief for `app/src/shell` (path chosen by `impeccable surface-brief write`; commit whatever file it creates)

**Interfaces:**
- Produces: `app/PRODUCT.md`; the direction contract (readable with `~/.claude/skills/impeccable/scripts/impeccable surface-brief read app/src/shell`); the two mockup PNGs. Tasks 7–20 hand these to `impeccable-finish-reviewer`.

- [ ] **Step 1 (controller, not a subagent): save the mockup screenshots**

The controller opens the canvas https://claude.ai/artifact/85qsVo8NmMQagPRXus4MwX in the browser, focuses artboard "A. Workbench" at 1440×900, and saves a screenshot with the theme tweak on `dark` and another on `light` to `docs/design/m1b-mockup-a-dark.png` and `docs/design/m1b-mockup-a-light.png`. If the browser is unavailable, ask the user to save the two screenshots there.

- [ ] **Step 2: Load impeccable and check what it needs**

Invoke the `impeccable` skill, then run from the app folder:

```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched/app && ~/.claude/skills/impeccable/scripts/impeccable context
```

Follow its setup for a missing `PRODUCT.md` (its `init` reference), answering from the spec rather than asking the user again; the facts are below.

- [ ] **Step 3: Write `app/PRODUCT.md`**

Use impeccable's PRODUCT.md structure, with this content (adapt headings to the template; keep every fact):

```markdown
# Dispatched (desktop)

## What it is
A cross-platform desktop explorer for Azure Service Bus: browse namespaces, inspect queues, topics and subscriptions, and repair dead-letter queues. The replacement for the Windows-only ServiceBusExplorer, which is the feature reference only, not the design reference.

## Who uses it
Developers and on-call operators who own Service Bus integrations. They open it during incidents ("why are 34,000 messages in this DLQ?") and in daily work (checking a subscription, finding one message). They know Service Bus well and use IDEs and database clients all day.

## Mode
Operate. A tool people are inside of for hours. Density, keyboard speed and trust beat discovery and delight.

## Success
They open it against a production namespace and immediately see where attention is needed: which entities have dead letters, which profile is production, what failed. It feels like a precise professional tool, never a generated template.

## Constraints
- Desktop app (Tauri): macOS, Windows, Linux. Works offline; no network fonts.
- Dark and light themes, following the OS by default.
- Production profiles are always visibly marked.
- Nothing destructive happens without a confirmation in context (no modal dialogs).
- Secrets never appear on screen after they are entered.

## Must not feel like
The previous Arkived UI ("too AI slop"): gradients, glows, decorative cards, pills, emoji, roomy marketing spacing. Also not like the Azure Portal's blades.
```

- [ ] **Step 4: Record the direction contract**

Write this body to a temporary file in the scratchpad and run `~/.claude/skills/impeccable/scripts/impeccable surface-brief write app/src/shell <file>`, then read it back with `surface-brief read app/src/shell` and confirm all six blocks are present:

```markdown
## Direction contract

THESIS: The data is the interface. A quiet IDE-style workbench where colour appears only when something needs attention; it refuses the dashboard-of-cards and the Portal blade.

OWN-WORLD: Graphite (dark) or paper-white (light) surfaces in three close tiers (chrome, side, content), 1px lines, square panels, 3px controls, OS UI font at 12px with JetBrains Mono for data, tabular numerals. One amber accent, only on dead-letter counts above zero, PROD, and errors. Neutral selection, 1px focus outline. Azure-style line glyphs at 1.3 stroke.

STORY: The operator sees which connection is active and whether it is production, scans the tree for amber counts, opens an entity as a tab, and reads its views and counts without hunting.

FIRST VIEWPORT: 38px top bar (connection selector with PROD badge left, palette field centre); tree on the left (about 260px); entity tab strip, header, sub-tab row and content in the centre; Jobs/Activity drawer at the bottom of the centre; 22px status bar. Primary action is choosing or opening an entity; the palette (⌘K) is always one key away.

FORM: IDE workbench, item 1 of 3 mockup directions (A Workbench, B Ledger, C Split), chosen by the user on 2026-10-15. Seed: user-selected from mockups; no concept-seed run.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
```

- [ ] **Step 5: Commit**

```bash
git add docs/design app/PRODUCT.md
git add -A app/.impeccable 2>/dev/null || true
git status --short
git commit -m "Design: product context, direction contract and mockup A references"
```

(`git status --short` shows any other file impeccable created; add it if it is the surface brief.)

---

### Task 7: Design tokens, themes, fonts, glyphs and formatting helpers

**Files:**
- Create: `app/src/ui/tokens.ts`, `app/src/ui/contrast.ts`, `app/src/ui/theme.ts`, `app/src/ui/lib/cn.ts`, `app/src/ui/lib/format.ts`, `app/src/ui/lib/platform.ts`, `app/src/ui/glyphs.tsx`, `app/src/ui/index.ts`
- Modify: `app/src/ui/styles.css`
- Test: `app/src/ui/tokens.test.ts`, `app/src/ui/theme.test.ts`, `app/src/ui/lib/format.test.ts`, `app/src/ui/glyphs.test.tsx`

**Interfaces:**
- Produces:
  - `type ThemeName = "dark" | "light"`, `type ThemePref = "system" | ThemeName`, `type Density = "compact" | "comfortable"`, `type Role` (14 roles), `themes: Record<ThemeName, Record<Role, string>>`, `contrastPairs`
  - `contrastRatio(a: string, b: string): number`
  - `rowHeight: Record<Density, number>`, `resolveTheme(pref, systemDark): ThemeName`, `applyTheme(name, density, root?)`, `useSystemDark(): boolean`, `useTheme(pref, density): ThemeName`
  - `cn(...inputs: ClassValue[]): string`
  - `formatNumber(n)`, `formatCount(n, counts?)`, `formatDuration(ms)`, `formatClock(iso)`
  - `isMac: boolean`, `shortcutLabel(spec: string): string` (`"mod+k"` → `"⌘K"` or `"Ctrl+K"`)
  - glyph components `NamespaceGlyph`, `QueueGlyph`, `TopicGlyph`, `SubscriptionGlyph`, `RuleGlyph`, `EntityGlyph({ kind })` with `kind: "namespace" | "queue" | "topic" | "subscription" | "rule"`
  - Tailwind utilities from `styles.css`: colours `bg-chrome bg-side bg-content bg-field bg-hover bg-selection bg-accent bg-accent-tint`, `text-fg text-muted text-faint text-icon text-accent text-accent-ink`, `border-line`; type `text-11 text-12 text-13 text-14`; `font-sans font-mono`; `rounded-ctl`; `shadow-float`; `h-row`; `ease-out-expo`

Load the `impeccable` and `design-taste-frontend` skills before starting.

- [ ] **Step 1: Write the failing tests**

`app/src/ui/tokens.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { contrastPairs, roles, themes } from "./tokens";
import { contrastRatio } from "./contrast";

describe("tokens", () => {
  it("define every role in both themes as a 6-digit hex", () => {
    for (const name of ["dark", "light"] as const) {
      for (const role of roles) expect(themes[name][role], `${name}.${role}`).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("meet WCAG contrast for every text-on-surface pair", () => {
    const failures: string[] = [];
    for (const name of ["dark", "light"] as const) {
      for (const { text, on, min } of contrastPairs) {
        const r = contrastRatio(themes[name][text], themes[name][on]);
        if (r < min) failures.push(`${name}: ${text} on ${on} = ${r.toFixed(2)} < ${min}`);
      }
    }
    expect(failures).toEqual([]);
  });

  it("computes known ratios", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(contrastRatio("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
  });
});
```

`app/src/ui/theme.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { applyTheme, resolveTheme, rowHeight } from "./theme";
import { themes } from "./tokens";

describe("theme", () => {
  it("resolves system to the OS preference", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
  });

  it("writes role variables, theme and row height to the root", () => {
    const root = document.createElement("div");
    applyTheme("light", "comfortable", root);
    expect(root.style.getPropertyValue("--c-accent")).toBe(themes.light.accent);
    expect(root.style.getPropertyValue("--c-accent-ink")).toBe(themes.light.accentInk);
    expect(root.dataset.theme).toBe("light");
    expect(root.style.getPropertyValue("--row")).toBe(`${rowHeight.comfortable}px`);
  });
});
```

`app/src/ui/lib/format.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { formatCount, formatDuration, formatNumber } from "./format";
import { shortcutLabel } from "./platform";

describe("format", () => {
  it("formats counts like the CLI", () => {
    expect(formatNumber(34621)).toBe("34,621");
    expect(formatCount(null)).toBe("?");
    expect(formatCount(undefined)).toBe("?");
    expect(formatCount(3417)).toBe("3,417");
    expect(formatCount(120, { approximate: true })).toBe("~120");
    expect(formatCount(10000, { approximate: true, truncated: true })).toBe("~10,000+");
  });

  it("formats durations", () => {
    expect(formatDuration(312)).toBe("312 ms");
    expect(formatDuration(1400)).toBe("1.4 s");
    expect(formatDuration(123000)).toBe("2 min 3 s");
  });

  it("labels shortcuts for the platform", () => {
    expect(shortcutLabel("mod+k", true)).toBe("⌘K");
    expect(shortcutLabel("mod+k", false)).toBe("Ctrl+K");
    expect(shortcutLabel("mod+shift+p", true)).toBe("⇧⌘P");
    expect(shortcutLabel("f5", false)).toBe("F5");
  });
});
```

`app/src/ui/glyphs.test.tsx`:

```tsx
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EntityGlyph } from "./glyphs";

describe("glyphs", () => {
  it.each(["namespace", "queue", "topic", "subscription", "rule"] as const)("%s is a decorative 16px-grid svg", (kind) => {
    const { container } = render(<EntityGlyph kind={kind} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("viewBox")).toBe("0 0 16 16");
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("stroke")).toBe("currentColor");
  });
});
```

Run: `npm test -- src/ui`
Expected: FAIL (modules missing).

- [ ] **Step 2: Implement tokens and contrast**

`app/src/ui/tokens.ts`:

```ts
export type ThemeName = "dark" | "light";
export type Role =
  | "chrome" | "side" | "content" | "field" | "line"
  | "fg" | "muted" | "faint" | "icon"
  | "hover" | "selection"
  | "accent" | "accentInk" | "accentTint";

export const roles: readonly Role[] = [
  "chrome", "side", "content", "field", "line", "fg", "muted", "faint", "icon",
  "hover", "selection", "accent", "accentInk", "accentTint",
];

/**
 * Colour roles per theme: the single source of truth. Values start from
 * mockup A; the contrast test below is the authority, so a value may move
 * slightly to pass it. Amber (`accent`) marks attention only: dead letters
 * above zero, PROD, errors.
 */
export const themes: Record<ThemeName, Record<Role, string>> = {
  dark: {
    chrome: "#1d1f23", side: "#191b1e", content: "#141518", field: "#121316", line: "#2a2d32",
    fg: "#d9dce1", muted: "#959aa3", faint: "#727780", icon: "#9aa0a9",
    hover: "#1c1e22", selection: "#262c36",
    accent: "#f0a537", accentInk: "#1d1406", accentTint: "#372e22",
  },
  light: {
    chrome: "#eef0f3", side: "#f6f7f9", content: "#ffffff", field: "#ffffff", line: "#dde0e5",
    fg: "#1d2127", muted: "#59606b", faint: "#767d86", icon: "#59606b",
    hover: "#f3f5f8", selection: "#e2e8f1",
    accent: "#8f4f00", accentInk: "#ffffff", accentTint: "#fbf0e2",
  },
};

const surfaces: Role[] = ["chrome", "side", "content", "field", "hover", "selection"];

/** Text roles drawn on each surface, with their minimum WCAG ratio. `faint`
 *  and `icon` are never the only carrier of must-read text, so they need 3:1. */
export const contrastPairs: ReadonlyArray<{ text: Role; on: Role; min: number }> = [
  ...(["fg", "muted", "accent"] as Role[]).flatMap((text) => surfaces.map((on) => ({ text, on, min: 4.5 }))),
  ...(["faint", "icon"] as Role[]).flatMap((text) => surfaces.map((on) => ({ text, on, min: 3 }))),
  { text: "accentInk", on: "accent", min: 4.5 },
  { text: "accent", on: "accentTint", min: 4.5 },
  { text: "fg", on: "accentTint", min: 4.5 },
];
```

`app/src/ui/contrast.ts`:

```ts
function channel(c: number): number {
  const s = c / 255;
  return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  return 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255);
}

/** WCAG 2 contrast ratio between two `#rrggbb` colours. */
export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}
```

Run `npm test -- src/ui/tokens.test.ts`. If a pair fails, adjust that role's value in the failing theme (darker text in light, lighter text in dark) in small steps until it passes, keeping roles visually distinct (`muted` stays clearly lighter than `fg`, `faint` clearly lighter than `muted`). Record any changed values in the commit message.

- [ ] **Step 3: Implement theme, helpers and glyphs**

`app/src/ui/theme.ts`:

```ts
import { useEffect, useState } from "react";
import { roles, themes, type Role, type ThemeName } from "./tokens";

export type { ThemeName } from "./tokens";
export type ThemePref = "system" | ThemeName;
export type Density = "compact" | "comfortable";

export const rowHeight: Record<Density, number> = { compact: 22, comfortable: 26 };

const cssVar = (role: Role) => `--c-${role.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;

export function resolveTheme(pref: ThemePref, systemDark: boolean): ThemeName {
  return pref === "system" ? (systemDark ? "dark" : "light") : pref;
}

export function applyTheme(name: ThemeName, density: Density, root: HTMLElement = document.documentElement): void {
  for (const role of roles) root.style.setProperty(cssVar(role), themes[name][role]);
  root.style.setProperty("--row", `${rowHeight[density]}px`);
  root.style.colorScheme = name;
  root.dataset.theme = name;
  root.dataset.density = density;
}

export function useSystemDark(): boolean {
  const query = "(prefers-color-scheme: dark)";
  const [dark, setDark] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mql = window.matchMedia(query);
    const onChange = (e: MediaQueryListEvent) => setDark(e.matches);
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);
  return dark;
}

/** Applies the theme to <html> and returns the resolved name. */
export function useTheme(pref: ThemePref, density: Density): ThemeName {
  const name = resolveTheme(pref, useSystemDark());
  useEffect(() => applyTheme(name, density), [name, density]);
  return name;
}
```

`app/src/ui/lib/cn.ts`:

```ts
import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// Teach tailwind-merge our custom font-size scale so `text-12` and `text-muted` don't collide.
const merge = extendTailwindMerge({
  extend: { classGroups: { "font-size": [{ text: ["11", "12", "13", "14"] }] } },
});

export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}
```

`app/src/ui/lib/format.ts`:

```ts
const nf = new Intl.NumberFormat("en-US");

export function formatNumber(n: number): string {
  return nf.format(n);
}

export interface CountQuality {
  approximate?: boolean;
  truncated?: boolean;
}

/** `?` unknown, `~N` counted by peeking, `~N+` when that count hit its cap. */
export function formatCount(n: number | null | undefined, q?: CountQuality): string {
  if (n == null) return "?";
  const s = nf.format(n);
  if (q?.truncated) return `~${s}+`;
  if (q?.approximate) return `~${s}`;
  return s;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)} min ${s % 60} s`;
}

/** Local wall-clock time `HH:MM:SS` for an RFC 3339 timestamp. */
export function formatClock(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString("en-GB", { hour12: false });
}
```

`app/src/ui/lib/platform.ts`:

```ts
export const isMac =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const MAC: Record<string, string> = { mod: "⌘", shift: "⇧", alt: "⌥", ctrl: "⌃" };
const OTHER: Record<string, string> = { mod: "Ctrl", shift: "Shift", alt: "Alt", ctrl: "Ctrl" };
const MAC_ORDER = ["ctrl", "alt", "shift", "mod"];

/** `"mod+shift+p"` → `"⇧⌘P"` on macOS, `"Ctrl+Shift+P"` elsewhere. */
export function shortcutLabel(spec: string, mac: boolean = isMac): string {
  const parts = spec.toLowerCase().split("+");
  const key = parts.pop()!;
  const keyLabel = key.length === 1 ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1);
  if (mac) {
    const mods = MAC_ORDER.filter((m) => parts.includes(m)).map((m) => MAC[m]);
    return mods.join("") + keyLabel;
  }
  return [...parts.map((m) => OTHER[m] ?? m), keyLabel].join("+");
}
```

(`shortcutLabel("f5", false)` gives `"F5"`.)

`app/src/ui/glyphs.tsx`:

```tsx
import type { SVGProps } from "react";

export type EntityKind = "namespace" | "queue" | "topic" | "subscription" | "rule";

type GlyphProps = Omit<SVGProps<SVGSVGElement>, "children"> & { size?: number };

function Glyph({ size = 14, children, ...rest }: GlyphProps & { children: React.ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.3}
      strokeLinejoin="round" strokeLinecap="round" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

export const NamespaceGlyph = (p: GlyphProps) => (
  <Glyph {...p}><rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M2 6.2h12" /></Glyph>
);
export const QueueGlyph = (p: GlyphProps) => (
  <Glyph {...p}><rect x="1.5" y="4.5" width="13" height="7" rx="1" /><path d="M5.8 4.5v7M10.2 4.5v7" /></Glyph>
);
export const TopicGlyph = (p: GlyphProps) => (
  <Glyph {...p}><circle cx="3.8" cy="8" r="1.8" /><path d="M5.6 8h3.2M8.8 8l4.4-4M8.8 8h4.4M8.8 8l4.4 4" /></Glyph>
);
export const SubscriptionGlyph = (p: GlyphProps) => (
  <Glyph {...p}><path d="M2 9.5 3.8 4h8.4L14 9.5V12H2z" /><path d="M2 9.5h3.2l.9 1.4h3.8l.9-1.4H14" /></Glyph>
);
export const RuleGlyph = (p: GlyphProps) => (
  <Glyph {...p}><path d="M2.5 3.5h11l-4.2 5v4l-2.6 1.2V8.5z" /></Glyph>
);

const byKind = { namespace: NamespaceGlyph, queue: QueueGlyph, topic: TopicGlyph, subscription: SubscriptionGlyph, rule: RuleGlyph };

export function EntityGlyph({ kind, ...rest }: GlyphProps & { kind: EntityKind }) {
  const G = byKind[kind];
  return <G {...rest} />;
}
```

`app/src/ui/styles.css` (replace):

```css
@import "tailwindcss";
@import "@fontsource/jetbrains-mono/400.css";
@import "@fontsource/jetbrains-mono/500.css";

/* Only our roles exist as colours: palette utilities like bg-zinc-800 don't compile. */
@theme {
  --color-*: initial;
  --text-*: initial;
  --radius-*: initial;
  --shadow-*: initial;
  --font-*: initial;
}

@theme inline {
  --color-chrome: var(--c-chrome);
  --color-side: var(--c-side);
  --color-content: var(--c-content);
  --color-field: var(--c-field);
  --color-line: var(--c-line);
  --color-fg: var(--c-fg);
  --color-muted: var(--c-muted);
  --color-faint: var(--c-faint);
  --color-icon: var(--c-icon);
  --color-hover: var(--c-hover);
  --color-selection: var(--c-selection);
  --color-accent: var(--c-accent);
  --color-accent-ink: var(--c-accent-ink);
  --color-accent-tint: var(--c-accent-tint);

  --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;

  --text-11: 11px;
  --text-11--line-height: 14px;
  --text-12: 12px;
  --text-12--line-height: 16px;
  --text-13: 13px;
  --text-13--line-height: 18px;
  --text-14: 14px;
  --text-14--line-height: 20px;

  --radius-ctl: 3px;
  --shadow-float: var(--float-shadow);
  --spacing-row: var(--row);
  --ease-out-expo: cubic-bezier(0.16, 1, 0.3, 1);
}

@layer base {
  :root {
    --row: 22px;
    --float-shadow: 0 10px 28px -8px rgb(0 0 0 / 0.55), 0 2px 6px -2px rgb(0 0 0 / 0.35);
    /* Boot colours before JS applies the theme (no flash). */
    background: #141518;
  }
  :root[data-theme="light"] {
    --float-shadow: 0 10px 28px -8px rgb(29 33 39 / 0.18), 0 2px 6px -2px rgb(29 33 39 / 0.12);
  }
  @media (prefers-color-scheme: light) {
    :root:not([data-theme]) { background: #ffffff; }
  }
  html, body, #root { height: 100%; }
  body {
    margin: 0;
    background: var(--c-content);
    color: var(--c-fg);
    font-family: var(--font-sans);
    font-size: 12px;
    line-height: 16px;
    -webkit-font-smoothing: antialiased;
    user-select: none;
    cursor: default;
    overflow: hidden;
  }
  input, textarea, [data-selectable] { user-select: text; }
  :focus-visible { outline: 1px solid var(--c-fg); outline-offset: -1px; }
  ::selection { background: var(--c-selection); color: var(--c-fg); }
  ::placeholder { color: var(--c-faint); }
  * { scrollbar-width: thin; scrollbar-color: var(--c-line) transparent; }
  .font-mono { font-variant-ligatures: none; }
  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { animation-duration: 0.01ms !important; transition-duration: 0.01ms !important; }
  }
}
```

(`h-row` comes from `--spacing-row`; check with `npx vite build` that `h-row` and `bg-chrome` compile. If Tailwind 4.3 rejects `--spacing-row` inside `@theme inline`, define `@utility h-row { height: var(--row); }` instead.)

`app/src/ui/index.ts`:

```ts
export * from "./tokens";
export * from "./contrast";
export * from "./theme";
export * from "./glyphs";
export { cn } from "./lib/cn";
export * from "./lib/format";
export * from "./lib/platform";
```

In `app/src/main.tsx`, call `applyTheme(resolveTheme("system", window.matchMedia("(prefers-color-scheme: dark)").matches), "compact")` before `createRoot(...)`, so the first frame already has the variables.

- [ ] **Step 4: Run the tests, typecheck and build**

Run: `npm test -- src/ui && npm run typecheck && npm run lint && npx vite build`
Expected: PASS; the build output CSS contains `--c-accent` and no palette colours (`grep -c "zinc" dist/assets/*.css` prints 0).

- [ ] **Step 5: Commit**

```bash
git add app/src/ui app/src/main.tsx
git commit -m "UI: colour roles with contrast test, themes, density, fonts and entity glyphs"
```

(This task has no screen of its own; its finish review happens with Task 8's catalogue preview.)

---
### Task 8: Core components and the dev catalogue

**Files:**
- Create: `app/src/ui/components/button.tsx`, `icon-button.tsx`, `input.tsx`, `field.tsx`, `select.tsx`, `badge.tsx`, `kbd.tsx`, `skeleton.tsx`, `empty-state.tsx`, `theme-scope.tsx`
- Modify: `app/src/ui/index.ts` (export them)
- Create: `app/src/dev/Catalogue.tsx`, `app/e2e/shots.spec.ts`
- Move: `app/src/App.tsx` → `app/src/LegacyApp.tsx` (unchanged M0 screen, deleted in Task 13); create a new `app/src/App.tsx` that routes `#catalogue`
- Test: `app/src/ui/components/components.test.tsx`

**Interfaces:**
- Consumes: Task 7 (`cn`, `formatCount`, `CountQuality`, `shortcutLabel`, `applyTheme`, `ThemeName`, `Density`, Tailwind role utilities).
- Produces:
  - `Button(props: ComponentProps<"button"> & { variant?: "default" | "quiet" | "primary" | "danger"; size?: "sm" | "md" | "lg" })`, `buttonVariants`
  - `IconButton(props: ComponentProps<"button"> & { label: string })` — `label` becomes `aria-label`
  - `Input(props: ComponentProps<"input"> & { mono?: boolean })`
  - `Field({ label, hint?, error?, children: (ids: FieldControlProps) => ReactNode })` with `FieldControlProps = { id: string; "aria-describedby"?: string; "aria-invalid"?: true }`
  - `Select`, `SelectTrigger`, `SelectValue`, `SelectContent`, `SelectItem` (Radix, shadcn structure)
  - `Badge({ tone?: "neutral" | "accent" | "attention" })`, `ProdBadge()`, `CountBadge({ value: number | null | undefined; quality?: CountQuality; attention?: boolean; className? })`
  - `Kbd({ shortcut: string })`
  - `Skeleton({ className? })`, `SkeletonRows({ rows?: number; columns?: number })`
  - `EmptyState({ title: string; body?: ReactNode; action?: ReactNode; tone?: "neutral" | "error" })`
  - `ThemeScope({ theme: ThemeName; density: Density; children })` — applies a theme to a subtree (catalogue side-by-side)
  - `app/e2e/shots.spec.ts` with an exported-by-convention `shots` array later tasks append to

**Finish review procedure (used by Tasks 8–19).** With the `impeccable` and `design-taste-frontend` skills loaded:
1. Run `cd app && npm run shots` (WebKit, the engine Tauri uses on macOS and Linux). Screenshots land in `app/test-results/shots/`.
2. Dispatch the `impeccable-finish-reviewer` agent with: the direction contract (`~/.claude/skills/impeccable/scripts/impeccable surface-brief read app/src/shell`), `docs/design/m1b-mockup-a-dark.png` and `-light.png`, the task's screenshots, the list of components or screens the task changed, and the Global Constraints' Colour/Type/Shape/Motion/Banned lines.
3. Fix every material finding, re-run the shots and re-review until the verdict passes. Record the verdict (one line) in the task's commit message.

- [ ] **Step 1: Write the failing tests**

`app/src/ui/components/components.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Button } from "./button";
import { IconButton } from "./icon-button";
import { Field } from "./field";
import { Input } from "./input";
import { CountBadge, ProdBadge } from "./badge";
import { Kbd } from "./kbd";
import { EmptyState } from "./empty-state";
import { SkeletonRows } from "./skeleton";

describe("Button", () => {
  it("is a type=button that clicks and disables", async () => {
    const onClick = vi.fn();
    const { rerender } = render(<Button onClick={onClick}>Refresh</Button>);
    const b = screen.getByRole("button", { name: "Refresh" });
    expect(b).toHaveAttribute("type", "button");
    await userEvent.click(b);
    expect(onClick).toHaveBeenCalledOnce();
    rerender(<Button onClick={onClick} disabled>Refresh</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("uses the accent only for the danger variant", () => {
    render(<><Button>A</Button><Button variant="danger">B</Button></>);
    expect(screen.getByRole("button", { name: "A" }).className).not.toContain("bg-accent");
    expect(screen.getByRole("button", { name: "B" }).className).toContain("bg-accent");
  });
});

describe("IconButton", () => {
  it("names the button with its label", () => {
    render(<IconButton label="Refresh all"><svg /></IconButton>);
    expect(screen.getByRole("button", { name: "Refresh all" })).toBeInTheDocument();
  });
});

describe("Field", () => {
  it("wires label, hint and error to the control", () => {
    render(
      <Field label="Name" hint="Shown in the connection list" error="A profile named Logistics already exists">
        {(ids) => <Input {...ids} defaultValue="Logistics" />}
      </Field>,
    );
    const input = screen.getByLabelText("Name");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription(/already exists/);
    expect(input).toHaveAccessibleDescription(/Shown in the connection list/);
  });
});

describe("CountBadge", () => {
  it("formats and marks attention only above zero", () => {
    const { rerender } = render(<CountBadge value={3417} attention />);
    expect(screen.getByText("3,417").className).toContain("text-accent");
    rerender(<CountBadge value={0} attention />);
    expect(screen.getByText("0").className).not.toContain("text-accent");
    rerender(<CountBadge value={null} attention />);
    expect(screen.getByText("?")).toBeInTheDocument();
    rerender(<CountBadge value={120} quality={{ approximate: true }} />);
    expect(screen.getByText("~120")).toBeInTheDocument();
  });
});

describe("ProdBadge, Kbd, EmptyState, SkeletonRows", () => {
  it("render accessibly", () => {
    render(
      <>
        <ProdBadge />
        <Kbd shortcut="mod+k" />
        <EmptyState title="No dead letters" body="Messages that fail delivery land here." tone="neutral" />
        <SkeletonRows rows={3} />
      </>,
    );
    expect(screen.getByText("PROD")).toHaveAttribute("title", "Production profile");
    expect(screen.getByText(/K$/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No dead letters" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Loading" }).querySelectorAll("[data-skeleton-row]")).toHaveLength(3);
  });
});
```

Run: `npm test -- src/ui/components`
Expected: FAIL (modules missing).

- [ ] **Step 2: Implement the components**

`app/src/ui/components/button.tsx`:

```tsx
import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export const buttonVariants = cva(
  "inline-flex select-none items-center justify-center gap-1.5 whitespace-nowrap rounded-ctl text-12 font-normal transition-colors duration-100 disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border border-line bg-field text-fg hover:bg-hover active:bg-selection",
        quiet: "text-fg hover:bg-hover active:bg-selection",
        primary: "bg-fg text-content hover:opacity-90 active:opacity-80",
        danger: "bg-accent text-accent-ink hover:opacity-90 active:opacity-80",
      },
      size: { sm: "h-[22px] px-1.5", md: "h-6 px-2", lg: "h-7 px-3 text-13" },
    },
    defaultVariants: { variant: "default", size: "md" },
  },
);

export function Button({
  className,
  variant,
  size,
  type = "button",
  ...props
}: ComponentProps<"button"> & VariantProps<typeof buttonVariants>) {
  return <button type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}
```

`app/src/ui/components/icon-button.tsx`:

```tsx
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export function IconButton({ label, className, type = "button", ...props }: ComponentProps<"button"> & { label: string }) {
  return (
    <button
      type={type}
      aria-label={label}
      className={cn(
        "inline-grid size-6 shrink-0 place-items-center rounded-ctl text-icon hover:bg-hover hover:text-fg active:bg-selection disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5",
        className,
      )}
      {...props}
    />
  );
}
```

`app/src/ui/components/input.tsx`:

```tsx
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export function Input({ className, mono, ...props }: ComponentProps<"input"> & { mono?: boolean }) {
  return (
    <input
      spellCheck={false}
      autoComplete="off"
      className={cn(
        "h-6 w-full min-w-0 rounded-ctl border border-line bg-field px-2 text-12 text-fg outline-none placeholder:text-faint",
        "focus-visible:border-fg focus-visible:outline-none aria-invalid:border-accent disabled:opacity-45",
        mono && "font-mono text-11",
        className,
      )}
      {...props}
    />
  );
}
```

`app/src/ui/components/field.tsx`:

```tsx
import { CircleAlert } from "lucide-react";
import { useId, type ReactNode } from "react";

export interface FieldControlProps {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: true;
}

export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string;
  hint?: ReactNode;
  error?: ReactNode;
  children: (ids: FieldControlProps) => ReactNode;
}) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-12 font-semibold text-muted">
        {label}
      </label>
      {children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined })}
      {hint && <p id={hintId} className="text-11 text-muted">{hint}</p>}
      {error && (
        <p id={errorId} className="flex items-start gap-1.5 text-12 text-fg">
          <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
          <span>{error}</span>
        </p>
      )}
    </div>
  );
}
```

`app/src/ui/components/select.tsx`:

```tsx
import { Check, ChevronDown } from "lucide-react";
import { Select as SelectPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export const Select = SelectPrimitive.Root;
export const SelectValue = SelectPrimitive.Value;

export function SelectTrigger({ className, children, ...props }: ComponentProps<typeof SelectPrimitive.Trigger>) {
  return (
    <SelectPrimitive.Trigger
      className={cn(
        "flex h-6 w-full items-center justify-between gap-2 rounded-ctl border border-line bg-field px-2 text-12 text-fg outline-none",
        "focus-visible:border-fg data-[placeholder]:text-faint aria-invalid:border-accent disabled:opacity-45",
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDown aria-hidden="true" className="size-3.5 text-icon" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

export function SelectContent({ className, children, position = "popper", ...props }: ComponentProps<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        position={position}
        sideOffset={2}
        className={cn(
          "z-50 max-h-72 min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-ctl border border-line bg-side py-1 text-12 text-fg shadow-float",
          className,
        )}
        {...props}
      >
        <SelectPrimitive.Viewport>{children}</SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

export function SelectItem({ className, children, ...props }: ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      className={cn(
        "relative flex h-row cursor-default items-center pl-6 pr-3 outline-none data-[highlighted]:bg-selection data-[disabled]:opacity-45",
        className,
      )}
      {...props}
    >
      <SelectPrimitive.ItemIndicator className="absolute left-1.5 inline-flex">
        <Check aria-hidden="true" className="size-3.5" />
      </SelectPrimitive.ItemIndicator>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
}
```

`app/src/ui/components/badge.tsx`:

```tsx
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";
import { formatCount, type CountQuality } from "../lib/format";

const tones = {
  neutral: "border border-line text-muted",
  accent: "bg-accent text-accent-ink",
  attention: "bg-accent-tint text-accent",
} as const;

export function Badge({ tone = "neutral", className, ...props }: ComponentProps<"span"> & { tone?: keyof typeof tones }) {
  return (
    <span
      className={cn("inline-flex h-4 items-center rounded-[2px] px-1 text-[10px] font-semibold leading-4 tracking-[0.04em]", tones[tone], className)}
      {...props}
    />
  );
}

export function ProdBadge({ className }: { className?: string }) {
  return (
    <Badge tone="accent" title="Production profile" className={className}>
      PROD
    </Badge>
  );
}

/** A count in a tree row or tab: `?` unknown, `~N` approximate, amber when `attention` and above zero. */
export function CountBadge({
  value,
  quality,
  attention,
  className,
}: {
  value: number | null | undefined;
  quality?: CountQuality;
  attention?: boolean;
  className?: string;
}) {
  const hot = attention && value != null && value > 0;
  return (
    <span className={cn("text-11 tabular-nums", hot ? "text-accent" : "text-faint", className)}>
      {formatCount(value, quality)}
    </span>
  );
}
```

(`text-[10px]` is the one sub-11px size, used only for the PROD/badge caps; it is allowed because badges are labels, not running text.)

`app/src/ui/components/kbd.tsx`:

```tsx
import { cn } from "../lib/cn";
import { shortcutLabel } from "../lib/platform";

export function Kbd({ shortcut, className }: { shortcut: string; className?: string }) {
  return <kbd className={cn("font-mono text-11 text-muted", className)}>{shortcutLabel(shortcut)}</kbd>;
}
```

`app/src/ui/components/skeleton.tsx`:

```tsx
import { cn } from "../lib/cn";

/** A static placeholder bar (no animation: only sheets, drawer and popovers move). */
export function Skeleton({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn("block h-2.5 rounded-[2px] bg-hover", className)} />;
}

const WIDTHS = ["w-[62%]", "w-[48%]", "w-[71%]", "w-[39%]", "w-[56%]", "w-[66%]", "w-[44%]", "w-[58%]"];

/** Rows shaped like the content that is loading. */
export function SkeletonRows({ rows = 8, columns = 1 }: { rows?: number; columns?: number }) {
  return (
    <div role="status" aria-label="Loading" className="flex flex-col">
      {Array.from({ length: rows }, (_, r) => (
        <div key={r} data-skeleton-row className="flex h-row items-center gap-4 px-3">
          {Array.from({ length: columns }, (_, c) => (
            <Skeleton key={c} className={cn(c === 0 ? WIDTHS[r % WIDTHS.length] : "w-10", c === 0 && "flex-1")} />
          ))}
        </div>
      ))}
    </div>
  );
}
```

`app/src/ui/components/empty-state.tsx`:

```tsx
import { CircleAlert } from "lucide-react";
import type { ReactNode } from "react";

export function EmptyState({
  title,
  body,
  action,
  tone = "neutral",
}: {
  title: string;
  body?: ReactNode;
  action?: ReactNode;
  tone?: "neutral" | "error";
}) {
  return (
    <div className="flex max-w-[440px] flex-col gap-1.5 px-4 py-5">
      <h3 className="m-0 flex items-center gap-1.5 text-13 font-semibold text-fg">
        {tone === "error" && <CircleAlert aria-hidden="true" className="size-3.5 text-accent" />}
        {title}
      </h3>
      {body && <div className="text-12 text-muted">{body}</div>}
      {action && <div className="pt-1.5">{action}</div>}
    </div>
  );
}
```

`app/src/ui/components/theme-scope.tsx`:

```tsx
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { applyTheme, type Density, type ThemeName } from "../theme";

/** Applies a theme to one subtree (side-by-side previews). */
export function ThemeScope({ theme, density, children }: { theme: ThemeName; density: Density; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (ref.current) applyTheme(theme, density, ref.current);
  }, [theme, density]);
  return (
    <div ref={ref} className="bg-content text-fg">
      {children}
    </div>
  );
}
```

Append to `app/src/ui/index.ts`:

```ts
export * from "./components/button";
export * from "./components/icon-button";
export * from "./components/input";
export * from "./components/field";
export * from "./components/select";
export * from "./components/badge";
export * from "./components/kbd";
export * from "./components/skeleton";
export * from "./components/empty-state";
export * from "./components/theme-scope";
```

- [ ] **Step 3: Run the tests**

Run: `npm test -- src/ui && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 4: Catalogue, routing and screenshot spec**

`git mv app/src/App.tsx app/src/LegacyApp.tsx` (rename its default export to `LegacyApp`). New `app/src/App.tsx`:

```tsx
import { lazy, Suspense } from "react";
import LegacyApp from "./LegacyApp";

const catalogueAllowed = import.meta.env.DEV || import.meta.env.VITE_DISPATCHED_MOCK === "1";
const Catalogue = catalogueAllowed ? lazy(() => import("./dev/Catalogue")) : null;

export default function App() {
  if (Catalogue && window.location.hash === "#catalogue") {
    return (
      <Suspense fallback={null}>
        <Catalogue />
      </Suspense>
    );
  }
  return <LegacyApp />;
}
```

`app/src/dev/Catalogue.tsx`:

```tsx
import { useState, type ReactNode } from "react";
import { RefreshCw } from "lucide-react";
import {
  Button, CountBadge, EmptyState, EntityGlyph, Field, IconButton, Input, Kbd, ProdBadge,
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue, SkeletonRows, ThemeScope,
  type Density, type ThemeName,
} from "@/ui";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-b border-line px-4 py-3">
      <h2 className="m-0 mb-2 text-12 font-semibold text-muted">{title}</h2>
      <div className="flex flex-wrap items-start gap-3">{children}</div>
    </section>
  );
}

function Column({ theme, density }: { theme: ThemeName; density: Density }) {
  return (
    <ThemeScope theme={theme} density={density}>
      <div className="min-h-screen">
        <Section title="Buttons">
          <Button>Default</Button>
          <Button variant="quiet">Quiet</Button>
          <Button variant="primary">Save profile</Button>
          <Button variant="danger">Delete profile</Button>
          <Button disabled>Disabled</Button>
          <IconButton label="Refresh"><RefreshCw /></IconButton>
        </Section>
        <Section title="Inputs">
          <div className="w-64">
            <Field label="Name" hint="Shown in the connection list">{(ids) => <Input {...ids} placeholder="Logistics" />}</Field>
          </div>
          <div className="w-64">
            <Field label="Connection string" error="This is not a Service Bus connection string.">
              {(ids) => <Input {...ids} mono defaultValue="Endpoint=sb://…" />}
            </Field>
          </div>
          <div className="w-48">
            <Select defaultValue="amqp">
              <SelectTrigger aria-label="Transport"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="amqp">AMQP</SelectItem>
                <SelectItem value="websockets">AMQP over WebSockets</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </Section>
        <Section title="Badges and counts">
          <ProdBadge />
          <CountBadge value={3417} attention />
          <CountBadge value={0} attention />
          <CountBadge value={2906} />
          <CountBadge value={null} />
          <CountBadge value={120} quality={{ approximate: true }} />
          <Kbd shortcut="mod+k" />
          <Kbd shortcut="f5" />
        </Section>
        <Section title="Glyphs">
          {(["namespace", "queue", "topic", "subscription", "rule"] as const).map((k) => (
            <span key={k} className="flex items-center gap-1.5 text-icon"><EntityGlyph kind={k} /><span className="text-fg">{k}</span></span>
          ))}
        </Section>
        <Section title="Loading and empty">
          <div className="w-72 border border-line"><SkeletonRows rows={4} columns={2} /></div>
          <EmptyState title="No dead letters" body="Messages that fail delivery land here. This subscription has none." />
          <EmptyState tone="error" title="Can't list subscriptions" body="This identity needs Listen or Manage on shipment-events." action={<Button>Retry</Button>} />
        </Section>
      </div>
    </ThemeScope>
  );
}

export default function Catalogue() {
  const [density, setDensity] = useState<Density>("compact");
  return (
    <div className="h-full overflow-auto">
      <div className="flex items-center gap-2 border-b border-line bg-chrome px-4 py-2">
        <span className="font-semibold">Component catalogue</span>
        <Button variant="quiet" onClick={() => setDensity(density === "compact" ? "comfortable" : "compact")}>
          Density: {density}
        </Button>
      </div>
      <div className="grid grid-cols-2">
        <Column theme="dark" density={density} />
        <Column theme="light" density={density} />
      </div>
    </div>
  );
}
```

`app/e2e/shots.spec.ts`:

```ts
import { test } from "@playwright/test";

/** Screens captured for impeccable finish reviews. Later tasks append entries. */
const shots: { name: string; path: string }[] = [{ name: "catalogue", path: "/#catalogue" }];

for (const colorScheme of ["dark", "light"] as const) {
  for (const s of shots) {
    test(`${s.name} (${colorScheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
      await page.goto(s.path);
      await page.waitForLoadState("networkidle");
      await page.screenshot({ path: `test-results/shots/${s.name}-${colorScheme}.png`, fullPage: true });
    });
  }
}
```

Exclude it from the normal `e2e` run: in `app/playwright.config.ts` add `testIgnore: process.env.SHOTS ? [] : ["**/shots.spec.ts"]`, and change the `shots` script to `SHOTS=1 playwright test e2e/shots.spec.ts --project=webkit`.

- [ ] **Step 5: Finish review**

Run the Finish review procedure for: Button, IconButton, Input, Field, Select, Badge/ProdBadge/CountBadge, Kbd, Skeleton, EmptyState, glyphs, tokens in both themes and both densities (catalogue screenshots).

- [ ] **Step 6: Commit**

```bash
git add app/src app/e2e app/playwright.config.ts app/package.json
git commit -m "UI: core components restyled for direction A, dev catalogue (finish review: <verdict>)"
```

---

### Task 9: Overlay and navigation components — tabs, tooltip, menus, sheet, palette, resizable panels

**Files:**
- Create: `app/src/ui/components/tabs.tsx`, `tooltip.tsx`, `dropdown-menu.tsx`, `context-menu.tsx`, `sheet.tsx`, `command.tsx`, `resizable.tsx`
- Modify: `app/src/ui/index.ts`, `app/src/dev/Catalogue.tsx` (an "Overlays" section)
- Test: `app/src/ui/components/overlays.test.tsx`

**Interfaces:**
- Consumes: Task 8 (`cn`, `Kbd`, `IconButton`, `Button`).
- Produces:
  - `Tabs`, `TabsList({ variant?: "strip" | "underline" })`, `TabsTrigger`, `TabsContent` (Radix Tabs; `strip` = entity tabs with a 2px top marker on the active tab, `underline` = sub-tabs with a 2px bottom marker)
  - `TooltipProvider`, `Tooltip`, `TooltipTrigger`, `TooltipContent`, and `Hint({ label, shortcut?, children })` (trigger + content in one)
  - `DropdownMenu`, `DropdownMenuTrigger`, `DropdownMenuContent`, `DropdownMenuItem`, `DropdownMenuSeparator`, `DropdownMenuLabel`, `MenuShortcut`
  - `ContextMenu`, `ContextMenuTrigger`, `ContextMenuContent`, `ContextMenuItem`, `ContextMenuSeparator`
  - `Sheet({ open, onOpenChange, title, description?, children, footer?, width? })` — right-edge side sheet (Radix Dialog, focus trapped, Esc closes, transparent overlay so the app stays visible, 150ms slide)
  - `CommandPaletteDialog({ open, onOpenChange, children })`, `CommandInput`, `CommandList`, `CommandEmpty`, `CommandGroup`, `CommandItem`, `CommandShortcut` (cmdk)
  - `PanelGroup`, `Panel`, `ResizeHandle` (react-resizable-panels, 1px line, 5px hit area)

- [ ] **Step 1: Write the failing tests**

`app/src/ui/components/overlays.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
import { Sheet } from "./sheet";
import { CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog } from "./command";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./dropdown-menu";
import { Button } from "./button";

describe("Tabs", () => {
  it("switch with arrow keys", async () => {
    render(
      <Tabs defaultValue="a">
        <TabsList variant="underline" aria-label="Views">
          <TabsTrigger value="a">Overview</TabsTrigger>
          <TabsTrigger value="b">Dead-letter</TabsTrigger>
        </TabsList>
        <TabsContent value="a">A body</TabsContent>
        <TabsContent value="b">B body</TabsContent>
      </Tabs>,
    );
    await userEvent.click(screen.getByRole("tab", { name: "Overview" }));
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Dead-letter" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("B body")).toBeVisible();
  });
});

describe("Sheet", () => {
  it("is a labelled dialog that closes on Escape", async () => {
    const onOpenChange = vi.fn();
    render(
      <Sheet open onOpenChange={onOpenChange} title="Add connection" description="Saved on this computer.">
        <input aria-label="Name" />
      </Sheet>,
    );
    expect(screen.getByRole("dialog", { name: "Add connection" })).toHaveAccessibleDescription("Saved on this computer.");
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

describe("Command palette", () => {
  it("filters and runs an item", async () => {
    const onSelect = vi.fn();
    render(
      <CommandPaletteDialog open onOpenChange={() => {}}>
        <CommandInput placeholder="Type a command" />
        <CommandList>
          <CommandGroup heading="App">
            <CommandItem onSelect={onSelect}>Toggle theme</CommandItem>
            <CommandItem onSelect={() => {}}>Refresh</CommandItem>
          </CommandGroup>
        </CommandList>
      </CommandPaletteDialog>,
    );
    await userEvent.type(screen.getByPlaceholderText("Type a command"), "theme");
    expect(screen.queryByText("Refresh")).not.toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledOnce();
  });
});

describe("DropdownMenu", () => {
  it("opens from its trigger and selects an item", async () => {
    const onSelect = vi.fn();
    render(
      <DropdownMenu>
        <DropdownMenuTrigger asChild><Button>Logistics</Button></DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={onSelect}>Manage profiles</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Logistics" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Manage profiles" }));
    expect(onSelect).toHaveBeenCalledOnce();
  });
});
```

Run: `npm test -- src/ui/components/overlays`
Expected: FAIL (modules missing).

- [ ] **Step 2: Implement**

`app/src/ui/components/tabs.tsx`:

```tsx
import { Tabs as TabsPrimitive } from "radix-ui";
import { createContext, useContext, type ComponentProps } from "react";
import { cn } from "../lib/cn";

type Variant = "strip" | "underline";
const VariantContext = createContext<Variant>("underline");

export const Tabs = TabsPrimitive.Root;

export function TabsList({ variant = "underline", className, ...props }: ComponentProps<typeof TabsPrimitive.List> & { variant?: Variant }) {
  return (
    <VariantContext.Provider value={variant}>
      <TabsPrimitive.List
        className={cn(
          "flex min-w-0 items-stretch overflow-x-auto [scrollbar-width:none]",
          variant === "strip" ? "h-[30px] bg-chrome" : "h-7 gap-0.5 px-2.5",
          className,
        )}
        {...props}
      />
    </VariantContext.Provider>
  );
}

export function TabsTrigger({ className, ...props }: ComponentProps<typeof TabsPrimitive.Trigger>) {
  const variant = useContext(VariantContext);
  return (
    <TabsPrimitive.Trigger
      className={cn(
        "group flex shrink-0 items-center gap-1.5 whitespace-nowrap text-12 text-muted outline-none hover:text-fg data-[state=active]:text-fg",
        variant === "strip"
          ? "border-r border-line px-3 data-[state=active]:bg-content data-[state=active]:shadow-[inset_0_2px_0_var(--c-fg)]"
          : "px-2 data-[state=active]:shadow-[inset_0_-2px_0_var(--c-fg)]",
        "focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg",
        className,
      )}
      {...props}
    />
  );
}

export function TabsContent({ className, ...props }: ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content className={cn("min-h-0 flex-1 outline-none", className)} {...props} />;
}
```

`app/src/ui/components/tooltip.tsx`:

```tsx
import { Tooltip as TooltipPrimitive } from "radix-ui";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../lib/cn";
import { Kbd } from "./kbd";

export function TooltipProvider(props: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={450} skipDelayDuration={150} {...props} />;
}
export const Tooltip = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export function TooltipContent({ className, sideOffset = 4, ...props }: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        sideOffset={sideOffset}
        className={cn("z-50 flex items-center gap-2 rounded-ctl border border-line bg-side px-2 py-1 text-12 text-fg shadow-float", className)}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
}

/** A trigger with a tooltip naming it, and its shortcut if it has one. */
export function Hint({ label, shortcut, children }: { label: string; shortcut?: string; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent>
        <span>{label}</span>
        {shortcut && <Kbd shortcut={shortcut} />}
      </TooltipContent>
    </Tooltip>
  );
}
```

`app/src/ui/components/dropdown-menu.tsx`:

```tsx
import { DropdownMenu as M } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export const menuContent =
  "z-50 min-w-[200px] overflow-hidden rounded-ctl border border-line bg-side py-1 text-12 text-fg shadow-float";
export const menuItem =
  "relative flex h-row cursor-default items-center gap-2 px-3 outline-none data-[highlighted]:bg-selection data-[disabled]:opacity-45 [&_svg]:size-3.5 [&_svg]:text-icon";

export const DropdownMenu = M.Root;
export const DropdownMenuTrigger = M.Trigger;

export function DropdownMenuContent({ className, sideOffset = 2, align = "start", ...props }: ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content sideOffset={sideOffset} align={align} className={cn(menuContent, className)} {...props} />
    </M.Portal>
  );
}
export function DropdownMenuItem({ className, ...props }: ComponentProps<typeof M.Item>) {
  return <M.Item className={cn(menuItem, className)} {...props} />;
}
export function DropdownMenuSeparator({ className, ...props }: ComponentProps<typeof M.Separator>) {
  return <M.Separator className={cn("my-1 h-px bg-line", className)} {...props} />;
}
export function DropdownMenuLabel({ className, ...props }: ComponentProps<typeof M.Label>) {
  return <M.Label className={cn("px-3 py-1 text-11 text-muted", className)} {...props} />;
}
export function MenuShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span className={cn("ml-auto pl-4 font-mono text-11 text-muted", className)} {...props} />;
}
```

`app/src/ui/components/context-menu.tsx`:

```tsx
import { ContextMenu as M } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";
import { menuContent, menuItem } from "./dropdown-menu";

export const ContextMenu = M.Root;
export const ContextMenuTrigger = M.Trigger;

export function ContextMenuContent({ className, ...props }: ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content className={cn(menuContent, className)} {...props} />
    </M.Portal>
  );
}
export function ContextMenuItem({ className, ...props }: ComponentProps<typeof M.Item>) {
  return <M.Item className={cn(menuItem, className)} {...props} />;
}
export function ContextMenuSeparator({ className, ...props }: ComponentProps<typeof M.Separator>) {
  return <M.Separator className={cn("my-1 h-px bg-line", className)} {...props} />;
}
```

`app/src/ui/components/sheet.tsx`:

```tsx
import { X } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Dialog } from "radix-ui";
import type { ReactNode } from "react";
import { IconButton } from "./icon-button";

const EASE = [0.16, 1, 0.3, 1] as const;

/** A right-edge side sheet. The app stays visible behind it (no dimming). */
export function Sheet({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  width = 420,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  const reduce = useReducedMotion();
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open && (
          <Dialog.Portal forceMount>
            <Dialog.Overlay className="fixed inset-0 z-40 bg-transparent" />
            <Dialog.Content asChild forceMount {...(description ? {} : { "aria-describedby": undefined })}>
              <motion.div
                initial={reduce ? false : { x: 24, opacity: 0 }}
                animate={{ x: 0, opacity: 1 }}
                exit={reduce ? { opacity: 0 } : { x: 24, opacity: 0 }}
                transition={{ duration: 0.15, ease: EASE }}
                style={{ width }}
                className="fixed inset-y-0 right-0 z-50 flex max-w-full flex-col border-l border-line bg-side text-fg shadow-float outline-none"
              >
                <div className="flex items-start gap-2 border-b border-line px-4 py-3">
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <Dialog.Title className="m-0 text-14 font-semibold">{title}</Dialog.Title>
                    {description ? (
                      <Dialog.Description className="m-0 text-12 text-muted">{description}</Dialog.Description>
                    ) : null}
                  </div>
                  <Dialog.Close asChild>
                    <IconButton label="Close"><X /></IconButton>
                  </Dialog.Close>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">{children}</div>
                {footer && <div className="flex justify-end gap-2 border-t border-line px-4 py-3">{footer}</div>}
              </motion.div>
            </Dialog.Content>
          </Dialog.Portal>
        )}
      </AnimatePresence>
    </Dialog.Root>
  );
}
```

(Without a description, `aria-describedby={undefined}` tells Radix the dialog intentionally has none.)

`app/src/ui/components/command.tsx`:

```tsx
import { Command as Cmdk } from "cmdk";
import { Search } from "lucide-react";
import { Dialog } from "radix-ui";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../lib/cn";

/** The ⌘K palette: centred near the top, no dimming, 560px wide. */
export function CommandPaletteDialog({ open, onOpenChange, children, label = "Command palette" }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
  label?: string;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-transparent" />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[72px] z-50 w-[560px] max-w-[calc(100vw-32px)] -translate-x-1/2 overflow-hidden rounded-ctl border border-line bg-side text-fg shadow-float outline-none"
        >
          <Dialog.Title className="sr-only">{label}</Dialog.Title>
          <Cmdk label={label} loop className="flex flex-col">{children}</Cmdk>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function CommandInput({ className, ...props }: ComponentProps<typeof Cmdk.Input>) {
  return (
    <div className="flex h-9 items-center gap-2 border-b border-line px-3">
      <Search aria-hidden="true" className="size-3.5 text-icon" />
      <Cmdk.Input className={cn("h-full flex-1 bg-transparent text-13 text-fg outline-none placeholder:text-faint", className)} {...props} />
    </div>
  );
}

export function CommandList({ className, ...props }: ComponentProps<typeof Cmdk.List>) {
  return <Cmdk.List className={cn("max-h-[360px] overflow-y-auto py-1", className)} {...props} />;
}

export function CommandEmpty(props: ComponentProps<typeof Cmdk.Empty>) {
  return <Cmdk.Empty className="px-3 py-3 text-12 text-muted" {...props} />;
}

export function CommandGroup({ className, ...props }: ComponentProps<typeof Cmdk.Group>) {
  return (
    <Cmdk.Group
      className={cn("[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:pb-0.5 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:text-11 [&_[cmdk-group-heading]]:text-muted", className)}
      {...props}
    />
  );
}

export function CommandItem({ className, ...props }: ComponentProps<typeof Cmdk.Item>) {
  return (
    <Cmdk.Item
      className={cn("flex h-7 cursor-default items-center gap-2 px-3 text-12 outline-none data-[selected=true]:bg-selection [&_svg]:size-3.5 [&_svg]:text-icon", className)}
      {...props}
    />
  );
}

export function CommandShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span className={cn("ml-auto font-mono text-11 text-muted", className)} {...props} />;
}
```

`app/src/ui/components/resizable.tsx`:

```tsx
import type { ComponentProps } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { cn } from "../lib/cn";

export { Panel, PanelGroup };

/** 1px line with a 5px invisible hit area; highlights on hover and drag. */
export function ResizeHandle({ className, ...props }: ComponentProps<typeof PanelResizeHandle>) {
  return (
    <PanelResizeHandle
      className={cn(
        "relative bg-line outline-none data-[panel-group-direction=horizontal]:w-px data-[panel-group-direction=vertical]:h-px",
        "after:absolute after:inset-y-0 after:-left-[2px] after:w-[5px] data-[panel-group-direction=vertical]:after:inset-x-0 data-[panel-group-direction=vertical]:after:-top-[2px] data-[panel-group-direction=vertical]:after:h-[5px] data-[panel-group-direction=vertical]:after:w-auto",
        "hover:bg-muted data-[resize-handle-state=drag]:bg-fg",
        className,
      )}
      {...props}
    />
  );
}
```

Append to `app/src/ui/index.ts`: exports for `./components/tabs`, `./components/tooltip`, `./components/dropdown-menu`, `./components/context-menu`, `./components/sheet`, `./components/command`, `./components/resizable`.

Add a catalogue section "Overlays" to each column: a `Tabs` strip with three entity tabs, an `underline` `Tabs` with the nine sub-tab names, a `DropdownMenu` with three items and shortcuts, a button that opens a `Sheet` ("Add connection" with two `Field`s and a footer), a button that opens the `CommandPaletteDialog` with two groups, and a horizontal `PanelGroup` 400px tall with two panels and a `ResizeHandle`. Wrap the catalogue root in `TooltipProvider`.

Add to `app/e2e/shots.spec.ts` `shots`: `{ name: "catalogue-overlays", path: "/#catalogue" }` with an extra step for this entry that clicks the "Open sheet" button in the dark column before the screenshot (add an optional `prepare?: (page) => Promise<void>` field to the shot type and call it before `screenshot`).

- [ ] **Step 3: Run tests**

Run: `npm test -- src/ui && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 4: Finish review**

Run the Finish review procedure for: entity tab strip, sub-tab underline, tooltip, dropdown menu, context menu, sheet, command palette, resize handles.

- [ ] **Step 5: Commit**

```bash
git add app/src app/e2e
git commit -m "UI: tabs, tooltips, menus, side sheet, command palette and resizable panels (finish review: <verdict>)"
```

---
### Task 10: Tree — virtualised, keyboard-first

**Files:**
- Create: `app/src/ui/tree/Tree.tsx`
- Modify: `app/src/ui/index.ts` (`export * from "./tree/Tree";`), `app/src/dev/Catalogue.tsx` (a "Tree" section)
- Test: `app/src/ui/tree/Tree.test.tsx`

**Interfaces:**
- Consumes: Task 7 (`cn`).
- Produces:
  - `interface TreeNode { id: string; parentId: string | null; depth: number; label: string; expandable: boolean; expanded: boolean }`
  - `interface TreeRowState { selected: boolean }`
  - `Tree<N extends TreeNode>(props: { label: string; nodes: N[]; selectedId: string | null; onSelect(id: string): void; onActivate(id: string): void; onToggle(id: string, expanded: boolean): void; renderRow(node: N, state: TreeRowState): ReactNode; rowHeight: number; className?: string; initialRect?: { width: number; height: number } })`
  - `nodes` is the flattened list of *visible* nodes in display order (the shell builds it). The tree draws indentation (14px per level) and the chevron; `renderRow` draws everything after the chevron.
  - Keys: ↓/`j` next, ↑/`k` previous, Home/End, → expand or move to first child, ← collapse or move to parent, Enter activate, printable characters type-ahead (500ms buffer, case-insensitive prefix, searching forward from the current row and wrapping). Single tab stop with `aria-activedescendant`. Click selects, double-click activates, clicking the chevron toggles. Each row carries `data-node-id={node.id}` so callers can map DOM events (context menus) back to nodes.

- [ ] **Step 1: Write the failing tests**

`app/src/ui/tree/Tree.test.tsx`:

```tsx
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { Tree, type TreeNode } from "./Tree";

const RECT = { width: 300, height: 220 };

function node(id: string, parentId: string | null, depth: number, expandable = false, expanded = false): TreeNode {
  return { id, parentId, depth, label: id, expandable, expanded };
}

const NODES: TreeNode[] = [
  node("queues", null, 0, true, true),
  node("invoice-generate", "queues", 1),
  node("label-print", "queues", 1),
  node("topics", null, 0, true, true),
  node("shipment-events", "topics", 1, true, false),
  node("tracking-sync", "topics", 1),
];

function Harness(props: { nodes?: TreeNode[]; onToggle?: (id: string, e: boolean) => void; onActivate?: (id: string) => void }) {
  const [sel, setSel] = useState<string | null>("queues");
  return (
    <Tree
      label="Namespace"
      nodes={props.nodes ?? NODES}
      selectedId={sel}
      onSelect={setSel}
      onActivate={props.onActivate ?? (() => {})}
      onToggle={props.onToggle ?? (() => {})}
      renderRow={(n) => <span>{n.label}</span>}
      rowHeight={22}
      initialRect={RECT}
    />
  );
}

const selected = () => screen.getAllByRole("treeitem").find((r) => r.getAttribute("aria-selected") === "true");

describe("Tree", () => {
  it("renders_only_visible_rows_of_5000", () => {
    const many = Array.from({ length: 5000 }, (_, i) => node(`q-${i}`, null, 0));
    render(<Harness nodes={many} />);
    expect(screen.getAllByRole("treeitem").length).toBeLessThan(60);
  });

  it("exposes tree semantics", () => {
    render(<Harness />);
    const tree = screen.getByRole("tree", { name: "Namespace" });
    expect(tree).toHaveAttribute("tabindex", "0");
    const topics = within(tree).getByRole("treeitem", { name: "topics" });
    expect(topics).toHaveAttribute("aria-level", "1");
    expect(topics).toHaveAttribute("aria-expanded", "true");
    expect(within(tree).getByRole("treeitem", { name: "tracking-sync" })).toHaveAttribute("aria-level", "2");
    expect(within(tree).getByRole("treeitem", { name: "tracking-sync" })).not.toHaveAttribute("aria-expanded");
  });

  it("moves with arrows and j/k, Home and End", async () => {
    render(<Harness />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("{ArrowDown}j");
    expect(selected()).toHaveAccessibleName("label-print");
    await userEvent.keyboard("k");
    expect(selected()).toHaveAccessibleName("invoice-generate");
    await userEvent.keyboard("{End}");
    expect(selected()).toHaveAccessibleName("tracking-sync");
    await userEvent.keyboard("{Home}");
    expect(selected()).toHaveAccessibleName("queues");
  });

  it("expands, collapses and moves to the parent", async () => {
    const onToggle = vi.fn();
    render(<Harness onToggle={onToggle} />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("{End}{ArrowUp}{ArrowRight}");
    expect(onToggle).toHaveBeenCalledWith("shipment-events", true);
    await userEvent.keyboard("{Home}{ArrowLeft}");
    expect(onToggle).toHaveBeenCalledWith("queues", false);
    await userEvent.keyboard("{ArrowDown}{ArrowLeft}");
    expect(selected()).toHaveAccessibleName("queues");
  });

  it("activates with Enter and double-click", async () => {
    const onActivate = vi.fn();
    render(<Harness onActivate={onActivate} />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("{End}{Enter}");
    expect(onActivate).toHaveBeenCalledWith("tracking-sync");
    await userEvent.dblClick(screen.getByRole("treeitem", { name: "label-print" }));
    expect(onActivate).toHaveBeenCalledWith("label-print");
  });

  it("type-ahead jumps to the next matching label", async () => {
    render(<Harness />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("tr");
    expect(selected()).toHaveAccessibleName("tracking-sync");
  });
});
```

Run: `npm test -- src/ui/tree`
Expected: FAIL (module missing).

- [ ] **Step 2: Implement `app/src/ui/tree/Tree.tsx`**

```tsx
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRight } from "lucide-react";
import { useCallback, useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { cn } from "../lib/cn";

export interface TreeNode {
  id: string;
  parentId: string | null;
  depth: number;
  label: string;
  expandable: boolean;
  expanded: boolean;
}

export interface TreeRowState {
  selected: boolean;
}

export interface TreeProps<N extends TreeNode> {
  label: string;
  nodes: N[];
  selectedId: string | null;
  onSelect(id: string): void;
  onActivate(id: string): void;
  onToggle(id: string, expanded: boolean): void;
  renderRow(node: N, state: TreeRowState): ReactNode;
  rowHeight: number;
  className?: string;
  initialRect?: { width: number; height: number };
}

const INDENT = 14;
const TYPEAHEAD_MS = 500;

export function Tree<N extends TreeNode>(props: TreeProps<N>) {
  const { label, nodes, selectedId, onSelect, onActivate, onToggle, renderRow, rowHeight, className, initialRect } = props;
  const domId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const typeahead = useRef({ text: "", at: 0 });

  const virtualizer = useVirtualizer({
    count: nodes.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    initialRect,
  });

  useEffect(() => virtualizer.measure(), [rowHeight, virtualizer]);

  const index = nodes.findIndex((n) => n.id === selectedId);

  const select = useCallback(
    (i: number) => {
      const n = nodes[Math.max(0, Math.min(nodes.length - 1, i))];
      if (!n) return;
      onSelect(n.id);
      virtualizer.scrollToIndex(nodes.indexOf(n), { align: "auto" });
    },
    [nodes, onSelect, virtualizer],
  );

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const cur = nodes[index];
    switch (e.key) {
      case "ArrowDown":
      case "j":
        e.preventDefault();
        select(index < 0 ? 0 : index + 1);
        return;
      case "ArrowUp":
      case "k":
        e.preventDefault();
        select(index < 0 ? 0 : index - 1);
        return;
      case "Home":
        e.preventDefault();
        select(0);
        return;
      case "End":
        e.preventDefault();
        select(nodes.length - 1);
        return;
      case "ArrowRight":
        if (!cur) return;
        e.preventDefault();
        if (cur.expandable && !cur.expanded) onToggle(cur.id, true);
        else if (cur.expandable && cur.expanded && nodes[index + 1]?.parentId === cur.id) select(index + 1);
        return;
      case "ArrowLeft":
        if (!cur) return;
        e.preventDefault();
        if (cur.expandable && cur.expanded) onToggle(cur.id, false);
        else if (cur.parentId) select(nodes.findIndex((n) => n.id === cur.parentId));
        return;
      case "Enter":
        if (cur) {
          e.preventDefault();
          onActivate(cur.id);
        }
        return;
    }
    if (e.key.length === 1 && e.key !== " ") {
      const now = Date.now();
      const t = typeahead.current;
      t.text = now - t.at < TYPEAHEAD_MS ? t.text + e.key.toLowerCase() : e.key.toLowerCase();
      t.at = now;
      // Start at the current row when extending a prefix, after it for a new one.
      const start = t.text.length > 1 ? Math.max(index, 0) : index + 1;
      for (let k = 0; k < nodes.length; k++) {
        const i = (start + k) % nodes.length;
        if (nodes[i].label.toLowerCase().startsWith(t.text)) {
          select(i);
          break;
        }
      }
    }
  }

  const rowId = (id: string) => `${domId}-${id}`;

  return (
    <div
      ref={scrollRef}
      role="tree"
      aria-label={label}
      tabIndex={0}
      aria-activedescendant={selectedId && index >= 0 ? rowId(selectedId) : undefined}
      onKeyDown={onKeyDown}
      className={cn("relative h-full overflow-y-auto outline-none focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg", className)}
    >
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((v) => {
          const n = nodes[v.index];
          const selected = n.id === selectedId;
          return (
            <div
              key={n.id}
              id={rowId(n.id)}
              data-node-id={n.id}
              role="treeitem"
              aria-level={n.depth + 1}
              aria-expanded={n.expandable ? n.expanded : undefined}
              aria-selected={selected}
              aria-label={n.label}
              onClick={() => onSelect(n.id)}
              onDoubleClick={() => onActivate(n.id)}
              className={cn("absolute inset-x-0 flex items-center pr-2.5", selected ? "bg-selection" : "hover:bg-hover")}
              style={{ height: rowHeight, transform: `translateY(${v.start}px)`, paddingLeft: 6 + n.depth * INDENT }}
            >
              <span
                className="grid w-3.5 shrink-0 place-items-center text-faint"
                onClick={(e) => {
                  if (!n.expandable) return;
                  e.stopPropagation();
                  onToggle(n.id, !n.expanded);
                }}
              >
                {n.expandable && (
                  <ChevronRight aria-hidden="true" className={cn("size-3 transition-transform duration-100", n.expanded && "rotate-90")} />
                )}
              </span>
              <div className="flex min-w-0 flex-1 items-center gap-1.5 pl-1">{renderRow(n, { selected })}</div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
```

(`aria-label` on each row keeps accessible names stable when `renderRow` adds counts and badges.)

- [ ] **Step 3: Run tests**

Run: `npm test -- src/ui/tree && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 4: Catalogue section and finish review**

Add a "Tree" section to each catalogue column: a 260×360 `Tree` over a static namespace (two namespaces, queues group with four queues, topics group with three topics and five subscriptions, from mockup A's data), rendering `EntityGlyph`, the label, and `CountBadge` active/dead-letter counts (`attention` on dead-letter), selection on `tracking-sync`. Then run the Finish review procedure for: Tree rows (indent, chevrons, glyphs, counts, selection, hover, focus) in both themes and densities.

- [ ] **Step 5: Commit**

```bash
git add app/src
git commit -m "UI: virtualised keyboard tree (finish review: <verdict>)"
```

---

### Task 11: DataGrid — virtualised, keyboard-first, multi-select

Built and tested now; M1c's message grid is its first real user. The catalogue shows it with mockup A's dead-letter rows.

**Files:**
- Create: `app/src/ui/grid/DataGrid.tsx`
- Modify: `app/src/ui/index.ts` (`export * from "./grid/DataGrid";`), `app/src/dev/Catalogue.tsx` (a "Grid" section)
- Test: `app/src/ui/grid/DataGrid.test.tsx`

**Interfaces:**
- Consumes: Task 7 (`cn`), Task 8 (`SkeletonRows`), TanStack Table 8 (`ColumnDef`, `useReactTable`, `getCoreRowModel`, `flexRender`).
- Produces:
  - `interface GridColumnMeta { align?: "start" | "end"; mono?: boolean; width?: string }` (declared on TanStack's `ColumnMeta` via module augmentation; `width` is a CSS grid track such as `"64px"` or `"minmax(190px, 1fr)"`)
  - `DataGrid<R>(props: { label: string; columns: ColumnDef<R, any>[]; rows: R[]; getRowId(row: R): string; selectedIds: ReadonlySet<string>; focusId: string | null; onSelectionChange(ids: Set<string>, focusId: string | null): void; onActivate?(id: string): void; rowHeight: number; loading?: boolean; empty?: ReactNode; error?: ReactNode; initialRect?: { width: number; height: number } })`
  - Keys: ↓/`j`, ↑/`k`, Home/End move the focus row and select it alone; Shift+↓/↑ extends from the anchor; mod+A selects all; Enter activates. Mouse: click selects alone, mod-click toggles, shift-click selects the range from the anchor.

- [ ] **Step 1: Write the failing tests**

`app/src/ui/grid/DataGrid.test.tsx`:

```tsx
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ColumnDef } from "@tanstack/react-table";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { DataGrid } from "./DataGrid";

interface Msg { seq: number; subject: string; size: string }
const columns: ColumnDef<Msg, unknown>[] = [
  { accessorKey: "seq", header: "Seq", meta: { mono: true, width: "64px" } },
  { accessorKey: "subject", header: "Subject", meta: { width: "minmax(140px, 1fr)" } },
  { accessorKey: "size", header: "Size", meta: { align: "end", width: "60px" } },
];
const rows = (n: number): Msg[] => Array.from({ length: n }, (_, i) => ({ seq: 48000 + i, subject: `Subject ${i}`, size: "1.8 KB" }));

function Harness({ data = rows(20), onActivate = () => {}, ...rest }: { data?: Msg[]; onActivate?: (id: string) => void; loading?: boolean; empty?: React.ReactNode }) {
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);
  return (
    <DataGrid
      label="Dead-lettered messages"
      columns={columns}
      rows={data}
      getRowId={(r) => String(r.seq)}
      selectedIds={sel}
      focusId={focus}
      onSelectionChange={(ids, f) => { setSel(ids); setFocus(f); }}
      onActivate={onActivate}
      rowHeight={22}
      initialRect={{ width: 800, height: 300 }}
      {...rest}
    />
  );
}

const selectedRows = () => screen.getAllByRole("row").filter((r) => r.getAttribute("aria-selected") === "true");

describe("DataGrid", () => {
  it("renders headers and only visible rows of 10,000", () => {
    render(<Harness data={rows(10_000)} />);
    const grid = screen.getByRole("grid", { name: "Dead-lettered messages" });
    expect(within(grid).getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["Seq", "Subject", "Size"]);
    expect(within(grid).getAllByRole("row").length).toBeLessThan(80);
  });

  it("right-aligns end columns and uses mono where asked", () => {
    render(<Harness />);
    const cells = within(screen.getAllByRole("row")[1]).getAllByRole("gridcell");
    expect(cells[0].className).toContain("font-mono");
    expect(cells[2].className).toContain("text-right");
  });

  it("moves, extends and activates with the keyboard", async () => {
    const onActivate = vi.fn();
    render(<Harness onActivate={onActivate} />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("jj");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48001"]);
    await userEvent.keyboard("{Shift>}{ArrowDown}{ArrowDown}{/Shift}");
    expect(selectedRows()).toHaveLength(3);
    await userEvent.keyboard("{Enter}");
    expect(onActivate).toHaveBeenCalledWith("48003");
  });

  it("toggles with mod-click and ranges with shift-click", async () => {
    // One user instance so held modifier keys apply to the clicks.
    const user = userEvent.setup();
    render(<Harness />);
    const rowEls = () => screen.getAllByRole("row").slice(1);
    await user.click(rowEls()[1]);
    await user.keyboard("{Meta>}");
    await user.click(rowEls()[3]);
    await user.keyboard("{/Meta}");
    expect(selectedRows()).toHaveLength(2);
    await user.keyboard("{Shift>}");
    await user.click(rowEls()[6]);
    await user.keyboard("{/Shift}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48003", "48004", "48005", "48006"]);
  });

  it("shows loading and empty states instead of rows", () => {
    const { rerender } = render(<Harness data={[]} loading />);
    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();
    rerender(<Harness data={[]} empty={<p>No dead letters</p>} />);
    expect(screen.getByText("No dead letters")).toBeInTheDocument();
  });
});
```

(The mod-click test uses `Meta`; the grid treats `metaKey || ctrlKey` as "mod" so it passes on every OS.)

Run: `npm test -- src/ui/grid`
Expected: FAIL.

- [ ] **Step 2: Implement `app/src/ui/grid/DataGrid.tsx`**

```tsx
import { flexRender, getCoreRowModel, useReactTable, type ColumnDef, type RowData } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useId, useMemo, useRef, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { SkeletonRows } from "../components/skeleton";
import { cn } from "../lib/cn";

export interface GridColumnMeta {
  align?: "start" | "end";
  mono?: boolean;
  width?: string;
}

declare module "@tanstack/react-table" {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface ColumnMeta<TData extends RowData, TValue> extends GridColumnMeta {}
}

export interface DataGridProps<R> {
  label: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  columns: ColumnDef<R, any>[];
  rows: R[];
  getRowId(row: R): string;
  selectedIds: ReadonlySet<string>;
  focusId: string | null;
  onSelectionChange(ids: Set<string>, focusId: string | null): void;
  onActivate?(id: string): void;
  rowHeight: number;
  loading?: boolean;
  empty?: ReactNode;
  error?: ReactNode;
  initialRect?: { width: number; height: number };
}

export function DataGrid<R>(p: DataGridProps<R>) {
  const domId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const anchor = useRef<number | null>(null);

  const table = useReactTable({ data: p.rows, columns: p.columns, getCoreRowModel: getCoreRowModel(), getRowId: p.getRowId });
  const rows = table.getRowModel().rows;
  const template = useMemo(
    () => table.getVisibleLeafColumns().map((c) => c.columnDef.meta?.width ?? "minmax(80px, 1fr)").join(" "),
    [table, p.columns], // eslint-disable-line react-hooks/exhaustive-deps
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => p.rowHeight,
    overscan: 16,
    initialRect: p.initialRect,
  });

  const focusIndex = p.focusId ? rows.findIndex((r) => r.id === p.focusId) : -1;
  const idAt = (i: number) => rows[i]?.id;

  function selectOnly(i: number) {
    const id = idAt(i);
    if (id === undefined) return;
    anchor.current = i;
    p.onSelectionChange(new Set([id]), id);
    virtualizer.scrollToIndex(i, { align: "auto" });
  }

  function selectRange(to: number) {
    const from = anchor.current ?? to;
    const [a, b] = from <= to ? [from, to] : [to, from];
    const ids = new Set(rows.slice(a, b + 1).map((r) => r.id));
    p.onSelectionChange(ids, idAt(to) ?? null);
    virtualizer.scrollToIndex(to, { align: "auto" });
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (rows.length === 0) return;
    const mod = e.metaKey || e.ctrlKey;
    const last = rows.length - 1;
    const step = (d: number) => Math.max(0, Math.min(last, (focusIndex < 0 ? (d > 0 ? -1 : 1) : focusIndex) + d));
    if (mod && e.key.toLowerCase() === "a") {
      e.preventDefault();
      p.onSelectionChange(new Set(rows.map((r) => r.id)), p.focusId);
      return;
    }
    if (mod || e.altKey) return;
    let to: number | null = null;
    if (e.key === "ArrowDown" || e.key === "j") to = step(1);
    else if (e.key === "ArrowUp" || e.key === "k") to = step(-1);
    else if (e.key === "Home") to = 0;
    else if (e.key === "End") to = last;
    else if (e.key === "Enter" && p.focusId) {
      e.preventDefault();
      p.onActivate?.(p.focusId);
      return;
    }
    if (to === null) return;
    e.preventDefault();
    if (e.shiftKey) selectRange(to);
    else selectOnly(to);
  }

  function onRowClick(e: MouseEvent, i: number) {
    const id = idAt(i)!;
    if (e.shiftKey) return selectRange(i);
    if (e.metaKey || e.ctrlKey) {
      const next = new Set(p.selectedIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      anchor.current = i;
      return p.onSelectionChange(next, id);
    }
    selectOnly(i);
  }

  const header = table.getHeaderGroups()[0];
  const body = p.error ?? (p.loading ? <SkeletonRows rows={10} columns={3} /> : rows.length === 0 ? p.empty : null);

  return (
    <div
      ref={scrollRef}
      role="grid"
      aria-label={p.label}
      aria-multiselectable="true"
      aria-rowcount={rows.length + 1}
      tabIndex={0}
      aria-activedescendant={p.focusId && focusIndex >= 0 ? `${domId}-${p.focusId}` : undefined}
      onKeyDown={onKeyDown}
      className="relative h-full overflow-auto outline-none focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg"
    >
      <div role="row" aria-rowindex={1} className="sticky top-0 z-10 grid h-6 min-w-max items-center border-b border-line bg-content px-2.5 text-12 text-muted" style={{ gridTemplateColumns: template }}>
        {header.headers.map((h) => (
          <div key={h.id} role="columnheader" className={cn("truncate px-1", h.column.columnDef.meta?.align === "end" && "text-right")}>
            {flexRender(h.column.columnDef.header, h.getContext())}
          </div>
        ))}
      </div>
      {body ?? (
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }} className="min-w-max">
          {virtualizer.getVirtualItems().map((v) => {
            const row = rows[v.index];
            const selected = p.selectedIds.has(row.id);
            const focused = row.id === p.focusId;
            return (
              <div
                key={row.id}
                id={`${domId}-${row.id}`}
                role="row"
                data-row-id={row.id}
                aria-rowindex={v.index + 2}
                aria-selected={selected}
                onClick={(e) => onRowClick(e, v.index)}
                onDoubleClick={() => p.onActivate?.(row.id)}
                className={cn(
                  "absolute inset-x-0 grid items-center px-2.5 text-12",
                  selected ? "bg-selection" : "hover:bg-hover",
                  focused && "outline outline-1 -outline-offset-1 outline-fg/40",
                )}
                style={{ height: p.rowHeight, transform: `translateY(${v.start}px)`, gridTemplateColumns: template }}
              >
                {row.getVisibleCells().map((c) => {
                  const meta = c.column.columnDef.meta;
                  return (
                    <div
                      key={c.id}
                      role="gridcell"
                      className={cn("truncate px-1", meta?.align === "end" && "text-right", meta?.mono && "font-mono text-11 tabular-nums")}
                    >
                      {flexRender(c.column.columnDef.cell, c.getContext())}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
```

(If `outline-fg/40` doesn't compile because colours are CSS variables, use `outline-muted` instead.)

- [ ] **Step 3: Run tests**

Run: `npm test -- src/ui/grid && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 4: Catalogue section and finish review**

Add a "Grid" section to each catalogue column: a 360px-tall `DataGrid` with mockup A's columns (Seq mono 64px, Enqueued mono 118px, Subject 140px, Message ID mono 120px, Dead-letter reason `minmax(190px,1fr)`, Del. end 44px, Size end 60px) and the fourteen dead-letter rows from mockup A (`48213 … 48085`), plus a small loading grid and an empty grid ("No dead letters"). Run the Finish review procedure for: grid header, rows, selection, focus row, alignment, loading and empty states.

- [ ] **Step 5: Commit**

```bash
git add app/src
git commit -m "UI: virtualised keyboard data grid with multi-select (finish review: <verdict>)"
```

---
### Task 12: Engine — typed commands, events, queries and the mock transport

**Files:**
- Create: `app/src/engine/types.ts`, `app/src/engine/ipc.ts`, `app/src/engine/commands.ts`, `app/src/engine/events.ts`, `app/src/engine/queries.ts`, `app/src/engine/index.ts`
- Create: `app/src/engine/mock/fixtures.ts`, `app/src/engine/mock/transport.ts`
- Modify: `app/src/main.tsx` (install the mock transport in mock builds)
- Test: `app/src/engine/engine.test.ts`, `app/src/engine/queries.test.tsx`

**Interfaces:**
- Consumes: the IPC surface from Tasks 3–4.
- Produces:
  - types (mirroring Rust, snake_case fields): `Transport`, `ProfileAuth`, `Profile`, `DraftAuth`, `ProfileDraft`, `Check`, `ConnectionReport`, `AccountView`, `SignInPrompt`, `ConnectionView`, `EntityCounts`, `QueueInfo`, `TopicInfo`, `SubscriptionInfo`, `EntityRef`, `EntityListing`, `ActivityEntry`, `JobEvent`, `AppInfo`
  - `class EngineError extends Error { kind: string }`, `interface IpcTransport { invoke<T>(cmd, args?): Promise<T>; listen<T>(event, cb): Promise<() => void> }`, `setTransport(t)`, `call<T>(cmd, args?)`, `subscribe<T>(event, cb)`, `toEngineError(e)`
  - `commands`: `appInfo()`, `settingsGet(key)`, `settingsSet(key, value)`, `profilesList()`, `profileSave(draft)`, `profileDelete(id)`, `profileTest(id)`, `accountCurrent()`, `signIn(tenant, deviceCode)`, `signInCancel()`, `connect(profileId)`, `disconnect()`, `listEntities()`, `listSubscriptions(topic)`
  - `events`: `onActivity(cb)`, `onJob(cb)`, `onSignInPrompt(cb)` (each returns `Promise<() => void>`)
  - `queries`: `keys`, `createQueryClient()`, `useAppInfo()`, `useProfiles()`, `useAccount()`, `useEntities(profileId, refetchMs)`, `useSubscriptionsFor(profileId, topics: string[])`, `useSaveProfile()`, `useDeleteProfile()`, `useTestProfile()`, `useSignIn()`, `useSetting(key)`
  - mock: `createMockTransport(scenario: Scenario, delayMs = 0, seed: Record<string, unknown> = {}): IpcTransport & { calls: { cmd: string; args?: unknown }[] }`, `installMock()`, `type Scenario = "default" | "empty" | "forbidden" | "missing-secret" | "network" | "many"`, chosen by the `?scenario=` URL parameter; `?autoconnect=<profile id>` seeds `last_profile`.

- [ ] **Step 1: Write the failing tests**

`app/src/engine/engine.test.ts`:

```ts
import { beforeEach, describe, expect, it } from "vitest";
import * as commands from "./commands";
import { EngineError, setTransport, toEngineError } from "./ipc";
import { createMockTransport } from "./mock/transport";
import { onActivity } from "./events";

describe("ipc", () => {
  it("maps Rust CommandError objects to EngineError", () => {
    const e = toEngineError({ kind: "forbidden", message: "access denied (needs Listen): no" });
    expect(e).toBeInstanceOf(EngineError);
    expect(e.kind).toBe("forbidden");
    expect(toEngineError(new Error("boom")).kind).toBe("unknown");
    expect(toEngineError("text").message).toBe("text");
  });
});

describe("commands over the mock transport", () => {
  let mock: ReturnType<typeof createMockTransport>;
  beforeEach(() => {
    mock = createMockTransport("default");
    setTransport(mock);
  });

  it("pass camelCase top-level arguments", async () => {
    await commands.connect("p-logistics");
    await commands.signIn(null, true);
    expect(mock.calls).toContainEqual({ cmd: "connect", args: { profileId: "p-logistics" } });
    expect(mock.calls).toContainEqual({ cmd: "sign_in", args: { tenant: null, deviceCode: true } });
  });

  it("lists the fixture namespace after connecting and emits activity", async () => {
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    await commands.connect("p-logistics");
    const l = await commands.listEntities();
    expect(l.queues.map((q) => q.name)).toContain("invoice-generate");
    expect(log).toContain("Connected to Logistics");
  });

  it("rejects listing before connecting with not_connected", async () => {
    await expect(commands.listEntities()).rejects.toMatchObject({ kind: "not_connected" });
  });

  it("never returns a stored connection string", async () => {
    const p = await commands.profileSave({
      name: "New",
      auth: { kind: "connection-string", connection_string: "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=TOPSECRET" },
      production: false,
      transport: "amqp",
      pinned_entities: [],
    });
    expect(JSON.stringify(p)).not.toContain("TOPSECRET");
    expect(JSON.stringify(await commands.profilesList())).not.toContain("TOPSECRET");
  });

  it("serves scenarios", async () => {
    setTransport(createMockTransport("forbidden"));
    await commands.connect("p-logistics");
    const l = await commands.listEntities();
    expect(l.limited).toMatch(/can't list/);
    expect(l.known.length).toBeGreaterThan(0);
    setTransport(createMockTransport("empty"));
    expect(await commands.profilesList()).toEqual([]);
  });
});
```

`app/src/engine/queries.test.tsx`:

```tsx
import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { setTransport } from "./ipc";
import { createMockTransport } from "./mock/transport";
import { createQueryClient, useProfiles, useSaveProfile } from "./queries";

function wrapper() {
  const client = createQueryClient();
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("queries", () => {
  it("load profiles and refresh them after a save", async () => {
    setTransport(createMockTransport("default"));
    const w = wrapper();
    const { result } = renderHook(() => ({ profiles: useProfiles(), save: useSaveProfile() }), { wrapper: w });
    await waitFor(() => expect(result.current.profiles.data?.length).toBe(3));
    await result.current.save.mutateAsync({ name: "Fourth", auth: { kind: "entra", namespace: "fourth" }, production: false, transport: "amqp", pinned_entities: [] });
    await waitFor(() => expect(result.current.profiles.data?.length).toBe(4));
  });
});
```

Run: `npm test -- src/engine`
Expected: FAIL (modules missing).

- [ ] **Step 2: Implement types, ipc, commands and events**

`app/src/engine/types.ts`:

```ts
// Mirrors the Rust types that cross IPC (snake_case fields, as serde writes them).

export type Transport = "amqp" | "websockets";
export type ProfileAuth = { kind: "connection-string" } | { kind: "entra"; sign_in_id: string; tenant: string | null };

export interface Profile {
  id: string;
  name: string;
  host: string;
  auth: ProfileAuth;
  emulator: boolean;
  production: boolean;
  color: string | null;
  transport: Transport;
  proxy: string | null;
  pinned_entities: string[];
}

export type DraftAuth =
  | { kind: "connection-string"; connection_string?: string | null }
  | { kind: "entra"; namespace: string; tenant?: string | null };

export interface ProfileDraft {
  id?: string | null;
  name: string;
  auth: DraftAuth;
  production: boolean;
  color?: string | null;
  transport: Transport;
  proxy?: string | null;
  pinned_entities: string[];
}

export type Check =
  | { status: "ok"; detail: string }
  | { status: "failed"; kind: string; message: string }
  | { status: "skipped"; reason: string };

export interface ConnectionReport {
  host: string;
  emulator: boolean;
  transport: Transport;
  auth: string;
  management: Check;
  messaging: Check;
}

export interface AccountView { id: string; user_principal: string; display_name: string | null; tenant_id: string }
export interface SignInPrompt { kind: "browser" | "device-code"; url: string; user_code: string | null; message: string }

export interface ConnectionView {
  profile_id: string;
  profile_name: string;
  host: string;
  emulator: boolean;
  auth: string;
  transport: Transport;
  production: boolean;
}

export interface EntityCounts {
  active: number;
  dead_letter: number;
  scheduled: number;
  transfer: number;
  transfer_dead_letter: number;
  approximate: boolean;
  truncated: boolean;
}

export interface QueueInfo {
  name: string;
  status: string | null;
  counts: EntityCounts | null;
  size_in_bytes: number | null;
  requires_session: boolean;
  max_delivery_count: number | null;
  lock_duration: string | null;
  forward_to: string | null;
}

export interface TopicInfo { name: string; status: string | null; size_in_bytes: number | null; subscription_count: number | null }

export interface SubscriptionInfo {
  topic: string;
  name: string;
  status: string | null;
  counts: EntityCounts | null;
  requires_session: boolean;
  max_delivery_count: number | null;
  forward_to: string | null;
}

export interface EntityRef { entity: string; subscription: string | null }

export interface EntityListing { queues: QueueInfo[]; topics: TopicInfo[]; limited: string | null; known: EntityRef[] }

export interface ActivityError { kind: string; message: string }
export interface ActivityEntry { at: string; action: string; duration_ms: number; error: ActivityError | null }

export type JobEvent =
  | { type: "started"; id: string; title: string; total: number | null }
  | { type: "progress"; id: string; done: number; total: number | null }
  | { type: "finished"; id: string; error: ActivityError | null };

export interface AppInfo { version: string; startup_error: string | null }
```

`app/src/engine/ipc.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Unlisten = () => void;

export interface IpcTransport {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten>;
}

export class EngineError extends Error {
  readonly kind: string;
  constructor(kind: string, message: string) {
    super(message);
    this.name = "EngineError";
    this.kind = kind;
  }
}

const tauri: IpcTransport = {
  invoke: (cmd, args) => invoke(cmd, args),
  listen: (event, cb) => listen(event, (e) => cb(e.payload as never)),
};

let transport: IpcTransport = tauri;

/** Swap the transport (tests and mock builds). */
export function setTransport(t: IpcTransport): void {
  transport = t;
}

export function toEngineError(e: unknown): EngineError {
  if (e instanceof EngineError) return e;
  if (e && typeof e === "object" && "kind" in e && "message" in e) {
    return new EngineError(String((e as { kind: unknown }).kind), String((e as { message: unknown }).message));
  }
  return new EngineError("unknown", e instanceof Error ? e.message : String(e));
}

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await transport.invoke<T>(cmd, args);
  } catch (e) {
    throw toEngineError(e);
  }
}

export function subscribe<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  return transport.listen<T>(event, cb);
}
```

`app/src/engine/commands.ts`:

```ts
import { call } from "./ipc";
import type {
  AccountView, AppInfo, ConnectionReport, ConnectionView, EntityListing, Profile, ProfileDraft, SubscriptionInfo,
} from "./types";

export const appInfo = () => call<AppInfo>("app_info");
export const settingsGet = (key: string) => call<unknown>("settings_get", { key });
export const settingsSet = (key: string, value: unknown) => call<void>("settings_set", { key, value });

export const profilesList = () => call<Profile[]>("profiles_list");
export const profileSave = (draft: ProfileDraft) => call<Profile>("profile_save", { draft });
export const profileDelete = (id: string) => call<void>("profile_delete", { id });
export const profileTest = (id: string) => call<ConnectionReport>("profile_test", { id });

export const accountCurrent = () => call<AccountView | null>("account_current");
export const signIn = (tenant: string | null, deviceCode: boolean) => call<AccountView>("sign_in", { tenant, deviceCode });
export const signInCancel = () => call<void>("sign_in_cancel");

export const connect = (profileId: string) => call<ConnectionView>("connect", { profileId });
export const disconnect = () => call<void>("disconnect");

export const listEntities = () => call<EntityListing>("list_entities");
export const listSubscriptions = (topic: string) => call<SubscriptionInfo[]>("list_subscriptions", { topic });
```

`app/src/engine/events.ts`:

```ts
import { subscribe } from "./ipc";
import type { ActivityEntry, JobEvent, SignInPrompt } from "./types";

export const onActivity = (cb: (e: ActivityEntry) => void) => subscribe<ActivityEntry>("activity", cb);
export const onJob = (cb: (e: JobEvent) => void) => subscribe<JobEvent>("job", cb);
export const onSignInPrompt = (cb: (p: SignInPrompt) => void) => subscribe<SignInPrompt>("sign-in-prompt", cb);
```

- [ ] **Step 3: Implement queries**

`app/src/engine/queries.ts`:

```ts
import { QueryClient, useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import * as commands from "./commands";
import type { EngineError } from "./ipc";
import type { ProfileDraft } from "./types";

export const keys = {
  appInfo: ["app-info"] as const,
  profiles: ["profiles"] as const,
  account: ["account"] as const,
  entities: (profileId: string | null) => ["entities", profileId] as const,
  subscriptions: (profileId: string | null, topic: string) => ["subscriptions", profileId, topic] as const,
  setting: (key: string) => ["setting", key] as const,
};

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false,
        staleTime: 30_000,
        retry: (count, e) => count < 1 && (e as EngineError).kind === "network",
      },
    },
  });
}

export const useAppInfo = () => useQuery({ queryKey: keys.appInfo, queryFn: commands.appInfo, staleTime: Infinity });
export const useProfiles = () => useQuery({ queryKey: keys.profiles, queryFn: commands.profilesList });
export const useAccount = () => useQuery({ queryKey: keys.account, queryFn: commands.accountCurrent });

/** Queues and topics of the connected profile. `refetchMs` 0 disables auto-refresh. */
export function useEntities(profileId: string | null, refetchMs = 0) {
  return useQuery({
    queryKey: keys.entities(profileId),
    queryFn: commands.listEntities,
    enabled: profileId !== null,
    refetchInterval: refetchMs > 0 ? refetchMs : false,
    placeholderData: (prev) => prev,
  });
}

/** Subscriptions for each expanded topic, loaded lazily and independently. */
export function useSubscriptionsFor(profileId: string | null, topics: string[], refetchMs = 0) {
  return useQueries({
    queries: topics.map((topic) => ({
      queryKey: keys.subscriptions(profileId, topic),
      queryFn: () => commands.listSubscriptions(topic),
      enabled: profileId !== null,
      refetchInterval: refetchMs > 0 ? refetchMs : (false as const),
    })),
  });
}

export function useSetting(key: string) {
  return useQuery({ queryKey: keys.setting(key), queryFn: () => commands.settingsGet(key), staleTime: Infinity });
}

export function useSaveProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (draft: ProfileDraft) => commands.profileSave(draft),
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.profiles }),
  });
}

export function useDeleteProfile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => commands.profileDelete(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.profiles }),
  });
}

export const useTestProfile = () => useMutation({ mutationFn: (id: string) => commands.profileTest(id) });

export function useSignIn() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ tenant, deviceCode }: { tenant: string | null; deviceCode: boolean }) => commands.signIn(tenant, deviceCode),
    onSuccess: (account) => qc.setQueryData(keys.account, account),
  });
}
```

`app/src/engine/index.ts`:

```ts
export * from "./types";
export { EngineError, setTransport, type IpcTransport } from "./ipc";
export * as commands from "./commands";
export * from "./events";
export * from "./queries";
```

- [ ] **Step 4: Implement the mock transport and fixtures**

`app/src/engine/mock/fixtures.ts` (mockup A's namespace; plus one empty topic):

```ts
import type { EntityCounts, Profile, QueueInfo, SubscriptionInfo, TopicInfo } from "../types";

export const counts = (active: number, dead_letter: number, scheduled = 0): EntityCounts => ({
  active, dead_letter, scheduled, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false,
});

export const profiles: Profile[] = [
  { id: "p-logistics", name: "Logistics", host: "sb-logistics-prod.servicebus.windows.net", auth: { kind: "connection-string" }, emulator: false, production: true, color: null, transport: "amqp", proxy: null, pinned_entities: [] },
  { id: "p-logistics-dev", name: "Logistics dev", host: "sb-logistics-dev.servicebus.windows.net", auth: { kind: "entra", sign_in_id: "oid-1", tenant: null }, emulator: false, production: false, color: null, transport: "amqp", proxy: null, pinned_entities: [] },
  { id: "p-partner", name: "Partner gateway", host: "sb-partner-gateway.servicebus.windows.net", auth: { kind: "connection-string" }, emulator: false, production: false, color: null, transport: "websockets", proxy: null, pinned_entities: ["orders"] },
];

const q = (name: string, c: EntityCounts | null, extra: Partial<QueueInfo> = {}): QueueInfo => ({
  name, status: "Active", counts: c, size_in_bytes: 81_920, requires_session: false, max_delivery_count: 10, lock_duration: "PT30S", forward_to: null, ...extra,
});

export const queues: QueueInfo[] = [
  q("customs-docs", counts(112, 0)),
  q("invoice-generate", counts(1284, 37)),
  q("label-print", counts(6, 0)),
  q("returns-intake", counts(0, 211), { requires_session: true }),
];

export const topics: TopicInfo[] = [
  { name: "archive-events", status: "Active", size_in_bytes: 0, subscription_count: 0 },
  { name: "carrier-updates", status: "Active", size_in_bytes: 20_480, subscription_count: 2 },
  { name: "shipment-events", status: "Active", size_in_bytes: 4_194_304, subscription_count: 3 },
  { name: "stock-levels", status: "Active", size_in_bytes: 4_096, subscription_count: 1 },
];

const s = (topic: string, name: string, c: EntityCounts | null): SubscriptionInfo => ({
  topic, name, status: "Active", counts: c, requires_session: false, max_delivery_count: 10, forward_to: null,
});

export const subscriptions: Record<string, SubscriptionInfo[]> = {
  "archive-events": [],
  "carrier-updates": [s("carrier-updates", "audit-log", counts(0, 0)), s("carrier-updates", "eta-recalc", counts(58, 9))],
  "shipment-events": [s("shipment-events", "billing", counts(0, 0)), s("shipment-events", "tracking-sync", counts(2906, 3417)), s("shipment-events", "warehouse-notify", counts(14, 1))],
  "stock-levels": [s("stock-levels", "replenish", null)],
};

export const manyQueues: QueueInfo[] = Array.from({ length: 2000 }, (_, i) =>
  q(`queue-${String(i).padStart(4, "0")}`, counts((i * 37) % 900, i % 17 === 0 ? (i % 50) + 1 : 0)),
);
```

`app/src/engine/mock/transport.ts`:

```ts
import { EngineError, setTransport, type IpcTransport, type Unlisten } from "../ipc";
import type { AccountView, ActivityEntry, ConnectionView, EntityListing, Profile, ProfileDraft } from "../types";
import * as fx from "./fixtures";

export type Scenario = "default" | "empty" | "forbidden" | "missing-secret" | "network" | "many";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function createMockTransport(scenario: Scenario, delayMs = 0, seed: Record<string, unknown> = {}) {
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const calls: { cmd: string; args?: unknown }[] = [];
  const settings = new Map<string, unknown>(Object.entries(seed));
  let profiles: Profile[] = scenario === "empty" ? [] : structuredClone(fx.profiles);
  let active: Profile | null = null;
  let account: AccountView | null = null;

  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((cb) => cb(payload));
  const activity = (action: string, started: number, error: EngineError | null = null) =>
    emit("activity", {
      at: new Date().toISOString(),
      action,
      duration_ms: Math.max(1, Math.round(performance.now() - started)),
      error: error ? { kind: error.kind, message: error.message } : null,
    } satisfies ActivityEntry);

  async function handle(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
    const started = performance.now();
    if (delayMs) await wait(delayMs);
    switch (cmd) {
      case "app_info":
        return { version: "0.0.1-mock", startup_error: null };
      case "settings_get":
        return settings.get(args.key as string) ?? null;
      case "settings_set":
        if (args.value == null) settings.delete(args.key as string);
        else settings.set(args.key as string, structuredClone(args.value));
        return null;
      case "profiles_list":
        return structuredClone(profiles);
      case "profile_save": {
        const d = args.draft as ProfileDraft;
        const existing = d.id ? profiles.find((p) => p.id === d.id) : undefined;
        const p: Profile = {
          id: existing?.id ?? `p-${Math.random().toString(36).slice(2, 8)}`,
          name: d.name.trim(),
          host: d.auth.kind === "entra" ? (d.auth.namespace.includes(".") ? d.auth.namespace : `${d.auth.namespace}.servicebus.windows.net`) : existing?.host ?? "new.servicebus.windows.net",
          auth: d.auth.kind === "entra" ? { kind: "entra", sign_in_id: account?.id ?? "oid-1", tenant: d.auth.tenant ?? null } : { kind: "connection-string" },
          emulator: false,
          production: d.production,
          color: d.color ?? null,
          transport: d.transport,
          proxy: d.proxy ?? null,
          pinned_entities: d.pinned_entities,
        };
        if (profiles.some((o) => o.id !== p.id && o.name.toLowerCase() === p.name.toLowerCase())) {
          const e = new EngineError("invalid", `a profile named \`${p.name}\` already exists`);
          activity("Saving the profile failed", started, e);
          throw e;
        }
        profiles = existing ? profiles.map((o) => (o.id === p.id ? p : o)) : [...profiles, p];
        activity(`Saved profile ${p.name}`, started);
        return structuredClone(p);
      }
      case "profile_delete": {
        const p = profiles.find((o) => o.id === args.id);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        profiles = profiles.filter((o) => o.id !== args.id);
        if (active?.id === p.id) active = null;
        activity(`Removed profile ${p.name}`, started);
        return null;
      }
      case "profile_test": {
        const p = profiles.find((o) => o.id === args.id)!;
        activity(`Tested ${p.name}: management ok, messaging ok`, started);
        return {
          host: p.host, emulator: false, transport: p.transport, auth: "SAS key reader",
          management: { status: "ok", detail: "listed 4 queue(s) (Manage)" },
          messaging: { status: "ok", detail: "peeked customs-docs (Listen)" },
        };
      }
      case "account_current":
        return account;
      case "sign_in": {
        emit("sign-in-prompt", args.deviceCode
          ? { kind: "device-code", url: "https://microsoft.com/devicelogin", user_code: "QX4T-9KLM", message: "Open the page and enter the code." }
          : { kind: "browser", url: "https://login.microsoftonline.com/", user_code: null, message: "Finish signing in in your browser." });
        await wait(400);
        account = { id: "oid-1", user_principal: "ops@contoso.example", display_name: "Ops", tenant_id: "tenant-1" };
        activity(`Signed in as ${account.user_principal}`, started);
        return account;
      }
      case "sign_in_cancel":
        return null;
      case "connect": {
        const p = profiles.find((o) => o.id === args.profileId);
        if (!p) throw new EngineError("not_found", "That profile no longer exists.");
        if (scenario === "missing-secret") {
          const e = new EngineError("invalid", `profile \`${p.name}\` has no stored connection string; remove and add it again`);
          activity(`Connecting to ${p.name} failed`, started, e);
          throw e;
        }
        active = p;
        activity(`Connected to ${p.name}`, started);
        return {
          profile_id: p.id, profile_name: p.name, host: p.host, emulator: p.emulator,
          auth: p.auth.kind === "entra" ? "Entra ID ops@contoso.example" : "SAS key reader",
          transport: p.transport, production: p.production,
        } satisfies ConnectionView;
      }
      case "disconnect":
        active = null;
        activity("Disconnected", started);
        return null;
      case "list_entities": {
        if (!active) throw new EngineError("not_connected", "Choose a connection first.");
        if (scenario === "network") {
          const e = new EngineError("network", "network error: connection reset by peer");
          activity("Listing the namespace failed", started, e);
          throw e;
        }
        if (scenario === "forbidden") {
          activity("Listing not allowed; showing 2 pinned entities", started);
          return {
            queues: [], topics: [],
            limited: "This identity can't list the namespace (needs Manage,EntityRead). Showing pinned entities.",
            known: [{ entity: "orders", subscription: null }, { entity: "shipment-events", subscription: "tracking-sync" }],
          } satisfies EntityListing;
        }
        const queues = scenario === "many" ? fx.manyQueues : fx.queues;
        activity(`Listed ${queues.length} queues and ${fx.topics.length} topics`, started);
        return { queues, topics: fx.topics, limited: null, known: [] } satisfies EntityListing;
      }
      case "list_subscriptions": {
        if (!active) throw new EngineError("not_connected", "Choose a connection first.");
        const subs = fx.subscriptions[args.topic as string] ?? [];
        activity(`Listed ${subs.length} subscriptions of ${args.topic}`, started);
        return subs;
      }
      default:
        throw new EngineError("unknown", `mock: unknown command ${cmd}`);
    }
  }

  const transport: IpcTransport & { calls: typeof calls } = {
    calls,
    async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
      calls.push(args === undefined ? { cmd } : { cmd, args });
      return (await handle(cmd, args)) as T;
    },
    async listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
      const set = listeners.get(event) ?? new Set();
      set.add(cb as (p: unknown) => void);
      listeners.set(event, set);
      return () => set.delete(cb as (p: unknown) => void);
    },
  };
  return transport;
}

/** Mock builds: `?scenario=` picks the scenario, `?autoconnect=<profile id>`
 *  seeds the last-used profile so the app opens connected. */
export function installMock(): void {
  const params = new URLSearchParams(window.location.search);
  const scenario = (params.get("scenario") ?? "default") as Scenario;
  const autoconnect = params.get("autoconnect");
  setTransport(createMockTransport(scenario, 120, autoconnect ? { last_profile: autoconnect } : {}));
}
```

`app/src/main.tsx` (replace the render call):

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import "./ui/styles.css";
import { applyTheme, resolveTheme } from "./ui/theme";
import App from "./App";

applyTheme(resolveTheme("system", window.matchMedia("(prefers-color-scheme: dark)").matches), "compact");

const boot =
  import.meta.env.VITE_DISPATCHED_MOCK === "1"
    ? import("./engine/mock/transport").then((m) => m.installMock())
    : Promise.resolve();

void boot.then(() => {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
```

- [ ] **Step 5: Run tests**

Run: `npm test -- src/engine && npm run typecheck && npm run lint && npm run build && ! grep -rl "QX4T-9KLM" dist/assets`
Expected: tests PASS; the production build contains no mock code (the last command finds no file with the mock's device code).

- [ ] **Step 6: Commit**

```bash
git add app/src/engine app/src/main.tsx
git commit -m "Engine: typed commands, events, queries and a scenario mock transport"
```

---
### Task 13: Shell foundation — state, connection, layout, top bar, status bar, welcome

**Files:**
- Create: `app/src/shell/state.ts`, `app/src/shell/connection.tsx`, `app/src/shell/context.tsx`, `app/src/shell/queryState.ts`, `app/src/shell/Shell.tsx`, `app/src/shell/Workbench.tsx`, `app/src/shell/TopBar.tsx`, `app/src/shell/ConnectionSelector.tsx`, `app/src/shell/StatusBar.tsx`, `app/src/shell/Welcome.tsx`, `app/src/shell/sheets/SheetHost.tsx`
- Modify: `app/src/App.tsx` (providers + `<Shell/>`), `app/src/engine/ipc.ts` (add `inTauri()`), `app/src/engine/index.ts` (export it), `app/e2e/shots.spec.ts`
- Delete: `app/src/LegacyApp.tsx`, `app/src/api.ts`
- Test: `app/src/shell/state.test.ts`, `app/src/shell/Shell.test.tsx`, `app/src/shell/test-utils.tsx`

**Interfaces:**
- Consumes: `@/ui` (Tasks 7–9), `@/engine` (Task 12).
- Produces:
  - `state.ts`: `Prefs`, `AutoRefresh`, `defaultPrefs`, `parsePrefs(v)`, `SUB_TABS`, `SubTabId`, `EntityKind = "queue" | "topic" | "subscription"`, `OpenTab { id; kind; subTab }`, `Layout { treeSize; drawerOpen; drawerSize; drawerTab: "jobs" | "activity" }`, `ProfileShell { tabs; activeTab; expanded; selected; layout }`, `emptyShell`, `parseProfileShell(v)`, `ShellAction`, `shellReducer`, `useProfileShell(profileId)`, `usePrefs()`
  - `connection.tsx`: `ConnStatus = "idle" | "connecting" | "connected" | "failed"`, `useConnectionController()` returning `{ status, view, error, profileId, connect(id), disconnect(), retry() }`
  - `context.tsx`: `ShellProvider`, `useShell()` → `{ prefs, setPrefs, conn, shell, dispatch, sheet, openSheet(s), closeSheet(), paletteOpen, setPaletteOpen, refresh() }` with `SheetState = { kind: "profile"; profileId: string | null; auth?: "connection-string" | "entra" } | { kind: "profiles" } | { kind: "settings" } | { kind: "update" } | null`
  - `Shell` layout with three slots: `tree`, `workspace`, `drawer`
  - `Workbench()` — the only place that fills the slots (`<Shell tree={…} workspace={…} drawer={…} />`); Tasks 14, 15 and 17 add their component there. `App` and `renderShell` render `<Workbench/>`.
  - test helper `renderShell(scenario, seed?)` in `shell/test-utils.tsx`
  - engine: `inTauri(): boolean`

- [ ] **Step 1: Write the failing tests**

`app/src/shell/state.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { emptyShell, parsePrefs, parseProfileShell, shellReducer, type ProfileShell } from "./state";

const open = (s: ProfileShell, id: string, kind: "queue" | "subscription" = "queue") => shellReducer(s, { type: "open", tab: { id, kind } });

describe("shell reducer", () => {
  it("opens new tabs after the active one and focuses existing ones", () => {
    let s = open(emptyShell, "orders");
    s = open(s, "shipment-events/subscriptions/tracking-sync", "subscription");
    expect(s.tabs.map((t) => t.id)).toEqual(["orders", "shipment-events/subscriptions/tracking-sync"]);
    expect(s.activeTab).toBe("shipment-events/subscriptions/tracking-sync");
    s = open(s, "orders");
    expect(s.tabs).toHaveLength(2);
    expect(s.activeTab).toBe("orders");
    expect(s.tabs[0].subTab).toBe("overview");
  });

  it("closing the active tab activates its right neighbour, else the left", () => {
    let s = ["a", "b", "c"].reduce((acc, id) => open(acc, id), emptyShell);
    s = shellReducer(s, { type: "activate", id: "b" });
    s = shellReducer(s, { type: "close", id: "b" });
    expect(s.activeTab).toBe("c");
    s = shellReducer(s, { type: "close", id: "c" });
    expect(s.activeTab).toBe("a");
    s = shellReducer(s, { type: "close", id: "a" });
    expect(s.activeTab).toBeNull();
  });

  it("switches by index and remembers the sub-tab per tab", () => {
    let s = ["a", "b"].reduce((acc, id) => open(acc, id), emptyShell);
    s = shellReducer(s, { type: "subTab", id: "a", subTab: "dead-letter" });
    s = shellReducer(s, { type: "activateIndex", index: 0 });
    expect(s.activeTab).toBe("a");
    expect(s.tabs[0].subTab).toBe("dead-letter");
    expect(shellReducer(s, { type: "activateIndex", index: 7 })).toBe(s);
  });

  it("tracks expanded nodes and the tree selection", () => {
    let s = shellReducer(emptyShell, { type: "toggleExpanded", nodeId: "t:shipment-events", expanded: true });
    expect(s.expanded).toContain("t:shipment-events");
    s = shellReducer(s, { type: "toggleExpanded", nodeId: "t:shipment-events", expanded: false });
    expect(s.expanded).not.toContain("t:shipment-events");
    expect(shellReducer(s, { type: "select", nodeId: "q:orders" }).selected).toBe("q:orders");
  });
});

describe("persisted state", () => {
  it("bad_persisted_state_falls_back_to_defaults", () => {
    expect(parseProfileShell(null)).toEqual(emptyShell);
    expect(parseProfileShell("garbage")).toEqual(emptyShell);
    const s = parseProfileShell({
      tabs: [{ id: "orders", kind: "queue", subTab: "dead-letter" }, { id: 3 }, { id: "x", kind: "nope", subTab: "overview" }, { id: "y", kind: "queue", subTab: "bogus" }],
      activeTab: "missing",
      expanded: ["t:a", 4],
      selected: 9,
      layout: { treeSize: 900, drawerOpen: "yes", drawerSize: 25, drawerTab: "jobs" },
    });
    expect(s.tabs).toEqual([
      { id: "orders", kind: "queue", subTab: "dead-letter" },
      { id: "y", kind: "queue", subTab: "overview" },
    ]);
    expect(s.activeTab).toBe("orders");
    expect(s.expanded).toEqual(["t:a"]);
    expect(s.selected).toBeNull();
    expect(s.layout).toEqual({ treeSize: emptyShell.layout.treeSize, drawerOpen: true, drawerSize: 25, drawerTab: "jobs" });
  });

  it("prefs fall back per field", () => {
    expect(parsePrefs({ theme: "light", density: "huge", autoRefresh: 30 })).toEqual({ theme: "light", density: "compact", autoRefresh: 30 });
    expect(parsePrefs(undefined)).toEqual({ theme: "system", density: "compact", autoRefresh: 0 });
  });
});
```

`app/src/shell/test-utils.tsx`:

```tsx
import { QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { createQueryClient, setTransport } from "@/engine";
import { createMockTransport, type Scenario } from "@/engine/mock/transport";
import { TooltipProvider } from "@/ui";
import { ShellProvider } from "./context";
import { Workbench } from "./Workbench";

export function renderShell(scenario: Scenario = "default", seed: Record<string, unknown> = {}) {
  const mock = createMockTransport(scenario, 0, seed);
  setTransport(mock);
  const client = createQueryClient();
  const utils = render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ShellProvider>
          <Workbench />
        </ShellProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return { ...utils, mock, client };
}
```

`app/src/shell/Shell.test.tsx`:

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "./test-utils";

describe("Shell", () => {
  it("welcomes with saved connections and connects on click", async () => {
    renderShell("default");
    const list = await screen.findByRole("list", { name: "Saved connections" });
    await userEvent.click(within(list).getByRole("button", { name: /Logistics\b.*sb-logistics-prod/ }));
    const bar = screen.getByRole("banner");
    await waitFor(() => expect(within(bar).getByRole("button", { name: /Connection: Logistics/ })).toBeInTheDocument());
    expect(within(bar).getByText("PROD")).toBeInTheDocument();
    expect(within(screen.getByRole("contentinfo")).getByText("Connected")).toBeInTheDocument();
  });

  it("reconnects to the last profile on start", async () => {
    renderShell("default", { last_profile: "p-logistics-dev" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Connection: Logistics dev/ })).toBeInTheDocument());
  });

  it("asks for a first connection when there are none", async () => {
    renderShell("empty");
    expect(await screen.findByRole("heading", { name: "Add a connection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add connection string" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in with Entra ID" })).toBeInTheDocument();
  });

  it("missing secret shows the fix", async () => {
    renderShell("missing-secret", { last_profile: "p-logistics" });
    expect(await screen.findByRole("heading", { name: "Couldn't connect to Logistics" })).toBeInTheDocument();
    expect(screen.getByText(/no stored connection string/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit connection" })).toBeInTheDocument();
  });
});
```

Run: `npm test -- src/shell`
Expected: FAIL (modules missing).

- [ ] **Step 2: Implement `state.ts`**

```ts
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useReducer, useState, type Dispatch } from "react";
import { commands, keys, useSetting } from "@/engine";
import type { Density, ThemePref } from "@/ui";

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const oneOf = <T,>(v: unknown, options: readonly T[], fallback: T): T => (options.includes(v as T) ? (v as T) : fallback);
const num = (v: unknown, min: number, max: number, fallback: number) =>
  typeof v === "number" && Number.isFinite(v) && v >= min && v <= max ? v : fallback;

// ---- prefs (global, key "ui") ----

export type AutoRefresh = 0 | 30 | 60;
export interface Prefs { theme: ThemePref; density: Density; autoRefresh: AutoRefresh }
export const defaultPrefs: Prefs = { theme: "system", density: "compact", autoRefresh: 0 };

export function parsePrefs(v: unknown): Prefs {
  const o = isObj(v) ? v : {};
  return {
    theme: oneOf(o.theme, ["system", "dark", "light"] as const, defaultPrefs.theme),
    density: oneOf(o.density, ["compact", "comfortable"] as const, defaultPrefs.density),
    autoRefresh: oneOf(o.autoRefresh, [0, 30, 60] as const, defaultPrefs.autoRefresh),
  };
}

export function usePrefs(): [Prefs, (patch: Partial<Prefs>) => void] {
  const q = useSetting("ui");
  const qc = useQueryClient();
  const prefs = parsePrefs(q.data);
  const update = useCallback(
    (patch: Partial<Prefs>) => {
      const next = { ...prefs, ...patch };
      qc.setQueryData(keys.setting("ui"), next);
      void commands.settingsSet("ui", next).catch(() => {});
    },
    [prefs, qc],
  );
  return [prefs, update];
}

// ---- per-profile shell state (key "shell:<profile id>") ----

export const SUB_TABS = ["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "rules", "listener"] as const;
export type SubTabId = (typeof SUB_TABS)[number];
export type EntityKind = "queue" | "topic" | "subscription";

export interface OpenTab { id: string; kind: EntityKind; subTab: SubTabId }
export interface Layout { treeSize: number; drawerOpen: boolean; drawerSize: number; drawerTab: "jobs" | "activity" }
export interface ProfileShell { tabs: OpenTab[]; activeTab: string | null; expanded: string[]; selected: string | null; layout: Layout }

export const MAX_TABS = 30;
export const emptyShell: ProfileShell = {
  tabs: [],
  activeTab: null,
  expanded: ["group:queues", "group:topics"],
  selected: null,
  layout: { treeSize: 20, drawerOpen: true, drawerSize: 18, drawerTab: "activity" },
};

export function parseProfileShell(v: unknown): ProfileShell {
  if (!isObj(v)) return emptyShell;
  const tabs: OpenTab[] = (Array.isArray(v.tabs) ? v.tabs : [])
    .filter(isObj)
    .filter((t) => typeof t.id === "string" && t.id.length > 0 && ["queue", "topic", "subscription"].includes(t.kind as string))
    .slice(0, MAX_TABS)
    .map((t) => ({ id: t.id as string, kind: t.kind as EntityKind, subTab: oneOf(t.subTab, SUB_TABS, "overview") }));
  const activeTab = tabs.some((t) => t.id === v.activeTab) ? (v.activeTab as string) : (tabs[0]?.id ?? null);
  const expanded = Array.isArray(v.expanded) ? v.expanded.filter((e): e is string => typeof e === "string") : emptyShell.expanded;
  const l = isObj(v.layout) ? v.layout : {};
  const d = emptyShell.layout;
  return {
    tabs,
    activeTab,
    expanded,
    selected: typeof v.selected === "string" ? v.selected : null,
    layout: {
      treeSize: num(l.treeSize, 10, 50, d.treeSize),
      drawerOpen: typeof l.drawerOpen === "boolean" ? l.drawerOpen : d.drawerOpen,
      drawerSize: num(l.drawerSize, 8, 70, d.drawerSize),
      drawerTab: oneOf(l.drawerTab, ["jobs", "activity"] as const, d.drawerTab),
    },
  };
}

export type ShellAction =
  | { type: "open"; tab: { id: string; kind: EntityKind; subTab?: SubTabId } }
  | { type: "close"; id: string }
  | { type: "activate"; id: string }
  | { type: "activateIndex"; index: number }
  | { type: "subTab"; id: string; subTab: SubTabId }
  | { type: "toggleExpanded"; nodeId: string; expanded: boolean }
  | { type: "select"; nodeId: string | null }
  | { type: "layout"; patch: Partial<Layout> }
  | { type: "reset"; state: ProfileShell };

export function shellReducer(s: ProfileShell, a: ShellAction): ProfileShell {
  switch (a.type) {
    case "open": {
      if (s.tabs.some((t) => t.id === a.tab.id)) return { ...s, activeTab: a.tab.id };
      const at = s.activeTab ? s.tabs.findIndex((t) => t.id === s.activeTab) + 1 : s.tabs.length;
      const tab: OpenTab = { id: a.tab.id, kind: a.tab.kind, subTab: a.tab.subTab ?? "overview" };
      const tabs = [...s.tabs.slice(0, at), tab, ...s.tabs.slice(at)].slice(-MAX_TABS);
      return { ...s, tabs, activeTab: tab.id };
    }
    case "close": {
      const i = s.tabs.findIndex((t) => t.id === a.id);
      if (i < 0) return s;
      const tabs = s.tabs.filter((t) => t.id !== a.id);
      const activeTab = s.activeTab !== a.id ? s.activeTab : (tabs[i]?.id ?? tabs[i - 1]?.id ?? null);
      return { ...s, tabs, activeTab };
    }
    case "activate":
      return s.tabs.some((t) => t.id === a.id) ? { ...s, activeTab: a.id } : s;
    case "activateIndex": {
      const t = s.tabs[a.index];
      return t ? { ...s, activeTab: t.id } : s;
    }
    case "subTab":
      return { ...s, tabs: s.tabs.map((t) => (t.id === a.id ? { ...t, subTab: a.subTab } : t)) };
    case "toggleExpanded": {
      const rest = s.expanded.filter((e) => e !== a.nodeId);
      return { ...s, expanded: a.expanded ? [...rest, a.nodeId] : rest };
    }
    case "select":
      return { ...s, selected: a.nodeId };
    case "layout":
      return { ...s, layout: { ...s.layout, ...a.patch } };
    case "reset":
      return a.state;
  }
}

/** Shell state for a profile, loaded from and saved (debounced) to settings. */
export function useProfileShell(profileId: string | null): [ProfileShell, Dispatch<ShellAction>, boolean] {
  const [state, dispatch] = useReducer(shellReducer, emptyShell);
  const [loadedFor, setLoadedFor] = useState<string | null>(null);

  useEffect(() => {
    if (!profileId) return;
    let live = true;
    commands
      .settingsGet(`shell:${profileId}`)
      .then((v) => live && (dispatch({ type: "reset", state: parseProfileShell(v) }), setLoadedFor(profileId)))
      .catch(() => live && (dispatch({ type: "reset", state: emptyShell }), setLoadedFor(profileId)));
    return () => {
      live = false;
    };
  }, [profileId]);

  useEffect(() => {
    if (!profileId || loadedFor !== profileId) return;
    const t = setTimeout(() => void commands.settingsSet(`shell:${profileId}`, state).catch(() => {}), 300);
    return () => clearTimeout(t);
  }, [state, profileId, loadedFor]);

  return [state, dispatch, loadedFor === profileId];
}
```

- [ ] **Step 3: Implement connection, context and `inTauri`**

Add to `app/src/engine/ipc.ts` and export from `engine/index.ts`:

```ts
/** True inside the desktop app (false in the browser mock and tests). */
export const inTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
```

`app/src/shell/connection.tsx`:

```tsx
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type ConnectionView, type EngineError } from "@/engine";

export type ConnStatus = "idle" | "connecting" | "connected" | "failed";

interface ConnState {
  status: ConnStatus;
  view: ConnectionView | null;
  error: EngineError | null;
  profileId: string | null;
}

const idle: ConnState = { status: "idle", view: null, error: null, profileId: null };

export function useConnectionController() {
  const qc = useQueryClient();
  const [s, setS] = useState<ConnState>(idle);
  const autoTried = useRef(false);

  const connect = useCallback(
    async (profileId: string) => {
      setS({ status: "connecting", view: null, error: null, profileId });
      try {
        const view = await commands.connect(profileId);
        qc.removeQueries({ queryKey: ["entities"] });
        qc.removeQueries({ queryKey: ["subscriptions"] });
        setS({ status: "connected", view, error: null, profileId });
        void commands.settingsSet("last_profile", profileId).catch(() => {});
      } catch (e) {
        setS({ status: "failed", view: null, error: e as EngineError, profileId });
      }
    },
    [qc],
  );

  const disconnect = useCallback(async () => {
    await commands.disconnect().catch(() => {});
    qc.removeQueries({ queryKey: ["entities"] });
    setS(idle);
    void commands.settingsSet("last_profile", null).catch(() => {});
  }, [qc]);

  const retry = useCallback(() => {
    if (s.profileId) void connect(s.profileId);
  }, [connect, s.profileId]);

  // Reopen the last connection once on start (StrictMode runs effects twice).
  useEffect(() => {
    if (autoTried.current) return;
    autoTried.current = true;
    void Promise.all([commands.settingsGet("last_profile"), commands.profilesList()])
      .then(([last, profiles]) => {
        if (typeof last === "string" && profiles.some((p) => p.id === last)) void connect(last);
      })
      .catch(() => {});
  }, [connect]);

  return { ...s, connect, disconnect, retry };
}

export type ConnectionController = ReturnType<typeof useConnectionController>;
```

`app/src/shell/context.tsx`:

```tsx
import { useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useState, type Dispatch, type ReactNode } from "react";
import { useTheme } from "@/ui";
import { useConnectionController, type ConnectionController } from "./connection";
import { usePrefs, useProfileShell, type Prefs, type ProfileShell, type ShellAction } from "./state";

export type SheetState =
  | { kind: "profile"; profileId: string | null; auth?: "connection-string" | "entra" }
  | { kind: "profiles" }
  | { kind: "settings" }
  | { kind: "update" }
  | null;

interface ShellContextValue {
  prefs: Prefs;
  setPrefs: (patch: Partial<Prefs>) => void;
  conn: ConnectionController;
  shell: ProfileShell;
  dispatch: Dispatch<ShellAction>;
  shellLoaded: boolean;
  sheet: SheetState;
  openSheet: (s: Exclude<SheetState, null>) => void;
  closeSheet: () => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  refresh: () => void;
}

const ShellContext = createContext<ShellContextValue | null>(null);

export function ShellProvider({ children }: { children: ReactNode }) {
  const [prefs, setPrefs] = usePrefs();
  useTheme(prefs.theme, prefs.density);
  const conn = useConnectionController();
  const [shell, dispatch, shellLoaded] = useProfileShell(conn.status === "connected" ? conn.profileId : null);
  const [sheet, setSheet] = useState<SheetState>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const qc = useQueryClient();
  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ["entities"] });
    void qc.invalidateQueries({ queryKey: ["subscriptions"] });
  }, [qc]);

  return (
    <ShellContext.Provider
      value={{
        prefs, setPrefs, conn, shell, dispatch, shellLoaded, sheet,
        openSheet: setSheet, closeSheet: () => setSheet(null),
        paletteOpen, setPaletteOpen, refresh,
      }}
    >
      {children}
    </ShellContext.Provider>
  );
}

export function useShell(): ShellContextValue {
  const v = useContext(ShellContext);
  if (!v) throw new Error("useShell outside ShellProvider");
  return v;
}
```

- [ ] **Step 4: Implement the layout and its parts**

`app/src/shell/Shell.tsx`:

```tsx
import type { ReactNode } from "react";
import { Panel, PanelGroup, ResizeHandle } from "@/ui";
import { useShell } from "./context";
import { SheetHost } from "./sheets/SheetHost";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { Welcome } from "./Welcome";

/** The window: top bar, then tree | (workspace / drawer), then status bar. */
export function Shell({ tree, workspace, drawer }: { tree?: ReactNode; workspace?: ReactNode; drawer?: ReactNode }) {
  const { conn, shell, shellLoaded, dispatch } = useShell();
  const ready = conn.status === "connected" && shellLoaded;
  return (
    <div className="flex h-full flex-col bg-content text-fg">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        {ready ? (
          <PanelGroup direction="horizontal" onLayout={([treeSize]) => dispatch({ type: "layout", patch: { treeSize } })}>
            <Panel defaultSize={shell.layout.treeSize} minSize={12} maxSize={45} className="bg-side">
              {tree}
            </Panel>
            <ResizeHandle />
            <Panel minSize={40}>
              <PanelGroup direction="vertical" onLayout={(sizes) => sizes.length === 2 && dispatch({ type: "layout", patch: { drawerSize: sizes[1] } })}>
                <Panel minSize={30}>{workspace}</Panel>
                {shell.layout.drawerOpen && drawer && (
                  <>
                    <ResizeHandle />
                    <Panel defaultSize={shell.layout.drawerSize} minSize={8} maxSize={70} className="bg-side">
                      {drawer}
                    </Panel>
                  </>
                )}
              </PanelGroup>
            </Panel>
          </PanelGroup>
        ) : (
          <Welcome />
        )}
      </div>
      <StatusBar />
      <SheetHost />
    </div>
  );
}
```

`app/src/shell/Workbench.tsx`:

```tsx
import { Shell } from "./Shell";

/** Fills the shell's slots. Tasks 14, 15 and 17 add the tree, workspace and drawer here. */
export function Workbench() {
  return <Shell />;
}
```

`app/src/shell/TopBar.tsx`:

```tsx
import { RefreshCw, Search } from "lucide-react";
import { inTauri } from "@/engine";
import { Hint, IconButton, Kbd, isMac } from "@/ui";
import { ConnectionSelector } from "./ConnectionSelector";
import { useShell } from "./context";

export function TopBar() {
  const { conn, setPaletteOpen, refresh } = useShell();
  const macInset = isMac && inTauri();
  return (
    <header
      data-tauri-drag-region
      className="flex h-[38px] shrink-0 items-center gap-3 border-b border-line bg-chrome pr-2"
      style={{ paddingLeft: macInset ? 84 : 10 }}
    >
      <ConnectionSelector />
      {conn.view && <span className="truncate text-12 text-faint">{conn.view.auth}</span>}
      <div data-tauri-drag-region className="h-full min-w-4 flex-1" />
      <button
        type="button"
        onClick={() => setPaletteOpen(true)}
        className="flex h-6 w-[380px] max-w-[40vw] items-center gap-2 rounded-ctl border border-line bg-field px-2 text-12 text-faint hover:text-muted"
      >
        <Search aria-hidden="true" className="size-3.5 text-icon" />
        <span className="flex-1 text-left">Go to entity or command</span>
        <Kbd shortcut="mod+k" />
      </button>
      <div data-tauri-drag-region className="h-full min-w-4 flex-1" />
      <Hint label="Refresh" shortcut="f5">
        <IconButton label="Refresh" onClick={refresh} disabled={conn.status !== "connected"}>
          <RefreshCw />
        </IconButton>
      </Hint>
    </header>
  );
}
```

`app/src/shell/ConnectionSelector.tsx`:

```tsx
import { Check, ChevronDown } from "lucide-react";
import { useProfiles } from "@/engine";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger, ProdBadge,
} from "@/ui";
import { useShell } from "./context";

export const shortHost = (host: string) => host.replace(/\.servicebus\.(windows\.net|chinacloudapi\.cn|usgovcloudapi\.net)$/, "");

export function ConnectionSelector() {
  const { conn, openSheet } = useShell();
  const profiles = useProfiles().data ?? [];
  const current = profiles.find((p) => p.id === conn.profileId);
  const label =
    conn.status === "connecting" && current ? `Connecting to ${current.name}…`
    : conn.status === "connected" && current ? current.name
    : "Choose a connection";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={`Connection: ${label}`}
          className="flex h-[26px] max-w-[420px] items-center gap-2 rounded-ctl border border-line bg-field px-2 text-12 hover:bg-hover"
        >
          <span className="truncate font-semibold">{label}</span>
          {conn.status === "connected" && current && <span className="truncate text-muted">{shortHost(current.host)}</span>}
          {conn.status === "connected" && current?.production && <ProdBadge />}
          <ChevronDown aria-hidden="true" className="size-3 shrink-0 text-icon" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="min-w-[300px]">
        <DropdownMenuLabel>Connections</DropdownMenuLabel>
        {profiles.map((p) => (
          <DropdownMenuItem key={p.id} onSelect={() => void conn.connect(p.id)}>
            <span className="w-3.5">{p.id === conn.profileId && conn.status === "connected" && <Check />}</span>
            <span className="truncate">{p.name}</span>
            <span className="truncate text-faint">{shortHost(p.host)}</span>
            {p.production && <ProdBadge className="ml-auto" />}
          </DropdownMenuItem>
        ))}
        {profiles.length > 0 && <DropdownMenuSeparator />}
        <DropdownMenuItem onSelect={() => openSheet({ kind: "profile", profileId: null })}>Add connection…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openSheet({ kind: "profiles" })}>Manage connections…</DropdownMenuItem>
        {conn.status === "connected" && <DropdownMenuItem onSelect={() => void conn.disconnect()}>Disconnect</DropdownMenuItem>}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
```

`app/src/shell/StatusBar.tsx`:

```tsx
import { CircleAlert } from "lucide-react";
import { keys, useAppInfo } from "@/engine";
import { Button, formatClock } from "@/ui";
import { useShell } from "./context";
import { useIsFetching, useQueryState } from "./queryState";

const LOST = new Set(["network", "timeout", "auth", "not_signed_in"]);

export function StatusBar() {
  const { conn } = useShell();
  const info = useAppInfo().data;
  const entities = useQueryState(keys.entities(conn.status === "connected" ? conn.profileId : null));
  const fetching = useIsFetching({ queryKey: ["entities"] }) > 0;
  const lost = conn.status === "connected" && entities.error != null && LOST.has(entities.error.kind);
  const state =
    conn.status === "connecting" ? "Connecting…"
    : lost ? "Connection lost"
    : conn.status === "connected" ? "Connected"
    : conn.status === "failed" ? "Not connected"
    : "Not connected";
  return (
    <footer className="flex h-[22px] shrink-0 items-center gap-4 border-t border-line bg-chrome px-3 text-11 text-muted">
      {conn.view && <span>{conn.view.transport === "websockets" ? "AMQP over WebSockets" : "AMQP"}</span>}
      <span className="flex items-center gap-1.5">
        {lost && <CircleAlert aria-hidden="true" className="size-3 text-accent" />}
        <span className={lost ? "text-fg" : undefined}>{state}</span>
        {lost && <Button size="sm" variant="quiet" onClick={conn.retry}>Reconnect</Button>}
      </span>
      {info?.startup_error && (
        <span className="flex items-center gap-1.5 text-fg">
          <CircleAlert aria-hidden="true" className="size-3 text-accent" />
          {info.startup_error}
        </span>
      )}
      <div className="flex-1" />
      {conn.status === "connected" && entities.dataUpdatedAt > 0 && (
        <span className="tabular-nums">{fetching ? "Refreshing…" : `Refreshed ${formatClock(new Date(entities.dataUpdatedAt).toISOString())}`}</span>
      )}
    </footer>
  );
}
```

`app/src/shell/queryState.ts` (lets components read a query's cached state without starting the query):

```ts
import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { useSyncExternalStore } from "react";
import type { EngineError } from "@/engine";

export { useIsFetching, useQueryClient };

/** Observe a query's cached state (data time, error) without fetching it. */
export function useQueryState(queryKey: readonly unknown[]): { dataUpdatedAt: number; error: EngineError | null } {
  const qc = useQueryClient();
  const cache = qc.getQueryCache();
  const read = () => {
    const q = cache.find({ queryKey, exact: true });
    return `${q?.state.dataUpdatedAt ?? 0}|${q?.state.error ? (q.state.error as EngineError).kind : ""}`;
  };
  const snap = useSyncExternalStore((cb) => cache.subscribe(cb), read);
  const q = cache.find({ queryKey, exact: true });
  void snap;
  return { dataUpdatedAt: q?.state.dataUpdatedAt ?? 0, error: (q?.state.error as EngineError | null) ?? null };
}
```

`app/src/shell/Welcome.tsx`:

```tsx
import { useProfiles } from "@/engine";
import { Button, EmptyState, ProdBadge, SkeletonRows } from "@/ui";
import { shortHost } from "./ConnectionSelector";
import { useShell } from "./context";

export function Welcome() {
  const { conn, openSheet } = useShell();
  const profiles = useProfiles();
  const failed = conn.status === "failed" ? profiles.data?.find((p) => p.id === conn.profileId) : undefined;

  if (profiles.isPending) return <div className="w-[360px] p-6"><SkeletonRows rows={4} /></div>;

  return (
    <div className="flex h-full w-full justify-center overflow-y-auto">
      <div className="flex w-[480px] max-w-full flex-col gap-4 py-16">
        {failed && (
          <EmptyState
            tone="error"
            title={`Couldn't connect to ${failed.name}`}
            body={conn.error?.message}
            action={
              <div className="flex gap-2">
                <Button onClick={conn.retry}>Try again</Button>
                <Button variant="quiet" onClick={() => openSheet({ kind: "profile", profileId: failed.id })}>Edit connection</Button>
              </div>
            }
          />
        )}
        {profiles.data && profiles.data.length === 0 ? (
          <EmptyState
            title="Add a connection"
            body="Use a namespace connection string, or sign in with Entra ID to use your Azure role assignments. Connections are saved on this computer; secrets go to the system keychain."
            action={
              <div className="flex gap-2">
                <Button variant="primary" onClick={() => openSheet({ kind: "profile", profileId: null, auth: "connection-string" })}>Add connection string</Button>
                <Button onClick={() => openSheet({ kind: "profile", profileId: null, auth: "entra" })}>Sign in with Entra ID</Button>
              </div>
            }
          />
        ) : (
          <section className="flex flex-col gap-1.5 px-4">
            <h2 className="m-0 text-13 font-semibold">Connections</h2>
            <ul aria-label="Saved connections" className="m-0 flex list-none flex-col border-y border-line p-0">
              {profiles.data?.map((p) => (
                <li key={p.id}>
                  <button
                    type="button"
                    onClick={() => void conn.connect(p.id)}
                    disabled={conn.status === "connecting"}
                    className="flex h-8 w-full items-center gap-2 px-2 text-left text-12 hover:bg-hover disabled:opacity-60"
                  >
                    <span className="font-semibold">{p.name}</span>
                    <span className="truncate text-muted">{shortHost(p.host)}</span>
                    {p.production && <ProdBadge className="ml-auto" />}
                  </button>
                </li>
              ))}
            </ul>
            <div className="pt-1">
              <Button variant="quiet" onClick={() => openSheet({ kind: "profile", profileId: null })}>Add connection…</Button>
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
```

(The list button's accessible name includes the host so the test can target "Logistics … sb-logistics-prod"; the regex `/Logistics\b.*sb-logistics-prod/` does not match "Logistics dev".)

`app/src/shell/sheets/SheetHost.tsx` (Tasks 18–19 add the sheets it renders):

```tsx
import { useShell } from "../context";

/** Renders the open side sheet, if any. */
export function SheetHost() {
  const { sheet } = useShell();
  if (!sheet) return null;
  return null;
}
```

`app/src/App.tsx`:

```tsx
import { QueryClientProvider } from "@tanstack/react-query";
import { lazy, Suspense, useState } from "react";
import { createQueryClient } from "@/engine";
import { TooltipProvider } from "@/ui";
import { ShellProvider } from "@/shell/context";
import { Workbench } from "@/shell/Workbench";

const catalogueAllowed = import.meta.env.DEV || import.meta.env.VITE_DISPATCHED_MOCK === "1";
const Catalogue = catalogueAllowed ? lazy(() => import("./dev/Catalogue")) : null;

export default function App() {
  const [client] = useState(createQueryClient);
  if (Catalogue && window.location.hash === "#catalogue") {
    return (
      <Suspense fallback={null}>
        <Catalogue />
      </Suspense>
    );
  }
  return (
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <ShellProvider>
          <Workbench />
        </ShellProvider>
      </TooltipProvider>
    </QueryClientProvider>
  );
}
```

Delete `app/src/LegacyApp.tsx` and `app/src/api.ts`.

Append to `shots` in `app/e2e/shots.spec.ts`: `{ name: "welcome", path: "/?scenario=default" }`, `{ name: "welcome-empty", path: "/?scenario=empty" }`, `{ name: "connect-failed", path: "/?scenario=missing-secret&autoconnect=p-logistics" }`.

- [ ] **Step 5: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 6: Finish review**

Run the Finish review procedure for: top bar (connection selector closed and open, PROD badge, palette field), status bar, welcome (connections list, empty, connect failed) in both themes.

- [ ] **Step 7: Commit**

```bash
git add -A app/src app/e2e
git commit -m "Shell: state and persistence, connection controller, layout, top bar, status bar, welcome (finish review: <verdict>)"
```

---
### Task 14: Namespace tree — lazy loading, counts, filter, errors, pinned entities, context menu

**Files:**
- Create: `app/src/shell/tree/model.ts`, `app/src/shell/tree/NamespaceTree.tsx`
- Modify: `app/src/shell/Workbench.tsx` (`tree={<NamespaceTree />}`), `app/e2e/shots.spec.ts`
- Test: `app/src/shell/tree/model.test.ts`, `app/src/shell/tree/NamespaceTree.test.tsx`

**Interfaces:**
- Consumes: `Tree`, `TreeNode`, `CountBadge`, `EntityGlyph`, `ContextMenu*`, `Input`, `SkeletonRows`, `EmptyState`, `rowHeight` (ui); `useEntities`, `useSubscriptionsFor`, `EntityListing`, `SubscriptionInfo`, `EntityCounts` (engine); `useShell`, `EntityKind` (shell).
- Produces:
  - `type NodeKind = "namespace" | "group" | "queue" | "topic" | "subscription" | "known" | "note" | "loading" | "error"`
  - `interface NsNode extends TreeNode { kind: NodeKind; entity?: { id: string; kind: EntityKind }; counts?: EntityCounts | null; topic?: string; message?: string }`
  - `interface SubsState { data?: SubscriptionInfo[]; error?: { message: string } | null; pending: boolean }`
  - `buildNodes(args: { host: string; listing: EntityListing; subs: Record<string, SubsState>; expanded: ReadonlySet<string>; filter: string }): NsNode[]`
  - `entityIdOf(kind, name, topic?)`: queue → `name`, topic → `name`, subscription → `` `${topic}/subscriptions/${name}` `` (the tab id, also a valid `EntityPath`)
  - node ids: `ns`, `group:queues`, `group:topics`, `group:pinned`, `q:<name>`, `t:<name>`, `s:<topic>/<name>`, `k:<path>`, `note:<topic>`, `loading:<topic>`, `error:<topic>`
  - The filter input has `id="tree-filter"` (Task 16's ⌘F focuses it).

- [ ] **Step 1: Write the failing tests**

`app/src/shell/tree/model.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import * as fx from "@/engine/mock/fixtures";
import type { EntityListing } from "@/engine";
import { buildNodes, type SubsState } from "./model";

const listing: EntityListing = { queues: fx.queues, topics: fx.topics, limited: null, known: [] };
const loaded = (topic: string): SubsState => ({ data: fx.subscriptions[topic], error: null, pending: false });
const ids = (n: { id: string }[]) => n.map((x) => x.id);

describe("buildNodes", () => {
  it("builds namespace, groups and entities with depth and parents", () => {
    const n = buildNodes({ host: "sb-logistics-prod", listing, subs: {}, expanded: new Set(["group:queues", "group:topics"]), filter: "" });
    expect(n[0]).toMatchObject({ id: "ns", depth: 0, parentId: null, kind: "namespace" });
    expect(n.find((x) => x.id === "q:invoice-generate")).toMatchObject({ depth: 2, parentId: "group:queues", entity: { id: "invoice-generate", kind: "queue" } });
    expect(n.find((x) => x.id === "t:shipment-events")).toMatchObject({ expandable: true, expanded: false });
    expect(ids(n)).not.toContain("s:shipment-events/tracking-sync");
  });

  it("model_handles_empty_topics_and_unknown_counts", () => {
    const n = buildNodes({
      host: "h", listing,
      subs: { "archive-events": loaded("archive-events"), "stock-levels": loaded("stock-levels") },
      expanded: new Set(["group:topics", "t:archive-events", "t:stock-levels"]), filter: "",
    });
    expect(n.find((x) => x.id === "note:archive-events")).toMatchObject({ kind: "note", label: "No subscriptions", depth: 3 });
    expect(n.find((x) => x.id === "s:stock-levels/replenish")).toMatchObject({ counts: null, entity: { id: "stock-levels/subscriptions/replenish", kind: "subscription" } });
  });

  it("shows loading and error children for expanded topics", () => {
    const n = buildNodes({
      host: "h", listing,
      subs: { "carrier-updates": { pending: true }, "shipment-events": { pending: false, error: { message: "access denied" } } },
      expanded: new Set(["group:topics", "t:carrier-updates", "t:shipment-events"]), filter: "",
    });
    expect(n.find((x) => x.id === "loading:carrier-updates")?.kind).toBe("loading");
    expect(n.find((x) => x.id === "error:shipment-events")).toMatchObject({ kind: "error", message: "access denied" });
  });

  it("filter_is_case_insensitive_and_keeps_ancestors", () => {
    const n = buildNodes({
      host: "h", listing, subs: { "shipment-events": loaded("shipment-events") },
      expanded: new Set(), filter: "TRACK",
    });
    expect(ids(n)).toEqual(["ns", "group:topics", "t:shipment-events", "s:shipment-events/tracking-sync"]);
    expect(n.find((x) => x.id === "t:shipment-events")?.expanded).toBe(true);
  });

  it("limited_listing_shows_known_entities_and_the_reason", () => {
    const limited: EntityListing = {
      queues: [], topics: [], limited: "This identity can't list the namespace (needs Manage). Showing pinned entities.",
      known: [{ entity: "orders", subscription: null }, { entity: "shipment-events", subscription: "tracking-sync" }],
    };
    const n = buildNodes({ host: "h", listing: limited, subs: {}, expanded: new Set(), filter: "" });
    expect(ids(n)).toEqual(["ns", "group:pinned", "k:orders", "k:shipment-events/subscriptions/tracking-sync"]);
    expect(n[2]).toMatchObject({ entity: { id: "orders", kind: "queue" }, counts: null });
    expect(n[3]).toMatchObject({ entity: { id: "shipment-events/subscriptions/tracking-sync", kind: "subscription" } });
  });
});
```

(The known entity `orders` could be a queue or a topic; with no listing rights it is shown as a queue, the CLI's convention for a bare EntityPath.)

`app/src/shell/tree/NamespaceTree.test.tsx`:

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";

const tree = () => screen.findByRole("tree", { name: "Namespace" });

describe("NamespaceTree", () => {
  it("lists queues and topics with counts, amber only on dead letters", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: "invoice-generate" });
    expect(within(row).getByText("1,284").className).not.toContain("text-accent");
    expect(within(row).getByText("37").className).toContain("text-accent");
  });

  it("loads subscriptions when a topic expands", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: "shipment-events" });
    await userEvent.click(row);
    await userEvent.keyboard("{ArrowRight}");
    expect(await within(t).findByRole("treeitem", { name: "tracking-sync" })).toBeInTheDocument();
    expect(mock.calls).toContainEqual({ cmd: "list_subscriptions", args: { topic: "shipment-events" } });
  });

  it("filters across loaded entities", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await within(t).findByRole("treeitem", { name: "label-print" });
    await userEvent.type(screen.getByRole("searchbox", { name: "Filter entities" }), "LABEL");
    await waitFor(() => expect(within(t).queryByRole("treeitem", { name: "invoice-generate" })).not.toBeInTheDocument());
    expect(within(t).getByRole("treeitem", { name: "label-print" })).toBeInTheDocument();
  });

  it("explains a limited listing and shows pinned entities", async () => {
    renderShell("forbidden", { last_profile: "p-logistics" });
    expect(await screen.findByText(/can't list the namespace/)).toBeInTheDocument();
    expect(within(await tree()).getByRole("treeitem", { name: "orders" })).toBeInTheDocument();
  });

  it("shows a retryable error when listing fails", async () => {
    renderShell("network", { last_profile: "p-logistics" });
    expect(await screen.findByRole("heading", { name: "Can't list this namespace" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });
});
```

Run: `npm test -- src/shell/tree`
Expected: FAIL (modules missing).

- [ ] **Step 2: Implement `app/src/shell/tree/model.ts`**

```ts
import type { EntityCounts, EntityListing, EntityRef, SubscriptionInfo } from "@/engine";
import type { TreeNode } from "@/ui";
import type { EntityKind } from "../state";

export type NodeKind = "namespace" | "group" | "queue" | "topic" | "subscription" | "known" | "note" | "loading" | "error";

export interface NsNode extends TreeNode {
  kind: NodeKind;
  entity?: { id: string; kind: EntityKind };
  counts?: EntityCounts | null;
  topic?: string;
  message?: string;
}

export interface SubsState {
  data?: SubscriptionInfo[];
  error?: { message: string } | null;
  pending: boolean;
}

export function entityIdOf(kind: EntityKind, name: string, topic?: string): string {
  return kind === "subscription" ? `${topic}/subscriptions/${name}` : name;
}

const knownId = (k: EntityRef) => (k.subscription ? entityIdOf("subscription", k.subscription, k.entity) : k.entity);

export function buildNodes(args: {
  host: string;
  listing: EntityListing;
  subs: Record<string, SubsState>;
  expanded: ReadonlySet<string>;
  filter: string;
}): NsNode[] {
  const { host, listing, subs, expanded } = args;
  const term = args.filter.trim().toLowerCase();
  const filtering = term.length > 0;
  const hit = (name: string) => !filtering || name.toLowerCase().includes(term);
  const out: NsNode[] = [];
  const push = (n: Omit<NsNode, "expandable" | "expanded"> & { expandable?: boolean; expanded?: boolean }) =>
    out.push({ expandable: false, expanded: false, ...n });

  push({ id: "ns", parentId: null, depth: 0, label: host, kind: "namespace" });

  if (listing.limited) {
    const known = listing.known.filter((k) => hit(knownId(k)));
    if (known.length) {
      push({ id: "group:pinned", parentId: "ns", depth: 1, label: "Pinned entities", kind: "group" });
      for (const k of known) {
        const id = knownId(k);
        push({
          id: `k:${id}`, parentId: "group:pinned", depth: 1, label: k.subscription ? `${k.entity}/${k.subscription}` : k.entity,
          kind: "known", counts: null, entity: { id, kind: k.subscription ? "subscription" : "queue" },
        });
      }
    }
    return out;
  }

  const queues = listing.queues.filter((q) => hit(q.name));
  if (queues.length) {
    const open = filtering || expanded.has("group:queues");
    push({ id: "group:queues", parentId: "ns", depth: 1, label: "Queues", kind: "group", expandable: true, expanded: open });
    if (open) {
      for (const q of queues) {
        push({ id: `q:${q.name}`, parentId: "group:queues", depth: 2, label: q.name, kind: "queue", counts: q.counts, entity: { id: q.name, kind: "queue" } });
      }
    }
  }

  const topicRows = listing.topics
    .map((t) => {
      const s = subs[t.name];
      const matchingSubs = (s?.data ?? []).filter((x) => hit(x.name));
      return { t, s, matchingSubs, show: hit(t.name) || matchingSubs.length > 0 };
    })
    .filter((r) => r.show);

  if (topicRows.length) {
    const open = filtering || expanded.has("group:topics");
    push({ id: "group:topics", parentId: "ns", depth: 1, label: "Topics", kind: "group", expandable: true, expanded: open });
    if (open) {
      for (const { t, s, matchingSubs } of topicRows) {
        const tid = `t:${t.name}`;
        const tOpen = (filtering && matchingSubs.length > 0) || expanded.has(tid);
        push({ id: tid, parentId: "group:topics", depth: 2, label: t.name, kind: "topic", expandable: true, expanded: tOpen, entity: { id: t.name, kind: "topic" } });
        if (!tOpen) continue;
        if (!s || s.pending) {
          push({ id: `loading:${t.name}`, parentId: tid, depth: 3, label: "Loading subscriptions", kind: "loading", topic: t.name });
        } else if (s.error) {
          push({ id: `error:${t.name}`, parentId: tid, depth: 3, label: "Couldn't list subscriptions", kind: "error", topic: t.name, message: s.error.message });
        } else if ((s.data ?? []).length === 0) {
          push({ id: `note:${t.name}`, parentId: tid, depth: 3, label: "No subscriptions", kind: "note", topic: t.name });
        } else {
          for (const sub of filtering ? matchingSubs : s.data!) {
            push({
              id: `s:${t.name}/${sub.name}`, parentId: tid, depth: 3, label: sub.name, kind: "subscription",
              counts: sub.counts, topic: t.name, entity: { id: entityIdOf("subscription", sub.name, t.name), kind: "subscription" },
            });
          }
        }
      }
    }
  }
  return out;
}
```

- [ ] **Step 3: Implement `app/src/shell/tree/NamespaceTree.tsx`**

```tsx
import { CircleAlert } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { useEntities, useSubscriptionsFor } from "@/engine";
import {
  Button, ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
  CountBadge, EmptyState, EntityGlyph, Input, Skeleton, SkeletonRows, Tree, formatNumber, rowHeight,
} from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";
import { buildNodes, type NsNode, type SubsState } from "./model";

const glyphKind = { namespace: "namespace", queue: "queue", topic: "topic", subscription: "subscription", known: "queue" } as const;

function Row({ n }: { n: NsNode }) {
  if (n.kind === "loading") return <Skeleton className="w-32" />;
  if (n.kind === "note") return <span className="text-faint">{n.label}</span>;
  if (n.kind === "error")
    return (
      <span className="flex min-w-0 items-center gap-1.5" title={n.message}>
        <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />
        <span className="truncate">{n.label}: {n.message}</span>
      </span>
    );
  const g = n.kind === "group" ? null : glyphKind[n.kind as keyof typeof glyphKind];
  const known = n.entity?.kind === "subscription" && n.kind === "known" ? "subscription" : g;
  return (
    <>
      {known && <EntityGlyph kind={known} className="shrink-0 text-icon" />}
      <span className={n.kind === "namespace" || n.kind === "group" ? "truncate font-semibold" : "truncate"}>
        {n.kind === "group" ? <span className="text-muted">{n.label}</span> : n.label}
      </span>
      <span className="flex-1" />
      {(n.kind === "queue" || n.kind === "subscription" || n.kind === "known") && (
        <>
          <CountBadge value={n.counts?.active} quality={n.counts ?? undefined} />
          <CountBadge value={n.counts?.dead_letter} quality={n.counts ?? undefined} attention className="min-w-[34px] text-right" />
        </>
      )}
    </>
  );
}

export function NamespaceTree() {
  const { conn, shell, dispatch, prefs } = useShell();
  const profileId = conn.profileId;
  const refetchMs = prefs.autoRefresh * 1000;
  const entities = useEntities(profileId, refetchMs);
  const expandedTopics = shell.expanded.filter((e) => e.startsWith("t:")).map((e) => e.slice(2));
  const subQueries = useSubscriptionsFor(profileId, expandedTopics, refetchMs);
  const [filter, setFilter] = useState("");
  const menuNode = useRef<NsNode | null>(null);

  const subs: Record<string, SubsState> = {};
  expandedTopics.forEach((t, i) => {
    const q = subQueries[i];
    subs[t] = { data: q?.data, error: q?.error ? { message: q.error.message } : null, pending: !q || q.isPending };
  });
  // Subscriptions already loaded for collapsed topics still count for the filter.
  const subsKey = JSON.stringify(Object.entries(subs).map(([t, s]) => [t, s.pending, s.data?.length, s.error?.message]));

  const nodes = useMemo(
    () =>
      entities.data
        ? buildNodes({ host: shortHost(conn.view?.host ?? ""), listing: entities.data, subs, expanded: new Set(shell.expanded), filter })
        : [],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [entities.data, subsKey, shell.expanded, filter, conn.view?.host],
  );

  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);

  function activate(id: string) {
    const n = byId.get(id);
    if (!n) return;
    if (n.entity) dispatch({ type: "open", tab: n.entity });
    else if (n.kind === "error" && n.topic) void subQueries[expandedTopics.indexOf(n.topic)]?.refetch();
    else if (n.expandable) dispatch({ type: "toggleExpanded", nodeId: n.id, expanded: !n.expanded });
  }

  const listed = entities.data && !entities.data.limited;
  const summary = listed ? `${formatNumber(entities.data!.queues.length)} queues, ${formatNumber(entities.data!.topics.length)} topics` : "";

  return (
    <nav aria-label="Namespace" className="flex h-full min-h-0 flex-col">
      <div className="flex h-[30px] shrink-0 items-center justify-between border-b border-line px-3">
        <span className="font-semibold text-muted">Explorer</span>
        <span className="text-11 tabular-nums text-faint">{summary}</span>
      </div>
      <div className="shrink-0 px-2 py-1.5">
        <Input id="tree-filter" type="search" aria-label="Filter entities" placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setFilter("")} />
      </div>
      {entities.error && entities.data && (
        <p className="m-0 flex shrink-0 items-start gap-1.5 border-y border-line px-3 py-2 text-12 text-fg">
          <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
          <span>Showing the last loaded data. {entities.error.message}</span>
        </p>
      )}
      {entities.data?.limited && (
        <p className="m-0 flex shrink-0 items-start gap-1.5 border-y border-line px-3 py-2 text-12 text-fg">
          <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
          <span>{entities.data.limited}</span>
        </p>
      )}
      <div className="min-h-0 flex-1">
        {entities.isPending ? (
          <SkeletonRows rows={12} columns={2} />
        ) : entities.error && !entities.data ? (
          <EmptyState tone="error" title="Can't list this namespace" body={entities.error.message} action={<Button onClick={() => void entities.refetch()}>Retry</Button>} />
        ) : entities.data?.limited && entities.data.known.length === 0 ? (
          <EmptyState title="No pinned entities" body="Edit this connection and add the queues or subscriptions you use, one per line, under Pinned entities." />
        ) : nodes.length === 1 && filter ? (
          <EmptyState title="No matches" body={`Nothing loaded matches "${filter}". Expand topics to search their subscriptions.`} />
        ) : (
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <div
                className="h-full"
                onContextMenuCapture={(e) => {
                  const id = (e.target as HTMLElement).closest<HTMLElement>("[data-node-id]")?.dataset.nodeId;
                  menuNode.current = id ? (byId.get(id) ?? null) : null;
                  if (id) dispatch({ type: "select", nodeId: id });
                }}
              >
                <Tree
                  label="Namespace"
                  nodes={nodes}
                  selectedId={shell.selected}
                  onSelect={(id) => dispatch({ type: "select", nodeId: id })}
                  onActivate={activate}
                  onToggle={(id, expanded) => dispatch({ type: "toggleExpanded", nodeId: id, expanded })}
                  renderRow={(n) => <Row n={n} />}
                  rowHeight={rowHeight[prefs.density]}
                />
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem disabled={!menuNode.current?.entity} onSelect={() => menuNode.current && activate(menuNode.current.id)}>Open</ContextMenuItem>
              <ContextMenuItem disabled={!menuNode.current?.entity} onSelect={() => void navigator.clipboard.writeText(menuNode.current?.label ?? "")}>Copy name</ContextMenuItem>
              <ContextMenuItem disabled={!menuNode.current?.entity} onSelect={() => void navigator.clipboard.writeText(menuNode.current?.entity?.id ?? "")}>Copy entity path</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem onSelect={() => void entities.refetch()}>Refresh</ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        )}
      </div>
    </nav>
  );
}
```

`app/src/shell/Workbench.tsx`: `return <Shell tree={<NamespaceTree />} />;` (import it).

Append to `shots`: `{ name: "tree", path: "/?scenario=default&autoconnect=p-logistics" }` with `prepare` that focuses the tree, selects `shipment-events` (`page.getByRole("treeitem", { name: "shipment-events" }).click()`), presses `ArrowRight`, then selects `tracking-sync`; `{ name: "tree-forbidden", path: "/?scenario=forbidden&autoconnect=p-logistics" }`; `{ name: "tree-error", path: "/?scenario=network&autoconnect=p-logistics" }`; `{ name: "tree-many", path: "/?scenario=many&autoconnect=p-logistics" }`.

- [ ] **Step 4: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 5: Finish review**

Run the Finish review procedure for: the tree (header, filter, rows, counts, amber, selection, expanded topic, loading child, error child, empty topic note), limited banner with pinned entities, listing error state, 2,000-queue tree, context menu.

- [ ] **Step 6: Commit**

```bash
git add app/src app/e2e
git commit -m "Shell: lazy namespace tree with counts, filter, pinned fallback and context menu (finish review: <verdict>)"
```

---

### Task 15: Entity tabs, entity header, sub-tabs and their empty states

**Files:**
- Create: `app/src/shell/tabs/subtabs.ts`, `app/src/shell/tabs/EntityTabs.tsx`, `app/src/shell/tabs/EntityView.tsx`, `app/src/shell/tabs/Workspace.tsx`
- Modify: `app/src/shell/Workbench.tsx` (`workspace={<Workspace />}`), `app/e2e/shots.spec.ts`
- Test: `app/src/shell/tabs/tabs.test.tsx`

**Interfaces:**
- Consumes: `Tabs`, `TabsList`, `TabsTrigger`, `TabsContent`, `IconButton`, `EmptyState`, `CountBadge`, `EntityGlyph`, `Button` (ui); `useEntities`, `keys`, `listSubscriptions` via `useQuery` (engine); `useShell`, `OpenTab`, `SubTabId`, `EntityKind` (shell).
- Produces:
  - `interface SubTabDef { id: SubTabId; label: string; description: string; count?(c: EntityCounts): { value: number; attention?: boolean } }`
  - `subTabsFor(kind: EntityKind): SubTabDef[]` — queue: overview, messages, dead-letter, transfer-dlq, scheduled, deferred, sessions, listener; subscription: the same plus rules after sessions; topic: overview only.
  - `useEntityInfo(tab: OpenTab): { status: "loading" | "found" | "gone" | "unknown"; counts: EntityCounts | null; facts: string[]; parent?: string }`
  - `Workspace()` — tab strip + active `EntityView`, or the "Open an entity" empty state.

Sub-tab descriptions (also the empty-state body):
- Overview: "Properties, settings and live counts for this entity."
- Messages: "Active messages, read by peeking so nothing is locked or removed."
- Dead-letter: "Messages that could not be delivered or processed, with the reason each was dead-lettered."
- Transfer DLQ: "Messages that failed to forward to another entity."
- Scheduled: "Messages waiting for their scheduled enqueue time."
- Deferred: "Messages a receiver set aside to process later."
- Sessions: "Sessions and their state, for session-enabled entities."
- Rules: "Filter rules that decide which messages reach this subscription."
- Listener: "New messages as they arrive."
Empty-state title: `` `${label} isn't available yet` ``; body: the description plus " This view arrives in the next update."

- [ ] **Step 1: Write the failing tests**

`app/src/shell/tabs/tabs.test.tsx`:

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";
import { subTabsFor } from "./subtabs";

describe("sub-tab definitions", () => {
  it("follow the spec per entity kind", () => {
    expect(subTabsFor("queue").map((t) => t.id)).toEqual(["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "listener"]);
    expect(subTabsFor("subscription").map((t) => t.id)).toContain("rules");
    expect(subTabsFor("topic").map((t) => t.id)).toEqual(["overview"]);
  });
});

describe("entity tabs", () => {
  it("open from the tree with Enter and show counts on sub-tabs", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const tree = await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(await within(tree).findByRole("treeitem", { name: "invoice-generate" }));
    await userEvent.keyboard("{Enter}");
    const strip = await screen.findByRole("tablist", { name: "Open entities" });
    expect(within(strip).getByRole("tab", { name: /invoice-generate/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { level: 1, name: "invoice-generate" })).toBeInTheDocument();
    const views = screen.getByRole("tablist", { name: "Entity views" });
    const dl = within(views).getByRole("tab", { name: /Dead-letter/ });
    expect(within(dl).getByText("37").className).toContain("text-accent");
    await userEvent.click(dl);
    expect(screen.getByRole("heading", { name: "Dead-letter isn't available yet" })).toBeInTheDocument();
  });

  it("close with the close button and fall back to the empty workspace", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }], activeTab: "label-print" } });
    await screen.findByRole("tab", { name: /label-print/ });
    await userEvent.click(screen.getByRole("button", { name: "Close label-print" }));
    expect(await screen.findByRole("heading", { name: "Open an entity" })).toBeInTheDocument();
  });

  it("tab_for_missing_entity_shows_gone_state", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "deleted-queue", kind: "queue", subTab: "overview" }], activeTab: "deleted-queue" } });
    expect(await screen.findByRole("heading", { name: "This entity no longer exists" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Close tab" }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: /deleted-queue/ })).not.toBeInTheDocument());
  });

  it("restores the saved sub-tab", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "dead-letter" }], activeTab: "shipment-events/subscriptions/tracking-sync" } });
    const views = await screen.findByRole("tablist", { name: "Entity views" });
    await waitFor(() => expect(within(views).getByRole("tab", { name: /Dead-letter/ })).toHaveAttribute("aria-selected", "true"));
    expect(await within(views).findByText("3,417")).toBeInTheDocument();
  });
});
```

Run: `npm test -- src/shell/tabs`
Expected: FAIL.

- [ ] **Step 2: Implement `subtabs.ts` and `useEntityInfo`**

`app/src/shell/tabs/subtabs.ts`:

```ts
import { useQuery } from "@tanstack/react-query";
import { commands, keys, useEntities, type EntityCounts } from "@/engine";
import { useShell } from "../context";
import type { EntityKind, OpenTab, SubTabId } from "../state";

export interface SubTabDef {
  id: SubTabId;
  label: string;
  description: string;
  count?(c: EntityCounts): { value: number; attention?: boolean };
}

const DEFS: Record<SubTabId, SubTabDef> = {
  overview: { id: "overview", label: "Overview", description: "Properties, settings and live counts for this entity." },
  messages: { id: "messages", label: "Messages", description: "Active messages, read by peeking so nothing is locked or removed.", count: (c) => ({ value: c.active }) },
  "dead-letter": { id: "dead-letter", label: "Dead-letter", description: "Messages that could not be delivered or processed, with the reason each was dead-lettered.", count: (c) => ({ value: c.dead_letter, attention: true }) },
  "transfer-dlq": { id: "transfer-dlq", label: "Transfer DLQ", description: "Messages that failed to forward to another entity.", count: (c) => ({ value: c.transfer_dead_letter, attention: true }) },
  scheduled: { id: "scheduled", label: "Scheduled", description: "Messages waiting for their scheduled enqueue time.", count: (c) => ({ value: c.scheduled }) },
  deferred: { id: "deferred", label: "Deferred", description: "Messages a receiver set aside to process later." },
  sessions: { id: "sessions", label: "Sessions", description: "Sessions and their state, for session-enabled entities." },
  rules: { id: "rules", label: "Rules", description: "Filter rules that decide which messages reach this subscription." },
  listener: { id: "listener", label: "Listener", description: "New messages as they arrive." },
};

const QUEUE: SubTabId[] = ["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "listener"];
const SUBSCRIPTION: SubTabId[] = ["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "rules", "listener"];

export function subTabsFor(kind: EntityKind): SubTabDef[] {
  const ids = kind === "queue" ? QUEUE : kind === "subscription" ? SUBSCRIPTION : (["overview"] as SubTabId[]);
  return ids.map((id) => DEFS[id]);
}

export function splitSubscription(id: string): { topic: string; name: string } {
  const [topic, , name] = id.split("/");
  return { topic, name };
}

const iso = (d: string | null) => {
  const m = d?.match(/^PT(?:(\d+)M)?(?:(\d+)S)?$/);
  if (!m) return d;
  const min = Number(m[1] ?? 0), sec = Number(m[2] ?? 0);
  return min && !sec ? `${min} min` : min ? `${min} min ${sec} s` : `${sec} s`;
};

export function useEntityInfo(tab: OpenTab) {
  const { conn } = useShell();
  const entities = useEntities(conn.profileId);
  const sub = tab.kind === "subscription" ? splitSubscription(tab.id) : null;
  const subs = useQuery({
    queryKey: keys.subscriptions(conn.profileId, sub?.topic ?? ""),
    queryFn: () => commands.listSubscriptions(sub!.topic),
    enabled: sub !== null && conn.profileId !== null && !entities.data?.limited,
  });
  const listing = entities.data;
  if (!listing || (sub && subs.isPending && !listing.limited)) return { status: "loading" as const, counts: null, facts: [] };
  if (listing.limited) return { status: "unknown" as const, counts: null, facts: [], parent: sub?.topic };
  if (tab.kind === "queue") {
    const q = listing.queues.find((x) => x.name === tab.id);
    if (!q) return { status: "gone" as const, counts: null, facts: [] };
    const facts = [q.max_delivery_count != null && `Max delivery ${q.max_delivery_count}`, q.lock_duration && `lock ${iso(q.lock_duration)}`, q.requires_session && "sessions", q.forward_to && `forwards to ${q.forward_to}`].filter(Boolean) as string[];
    return { status: "found" as const, counts: q.counts, facts };
  }
  if (tab.kind === "topic") {
    const t = listing.topics.find((x) => x.name === tab.id);
    if (!t) return { status: "gone" as const, counts: null, facts: [] };
    return { status: "found" as const, counts: null, facts: t.subscription_count != null ? [`${t.subscription_count} subscriptions`] : [] };
  }
  const s = subs.data?.find((x) => x.name === sub!.name);
  if (!s) return { status: subs.error ? ("unknown" as const) : ("gone" as const), counts: null, facts: [], parent: sub!.topic };
  const facts = [s.max_delivery_count != null && `Max delivery ${s.max_delivery_count}`, s.requires_session && "sessions", s.forward_to && `forwards to ${s.forward_to}`].filter(Boolean) as string[];
  return { status: "found" as const, counts: s.counts, facts, parent: sub!.topic };
}
```

- [ ] **Step 3: Implement the tab strip, entity view and workspace**

`app/src/shell/tabs/EntityTabs.tsx`:

```tsx
import { X } from "lucide-react";
import { EntityGlyph, IconButton, Tabs, TabsList, TabsTrigger } from "@/ui";
import { useShell } from "../context";
import { splitSubscription } from "./subtabs";

export function EntityTabs() {
  const { shell, dispatch } = useShell();
  return (
    <Tabs value={shell.activeTab ?? ""} onValueChange={(id) => dispatch({ type: "activate", id })}>
      <TabsList variant="strip" aria-label="Open entities" className="border-b border-line">
        {shell.tabs.map((t) => {
          const sub = t.kind === "subscription" ? splitSubscription(t.id) : null;
          const name = sub ? sub.name : t.id;
          return (
            <div key={t.id} className="group/tab flex items-stretch border-r border-line" onAuxClick={(e) => e.button === 1 && dispatch({ type: "close", id: t.id })}>
              <TabsTrigger value={t.id} className="border-r-0 pr-1.5">
                <EntityGlyph kind={t.kind} className="text-icon" />
                <span>{name}</span>
                {sub && <span className="text-faint">{sub.topic}</span>}
              </TabsTrigger>
              <IconButton
                label={`Close ${name}`}
                onClick={() => dispatch({ type: "close", id: t.id })}
                className="my-auto mr-1 size-5 opacity-0 group-hover/tab:opacity-100 focus-visible:opacity-100 group-has-[[data-state=active]]/tab:opacity-100"
              >
                <X />
              </IconButton>
            </div>
          );
        })}
      </TabsList>
    </Tabs>
  );
}
```

`app/src/shell/tabs/EntityView.tsx`:

```tsx
import { Button, CountBadge, EmptyState, SkeletonRows, Tabs, TabsContent, TabsList, TabsTrigger } from "@/ui";
import { useShell } from "../context";
import type { OpenTab, SubTabId } from "../state";
import { subTabsFor, useEntityInfo } from "./subtabs";

const KIND_LINE = { queue: "Queue", topic: "Topic", subscription: "Subscription" } as const;

export function EntityView({ tab }: { tab: OpenTab }) {
  const { dispatch } = useShell();
  const info = useEntityInfo(tab);
  const name = tab.kind === "subscription" ? tab.id.split("/")[2] : tab.id;

  if (info.status === "loading") return <SkeletonRows rows={6} />;
  if (info.status === "gone")
    return (
      <EmptyState
        tone="error"
        title="This entity no longer exists"
        body={`${name} may have been deleted or renamed. Refresh the tree to see what is there now.`}
        action={<Button onClick={() => dispatch({ type: "close", id: tab.id })}>Close tab</Button>}
      />
    );

  const defs = subTabsFor(tab.kind);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1 px-3.5 pt-2.5">
        <h1 className="m-0 text-14 font-semibold">{name}</h1>
        <span className="text-muted">{tab.kind === "subscription" ? `Subscription on ${info.parent}` : KIND_LINE[tab.kind]}</span>
        {info.facts.length > 0 && <span className="text-faint">{info.facts.join(", ")}</span>}
      </div>
      <Tabs value={tab.subTab} onValueChange={(v) => dispatch({ type: "subTab", id: tab.id, subTab: v as SubTabId })} className="flex min-h-0 flex-1 flex-col">
        <TabsList variant="underline" aria-label="Entity views" className="mt-1.5 border-b border-line">
          {defs.map((d) => {
            const c = d.count && info.counts ? d.count(info.counts) : null;
            return (
              <TabsTrigger key={d.id} value={d.id}>
                <span>{d.label}</span>
                {c && <CountBadge value={c.value} quality={info.counts ?? undefined} attention={c.attention} />}
              </TabsTrigger>
            );
          })}
        </TabsList>
        {defs.map((d) => (
          <TabsContent key={d.id} value={d.id} className="overflow-y-auto">
            <EmptyState title={`${d.label} isn't available yet`} body={`${d.description} This view arrives in the next update.`} />
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
```

`app/src/shell/tabs/Workspace.tsx`:

```tsx
import { EmptyState, Kbd } from "@/ui";
import { useShell } from "../context";
import { EntityTabs } from "./EntityTabs";
import { EntityView } from "./EntityView";

export function Workspace() {
  const { shell } = useShell();
  const active = shell.tabs.find((t) => t.id === shell.activeTab);
  return (
    <main className="flex h-full min-h-0 flex-col">
      {shell.tabs.length > 0 && <EntityTabs />}
      <div className="min-h-0 flex-1">
        {active ? (
          <EntityView key={active.id} tab={active} />
        ) : (
          <EmptyState
            title="Open an entity"
            body={<>Double-click a queue, topic or subscription in the tree, press Enter on it, or press <Kbd shortcut="mod+k" /> and type its name.</>}
          />
        )}
      </div>
    </main>
  );
}
```

`app/src/shell/Workbench.tsx`: `return <Shell tree={<NamespaceTree />} workspace={<Workspace />} />;`

Append to `shots`: `{ name: "entity", path: "/?scenario=default&autoconnect=p-logistics" }` with `prepare` that opens `tracking-sync` (expand `shipment-events`, select `tracking-sync`, Enter), opens `invoice-generate` too, then clicks the `Dead-letter` sub-tab; and `{ name: "workspace-empty", path: "/?scenario=default&autoconnect=p-logistics" }`.

- [ ] **Step 4: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 5: Finish review**

Run the Finish review procedure for: tab strip (active, inactive, hover close), entity header, sub-tab row with counts (amber dead-letter), sub-tab empty states, gone state, empty workspace.

- [ ] **Step 6: Commit**

```bash
git add app/src app/e2e
git commit -m "Shell: entity tabs, header, sub-tabs with counts and empty states (finish review: <verdict>)"
```

---

### Task 16: Keyboard map and the ⌘K palette

**Files:**
- Create: `app/src/shell/keys.ts`, `app/src/shell/Hotkeys.tsx`, `app/src/shell/palette/commands.ts`, `app/src/shell/palette/CommandPalette.tsx`
- Modify: `app/src/shell/Shell.tsx` (render `<Hotkeys />` and `<CommandPalette />`), `app/e2e/shots.spec.ts`
- Test: `app/src/shell/keys.test.ts`, `app/src/shell/palette/palette.test.tsx`

**Interfaces:**
- Consumes: `isMac`, `shortcutLabel`, `CommandPaletteDialog`, `CommandInput`, `CommandList`, `CommandEmpty`, `CommandGroup`, `CommandItem`, `CommandShortcut`, `EntityGlyph` (ui); `useProfiles`, `keys`, `EntityListing`, `SubscriptionInfo` (engine); `useShell`, `entityIdOf` (shell).
- Produces:
  - `matches(e: KeyboardEvent, combo: string, mac?: boolean): boolean` — combos like `"mod+k"`, `"mod+shift+p"`, `"f5"`, `"mod+1"`, `"mod+,"`
  - `useHotkeys(bindings: { combo: string; run(e: KeyboardEvent): void; inInputs?: boolean }[])`
  - `interface PaletteCommand { id: string; group: "Go to" | "Connections" | "App"; label: string; keywords?: string[]; shortcut?: string; glyph?: EntityKind; run(): void }`
  - `buildCommands(ctx): PaletteCommand[]` (pure; tested)
  - Shortcuts: `mod+k` palette (also in inputs), `mod+f` focus `#tree-filter`, `mod+1`…`mod+9` tabs, `mod+w` close active tab, `mod+j` toggle drawer, `mod+,` settings, `f5` refresh.

- [ ] **Step 1: Write the failing tests**

`app/src/shell/keys.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { matches } from "./keys";

const ev = (key: string, mods: Partial<KeyboardEvent> = {}) => new KeyboardEvent("keydown", { key, ...mods });

describe("matches", () => {
  it("maps mod to Cmd on macOS and Ctrl elsewhere", () => {
    expect(matches(ev("k", { metaKey: true }), "mod+k", true)).toBe(true);
    expect(matches(ev("k", { ctrlKey: true }), "mod+k", true)).toBe(false);
    expect(matches(ev("k", { ctrlKey: true }), "mod+k", false)).toBe(true);
  });

  it("requires exact modifiers", () => {
    expect(matches(ev("k", { metaKey: true, shiftKey: true }), "mod+k", true)).toBe(false);
    expect(matches(ev("P", { metaKey: true, shiftKey: true }), "mod+shift+p", true)).toBe(true);
    expect(matches(ev("F5"), "f5")).toBe(true);
    expect(matches(ev("3", { metaKey: true }), "mod+3", true)).toBe(true);
    expect(matches(ev(",", { metaKey: true }), "mod+,", true)).toBe(true);
  });
});
```

`app/src/shell/palette/palette.test.tsx`:

```tsx
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import * as fx from "@/engine/mock/fixtures";
import { renderShell } from "../test-utils";
import { buildCommands } from "./commands";

describe("buildCommands", () => {
  it("offers entities, connections and app commands with shortcuts", () => {
    const cmds = buildCommands({
      listing: { queues: fx.queues, topics: fx.topics, limited: null, known: [] },
      subs: { "shipment-events": fx.subscriptions["shipment-events"] },
      profiles: fx.profiles,
      activeProfileId: "p-logistics",
      open: vi.fn(), connect: vi.fn(), openSheet: vi.fn(), refresh: vi.fn(), toggleDrawer: vi.fn(), setPrefs: vi.fn(),
    });
    const labels = cmds.map((c) => `${c.group}: ${c.label}`);
    expect(labels).toContain("Go to: invoice-generate");
    expect(labels).toContain("Go to: shipment-events/tracking-sync");
    expect(labels).toContain("Connections: Switch to Logistics dev");
    expect(labels).not.toContain("Connections: Switch to Logistics");
    expect(cmds.find((c) => c.label === "Toggle drawer")?.shortcut).toBe("mod+j");
  });
});

describe("palette", () => {
  it("opens with mod+k and opens an entity", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const input = await screen.findByPlaceholderText("Go to an entity or run a command");
    await userEvent.type(input, "label-print");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true"));
  });

  // Needs Task 17's drawer; Task 17 replaces this with the real test.
  it.todo("toggles the drawer with mod+j");
});
```

(Tests run in jsdom where `isMac` is false, so `mod` is Ctrl.)

Run: `npm test -- src/shell/keys src/shell/palette`
Expected: FAIL.

- [ ] **Step 2: Implement keys and hotkeys**

`app/src/shell/keys.ts`:

```ts
import { useEffect, useRef } from "react";
import { isMac } from "@/ui";

export function matches(e: KeyboardEvent, combo: string, mac: boolean = isMac): boolean {
  const parts = combo.toLowerCase().split("+");
  const key = parts.pop()!;
  const want = { mod: parts.includes("mod"), shift: parts.includes("shift"), alt: parts.includes("alt") };
  const mod = mac ? e.metaKey : e.ctrlKey;
  const otherMod = mac ? e.ctrlKey : e.metaKey;
  return e.key.toLowerCase() === key && mod === want.mod && e.shiftKey === want.shift && e.altKey === want.alt && !otherMod;
}

export interface Binding {
  combo: string;
  run(e: KeyboardEvent): void;
  inInputs?: boolean;
}

const typing = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

export function useHotkeys(bindings: Binding[]): void {
  const ref = useRef(bindings);
  ref.current = bindings;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      for (const b of ref.current) {
        if (!matches(e, b.combo)) continue;
        if (typing(e.target) && !b.inInputs) continue;
        e.preventDefault();
        b.run(e);
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
```

`app/src/shell/Hotkeys.tsx`:

```tsx
import { useShell } from "./context";
import { useHotkeys, type Binding } from "./keys";

export function Hotkeys() {
  const { conn, shell, dispatch, setPaletteOpen, openSheet, refresh } = useShell();
  const connected = conn.status === "connected";
  const bindings: Binding[] = [
    { combo: "mod+k", run: () => setPaletteOpen(true), inInputs: true },
    { combo: "mod+,", run: () => openSheet({ kind: "settings" }), inInputs: true },
    { combo: "f5", run: () => connected && refresh(), inInputs: true },
    { combo: "mod+f", run: () => document.getElementById("tree-filter")?.focus() },
    { combo: "mod+j", run: () => connected && dispatch({ type: "layout", patch: { drawerOpen: !shell.layout.drawerOpen } }) },
    { combo: "mod+w", run: () => shell.activeTab && dispatch({ type: "close", id: shell.activeTab }) },
    ...Array.from({ length: 9 }, (_, i) => ({ combo: `mod+${i + 1}`, run: () => dispatch({ type: "activateIndex", index: i }) })),
  ];
  useHotkeys(bindings);
  return null;
}
```

(`mod+w` would close the window on some platforms; Tauri's default menu on macOS binds ⌘W to close the window. If the menu intercepts it, set the window's menu in Task 21's live check, or document that ⌘W closes the tab only when the webview receives it. Check in the live run and note the result.)

- [ ] **Step 3: Implement palette commands and UI**

`app/src/shell/palette/commands.ts`:

```ts
import type { EntityListing, Profile, SubscriptionInfo } from "@/engine";
import type { EntityKind, Prefs } from "../state";
import { entityIdOf } from "../tree/model";
import type { SheetState } from "../context";

export interface PaletteCommand {
  id: string;
  group: "Go to" | "Connections" | "App";
  label: string;
  keywords?: string[];
  shortcut?: string;
  glyph?: EntityKind;
  run(): void;
}

export interface PaletteContext {
  listing: EntityListing | undefined;
  subs: Record<string, SubscriptionInfo[] | undefined>;
  profiles: Profile[];
  activeProfileId: string | null;
  open(tab: { id: string; kind: EntityKind }): void;
  connect(profileId: string): void;
  openSheet(s: Exclude<SheetState, null>): void;
  refresh(): void;
  toggleDrawer(): void;
  setPrefs(p: Partial<Prefs>): void;
}

export function buildCommands(c: PaletteContext): PaletteCommand[] {
  const out: PaletteCommand[] = [];
  const goTo = (id: string, kind: EntityKind, label: string, keywords: string[] = []) =>
    out.push({ id: `go:${kind}:${id}`, group: "Go to", label, glyph: kind, keywords, run: () => c.open({ id, kind }) });
  if (c.listing) {
    for (const q of c.listing.queues) goTo(q.name, "queue", q.name, ["queue"]);
    for (const t of c.listing.topics) {
      goTo(t.name, "topic", t.name, ["topic"]);
      for (const s of c.subs[t.name] ?? []) goTo(entityIdOf("subscription", s.name, t.name), "subscription", `${t.name}/${s.name}`, ["subscription"]);
    }
    for (const k of c.listing.known) {
      const id = k.subscription ? entityIdOf("subscription", k.subscription, k.entity) : k.entity;
      goTo(id, k.subscription ? "subscription" : "queue", k.subscription ? `${k.entity}/${k.subscription}` : k.entity, ["pinned"]);
    }
  }
  for (const p of c.profiles) {
    if (p.id === c.activeProfileId) continue;
    out.push({ id: `conn:${p.id}`, group: "Connections", label: `Switch to ${p.name}`, keywords: [p.host], run: () => c.connect(p.id) });
  }
  out.push({ id: "conn:add", group: "Connections", label: "Add connection", run: () => c.openSheet({ kind: "profile", profileId: null }) });
  out.push({ id: "conn:manage", group: "Connections", label: "Manage connections", run: () => c.openSheet({ kind: "profiles" }) });
  const app: Omit<PaletteCommand, "group">[] = [
    { id: "app:refresh", label: "Refresh", shortcut: "f5", run: c.refresh },
    { id: "app:drawer", label: "Toggle drawer", shortcut: "mod+j", run: c.toggleDrawer },
    { id: "app:theme-system", label: "Theme: follow system", keywords: ["appearance"], run: () => c.setPrefs({ theme: "system" }) },
    { id: "app:theme-dark", label: "Theme: dark", keywords: ["appearance"], run: () => c.setPrefs({ theme: "dark" }) },
    { id: "app:theme-light", label: "Theme: light", keywords: ["appearance"], run: () => c.setPrefs({ theme: "light" }) },
    { id: "app:density-compact", label: "Density: compact", run: () => c.setPrefs({ density: "compact" }) },
    { id: "app:density-comfortable", label: "Density: comfortable", run: () => c.setPrefs({ density: "comfortable" }) },
    { id: "app:settings", label: "Settings", shortcut: "mod+,", run: () => c.openSheet({ kind: "settings" }) },
    { id: "app:update", label: "Check for updates", run: () => c.openSheet({ kind: "update" }) },
  ];
  for (const a of app) out.push({ ...a, group: "App" });
  return out;
}
```

`app/src/shell/palette/CommandPalette.tsx`:

```tsx
import { useQueryClient } from "@tanstack/react-query";
import { keys, useEntities, useProfiles, type SubscriptionInfo } from "@/engine";
import {
  CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog, CommandShortcut, EntityGlyph, shortcutLabel,
} from "@/ui";
import { useShell } from "../context";
import { buildCommands, type PaletteCommand } from "./commands";

export function CommandPalette() {
  const { paletteOpen, setPaletteOpen, conn, shell, dispatch, openSheet, refresh, setPrefs } = useShell();
  const qc = useQueryClient();
  const profiles = useProfiles().data ?? [];
  const listing = useEntities(conn.status === "connected" ? conn.profileId : null).data;

  if (!paletteOpen) return null;

  const subs: Record<string, SubscriptionInfo[] | undefined> = {};
  for (const t of listing?.topics ?? []) subs[t.name] = qc.getQueryData(keys.subscriptions(conn.profileId, t.name));

  const cmds = buildCommands({
    listing,
    subs,
    profiles,
    activeProfileId: conn.status === "connected" ? conn.profileId : null,
    open: (tab) => {
      dispatch({ type: "open", tab });
      dispatch({ type: "select", nodeId: tab.kind === "queue" ? `q:${tab.id}` : tab.kind === "topic" ? `t:${tab.id}` : `s:${tab.id.replace("/subscriptions/", "/")}` });
    },
    connect: (id) => void conn.connect(id),
    openSheet,
    refresh,
    toggleDrawer: () => dispatch({ type: "layout", patch: { drawerOpen: !shell.layout.drawerOpen } }),
    setPrefs,
  });
  const groups = ["Go to", "Connections", "App"] as const;
  const run = (c: PaletteCommand) => {
    setPaletteOpen(false);
    c.run();
  };

  return (
    <CommandPaletteDialog open onOpenChange={setPaletteOpen}>
      <CommandInput placeholder="Go to an entity or run a command" />
      <CommandList>
        <CommandEmpty>No matching entity or command.</CommandEmpty>
        {groups.map((g) => {
          const items = cmds.filter((c) => c.group === g);
          if (!items.length) return null;
          return (
            <CommandGroup key={g} heading={g}>
              {items.map((c) => (
                <CommandItem key={c.id} value={`${c.label} ${c.keywords?.join(" ") ?? ""}`} onSelect={() => run(c)}>
                  {c.glyph && <EntityGlyph kind={c.glyph} />}
                  <span className="truncate">{c.label}</span>
                  {c.shortcut && <CommandShortcut>{shortcutLabel(c.shortcut)}</CommandShortcut>}
                </CommandItem>
              ))}
            </CommandGroup>
          );
        })}
      </CommandList>
    </CommandPaletteDialog>
  );
}
```

In `app/src/shell/Shell.tsx` render `<Hotkeys />` and `<CommandPalette />` next to `<SheetHost />`.

Append to `shots`: `{ name: "palette", path: "/?scenario=default&autoconnect=p-logistics" }` with `prepare` pressing `ControlOrMeta+k` and typing `ship`.

- [ ] **Step 4: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS (the drawer test is `it.todo` until Task 17).

- [ ] **Step 5: Finish review**

Run the Finish review procedure for: the palette (groups, glyphs, shortcuts, selected item, empty result).

- [ ] **Step 6: Commit**

```bash
git add app/src app/e2e
git commit -m "Shell: keyboard map and command palette (finish review: <verdict>)"
```

---

### Task 17: Jobs and Activity drawer

**Files:**
- Create: `app/src/shell/drawer/activity.ts`, `app/src/shell/drawer/Drawer.tsx`
- Modify: `app/src/shell/context.tsx` (start capturing events in `ShellProvider`), `app/src/shell/Workbench.tsx` (`drawer={<Drawer />}`), `app/src/shell/StatusBar.tsx` ("Show activity" when hidden), `app/src/shell/test-utils.tsx` (reset the stores), `app/src/shell/palette/palette.test.tsx` (the real drawer test), `app/e2e/shots.spec.ts`
- Test: `app/src/shell/drawer/activity.test.ts`, `app/src/shell/drawer/Drawer.test.tsx`

**Interfaces:**
- Consumes: `onActivity`, `onJob`, `ActivityEntry`, `JobEvent` (engine); `Tabs*`, `IconButton`, `EmptyState`, `formatClock`, `formatDuration`, `formatNumber`, `Hint` (ui); `useShell` (shell).
- Produces:
  - `ACTIVITY_CAP = 1000`; `createActivityStore()` → `{ add(e), all(), subscribe(cb) }`; `createJobStore()` → `{ apply(e: JobEvent), all(): Job[], subscribe(cb) }` with `Job { id; title; done; total: number | null; error: ActivityError | null; finished: boolean }`
  - module singletons `activityStore`, `jobStore`, `useActivity()`, `useJobs()`, `startEventCapture(): Promise<() => void>`
  - `Drawer()` — region "Jobs and activity" with Jobs/Activity tabs and a collapse button

- [ ] **Step 1: Write the failing tests**

`app/src/shell/drawer/activity.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { ACTIVITY_CAP, createActivityStore, createJobStore } from "./activity";

const entry = (i: number) => ({ at: "2026-10-15T09:41:12Z", action: `Step ${i}`, duration_ms: 12, error: null });

describe("activity store", () => {
  it("keeps the newest entries up to the cap", () => {
    const s = createActivityStore();
    for (let i = 0; i < ACTIVITY_CAP + 25; i++) s.add(entry(i));
    expect(s.all()).toHaveLength(ACTIVITY_CAP);
    expect(s.all()[0].action).toBe("Step 25");
    expect(s.all().at(-1)!.action).toBe(`Step ${ACTIVITY_CAP + 24}`);
  });
});

describe("job store", () => {
  it("tracks start, progress and finish", () => {
    const s = createJobStore();
    s.apply({ type: "started", id: "j1", title: "Export dead letters", total: 3417 });
    s.apply({ type: "progress", id: "j1", done: 1920, total: 3417 });
    expect(s.all()[0]).toMatchObject({ done: 1920, total: 3417, finished: false });
    s.apply({ type: "finished", id: "j1", error: null });
    expect(s.all()[0].finished).toBe(true);
    s.apply({ type: "progress", id: "unknown", done: 1, total: null });
    expect(s.all()).toHaveLength(1);
  });
});
```

`app/src/shell/drawer/Drawer.test.tsx`:

```tsx
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";

describe("Drawer", () => {
  it("logs engine calls with durations and keeps jobs empty with an explanation", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    expect(await within(drawer).findByText("Connected to Logistics")).toBeInTheDocument();
    expect(await within(drawer).findByText(/Listed 4 queues and 4 topics/)).toBeInTheDocument();
    await userEvent.click(within(drawer).getByRole("tab", { name: /Jobs/ }));
    expect(within(drawer).getByText(/Exports and bulk operations show their progress here/)).toBeInTheDocument();
  });

  it("shows failures with their reason", async () => {
    renderShell("network", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    expect(await within(drawer).findByText("Listing the namespace failed")).toBeInTheDocument();
    expect(within(drawer).getByText(/connection reset by peer/)).toBeInTheDocument();
  });
});
```

(The module-level stores outlive a test; `renderShell` must reset them: add `activityStore.clear(); jobStore.clear();` to `renderShell` and give both stores a `clear()`.)

In `app/src/shell/palette/palette.test.tsx`, replace `it.todo("toggles the drawer with mod+j");` with:

```tsx
  it("toggles the drawer with mod+j", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    expect(await screen.findByRole("region", { name: "Jobs and activity" })).toBeInTheDocument();
    await userEvent.keyboard("{Control>}j{/Control}");
    await waitFor(() => expect(screen.queryByRole("region", { name: "Jobs and activity" })).not.toBeInTheDocument());
    await userEvent.keyboard("{Control>}j{/Control}");
    expect(await screen.findByRole("region", { name: "Jobs and activity" })).toBeInTheDocument();
  });
```

Run: `npm test -- src/shell/drawer`
Expected: FAIL.

- [ ] **Step 2: Implement the stores**

`app/src/shell/drawer/activity.ts`:

```ts
import { useSyncExternalStore } from "react";
import { onActivity, onJob, type ActivityEntry, type ActivityError, type JobEvent } from "@/engine";

export const ACTIVITY_CAP = 1000;

function createStore<T>() {
  let items: T[] = [];
  const subs = new Set<() => void>();
  return {
    get: () => items,
    set: (next: T[]) => {
      items = next;
      subs.forEach((f) => f());
    },
    subscribe: (cb: () => void) => {
      subs.add(cb);
      return () => subs.delete(cb);
    },
  };
}

export function createActivityStore() {
  const s = createStore<ActivityEntry>();
  return {
    add: (e: ActivityEntry) => s.set([...s.get(), e].slice(-ACTIVITY_CAP)),
    all: s.get,
    clear: () => s.set([]),
    subscribe: s.subscribe,
  };
}

export interface Job { id: string; title: string; done: number; total: number | null; error: ActivityError | null; finished: boolean }

export function createJobStore() {
  const s = createStore<Job>();
  return {
    apply(e: JobEvent) {
      const jobs = s.get();
      if (e.type === "started") return s.set([...jobs, { id: e.id, title: e.title, done: 0, total: e.total, error: null, finished: false }]);
      if (!jobs.some((j) => j.id === e.id)) return;
      s.set(jobs.map((j) => (j.id !== e.id ? j : e.type === "progress" ? { ...j, done: e.done, total: e.total } : { ...j, finished: true, error: e.error })));
    },
    all: s.get,
    clear: () => s.set([]),
    subscribe: s.subscribe,
  };
}

export const activityStore = createActivityStore();
export const jobStore = createJobStore();

export const useActivity = () => useSyncExternalStore(activityStore.subscribe, activityStore.all);
export const useJobs = () => useSyncExternalStore(jobStore.subscribe, jobStore.all);

/** Subscribe the stores to the engine's events. Returns an unsubscribe. */
export async function startEventCapture(): Promise<() => void> {
  const offs = await Promise.all([onActivity((e) => activityStore.add(e)), onJob((e) => jobStore.apply(e))]);
  return () => offs.forEach((off) => off());
}
```

In `ShellProvider` (`context.tsx`), before the other hooks:

```tsx
  useEffect(() => {
    let off: (() => void) | undefined;
    let live = true;
    void startEventCapture().then((f) => (live ? (off = f) : f()));
    return () => {
      live = false;
      off?.();
    };
  }, []);
```

(Capture must start before the auto-connect fires so "Connected to …" is logged; `useConnectionController`'s auto-connect waits on two IPC round trips, which is enough. If the test shows a race, start capture in `main.tsx` before rendering instead.)

- [ ] **Step 3: Implement `Drawer.tsx`**

```tsx
import { ChevronDown, CircleAlert } from "lucide-react";
import { useEffect, useRef } from "react";
import { EmptyState, Hint, IconButton, Tabs, TabsContent, TabsList, TabsTrigger, formatClock, formatDuration, formatNumber } from "@/ui";
import { useShell } from "../context";
import { useActivity, useJobs } from "./activity";

function ActivityLog() {
  const entries = useActivity();
  const end = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = box.current;
    // Follow new entries only when already at the bottom.
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 40) end.current?.scrollIntoView({ block: "end" });
  }, [entries.length]);
  if (!entries.length)
    return <EmptyState title="No activity yet" body="Every call to Service Bus appears here: what ran, how long it took, and why anything failed." />;
  return (
    <div ref={box} className="h-full overflow-y-auto py-1 font-mono text-11 tabular-nums" data-selectable>
      {entries.map((e, i) => (
        <div key={i} className="grid grid-cols-[64px_minmax(0,1fr)_72px] gap-x-3 px-3.5 leading-[18px]">
          <span className="text-faint">{formatClock(e.at)}</span>
          <span className="min-w-0">
            <span className={e.error ? "text-fg" : "text-muted"}>{e.action}</span>
            {e.error && (
              <span className="flex items-start gap-1.5 text-fg">
                <CircleAlert aria-hidden="true" className="mt-0.5 size-3 shrink-0 text-accent" />
                <span>{e.error.message}</span>
              </span>
            )}
          </span>
          <span className="text-right text-faint">{formatDuration(e.duration_ms)}</span>
        </div>
      ))}
      <div ref={end} />
    </div>
  );
}

function JobsList() {
  const jobs = useJobs();
  if (!jobs.length)
    return <EmptyState title="No jobs" body="Exports and bulk operations show their progress here, and keep running while you work." />;
  return (
    <ul className="m-0 list-none p-0">
      {jobs.map((j) => (
        <li key={j.id} className="grid grid-cols-[minmax(240px,1fr)_200px_120px] items-center gap-x-4 px-3.5 py-2 text-12">
          <span className="truncate">{j.title}</span>
          <span className="block h-[3px] bg-line" aria-hidden="true">
            <span className="block h-[3px] bg-fg" style={{ width: j.total ? `${Math.min(100, (j.done / j.total) * 100)}%` : j.finished ? "100%" : "0%" }} />
          </span>
          <span className="font-mono text-11 tabular-nums text-muted">
            {j.error ? <span className="text-fg">{j.error.message}</span> : j.finished ? "Done" : `${formatNumber(j.done)}${j.total ? ` / ${formatNumber(j.total)}` : ""}`}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function Drawer() {
  const { shell, dispatch } = useShell();
  const jobs = useJobs();
  const running = jobs.filter((j) => !j.finished).length;
  return (
    <section aria-label="Jobs and activity" className="flex h-full min-h-0 flex-col">
      <Tabs value={shell.layout.drawerTab} onValueChange={(v) => dispatch({ type: "layout", patch: { drawerTab: v as "jobs" | "activity" } })} className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-[26px] shrink-0 items-stretch border-b border-line pr-1">
          <TabsList variant="underline" aria-label="Drawer" className="h-[26px] px-1.5">
            <TabsTrigger value="jobs">Jobs {running > 0 && <span className="tabular-nums text-muted">{running}</span>}</TabsTrigger>
            <TabsTrigger value="activity">Activity</TabsTrigger>
          </TabsList>
          <div className="flex-1" />
          <Hint label="Hide drawer" shortcut="mod+j">
            <IconButton label="Hide drawer" className="my-auto size-5" onClick={() => dispatch({ type: "layout", patch: { drawerOpen: false } })}>
              <ChevronDown />
            </IconButton>
          </Hint>
        </div>
        <TabsContent value="jobs" className="overflow-y-auto"><JobsList /></TabsContent>
        <TabsContent value="activity"><ActivityLog /></TabsContent>
      </Tabs>
    </section>
  );
}
```

`Workbench`: `return <Shell tree={<NamespaceTree />} workspace={<Workspace />} drawer={<Drawer />} />;`

When the drawer is hidden, the status bar shows a quiet "Activity" button that reopens it: add to `StatusBar.tsx` (left side, after the state) `{conn.status === "connected" && !shell.layout.drawerOpen && <Button size="sm" variant="quiet" onClick={() => dispatch({ type: "layout", patch: { drawerOpen: true } })}>Show activity</Button>}` (read `shell` and `dispatch` from `useShell()`).

Append to `shots`: `{ name: "drawer", path: "/?scenario=network&autoconnect=p-logistics" }`.

- [ ] **Step 4: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS, including the drawer toggle test in `palette.test.tsx`.

- [ ] **Step 5: Finish review**

Run the Finish review procedure for: drawer header, activity rows (success and failure), jobs empty state, hidden drawer with the status bar button.

- [ ] **Step 6: Commit**

```bash
git add app/src app/e2e
git commit -m "Shell: jobs and activity drawer fed by engine events (finish review: <verdict>)"
```

---
### Task 18: Connection sheets — add, edit, test, delete, Entra sign-in

**Files:**
- Create: `app/src/ui/components/segmented.tsx`, `app/src/ui/components/label-swatch.tsx`
- Modify: `app/src/ui/components/input.tsx` (add `Textarea`), `app/src/ui/tokens.ts` (label colours), `app/src/ui/theme.ts` (write `--label-*`), `app/src/ui/index.ts`, `app/src/dev/Catalogue.tsx` (segmented, textarea, swatches)
- Create: `app/src/shell/sheets/ProfileSheet.tsx`, `app/src/shell/sheets/ProfilesSheet.tsx`, `app/src/shell/sheets/SignInPanel.tsx`
- Modify: `app/src/shell/sheets/SheetHost.tsx`, `app/src/shell/ConnectionSelector.tsx` and `app/src/shell/Welcome.tsx` (show label swatches), `app/e2e/shots.spec.ts`
- Test: `app/src/shell/sheets/ProfileSheet.test.tsx`, additions to `app/src/ui/tokens.test.ts`

**Interfaces:**
- Consumes: `Sheet`, `Field`, `Input`, `Select*`, `Button`, `EmptyState`, `ProdBadge` (ui); `useProfiles`, `useSaveProfile`, `useDeleteProfile`, `useTestProfile`, `useAccount`, `useSignIn`, `commands.signInCancel`, `onSignInPrompt`, `Profile`, `ProfileDraft`, `ConnectionReport`, `SignInPrompt`, `EngineError` (engine); `useShell`, `shortHost` (shell).
- Produces:
  - ui: `Segmented<T extends string>({ label, value, onChange, options: { value: T; label: string }[] })` (Radix ToggleGroup, single), `Textarea`, `LabelColor = "slate" | "blue" | "green" | "violet" | "rose"`, `labelColors`, `LabelSwatch({ color: string | null })` (renders nothing for unknown or null)
  - shell: `ProfileSheet({ profileId: string | null; initialAuth?: "connection-string" | "entra" })`, `ProfilesSheet()`, `SignInPanel({ onDone? })`

Label colours are the one exception to "roles only": a user-chosen identification swatch (8px square) next to a connection's name, never used for text or surfaces.

- [ ] **Step 1: Write the failing tests**

Add to `app/src/ui/tokens.test.ts`:

```ts
import { labelColors } from "./tokens";

describe("label colours", () => {
  it("are distinguishable on the side surface (3:1) in both themes", () => {
    for (const [name, c] of Object.entries(labelColors)) {
      for (const theme of ["dark", "light"] as const) {
        expect(contrastRatio(c[theme], themes[theme].side), `${name} ${theme}`).toBeGreaterThanOrEqual(3);
      }
    }
  });
});
```

`app/src/shell/sheets/ProfileSheet.test.tsx`:

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";

const CS = "Endpoint=sb://orders.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0";

describe("ProfileSheet", () => {
  it("adds a connection-string profile and connects to it", async () => {
    renderShell("empty");
    await userEvent.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await userEvent.type(within(sheet).getByLabelText("Name"), "Orders");
    await userEvent.type(within(sheet).getByLabelText("Connection string"), CS);
    await userEvent.click(within(sheet).getByRole("checkbox", { name: /Production/ }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(await screen.findByRole("button", { name: /Connection: Orders/ })).toBeInTheDocument();
    expect(screen.getByText("PROD")).toBeInTheDocument();
  });

  it("validates before saving", async () => {
    const { mock } = renderShell("empty");
    await userEvent.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(within(sheet).getByText("Enter a name for this connection.")).toBeInTheDocument();
    expect(within(sheet).getByText("Paste the namespace's connection string.")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
  });

  it("shows a duplicate name under the Name field", async () => {
    renderShell("default");
    await userEvent.click(await screen.findByRole("button", { name: "Add connection…" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await userEvent.type(within(sheet).getByLabelText("Name"), "logistics");
    await userEvent.type(within(sheet).getByLabelText("Connection string"), CS);
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(await within(sheet).findByText(/already exists/)).toBeInTheDocument();
    expect(within(sheet).getByLabelText("Name")).toHaveAttribute("aria-invalid", "true");
  });

  it("edits without ever showing the stored secret, tests, and deletes after confirming", async () => {
    renderShell("default");
    await userEvent.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await userEvent.click(within(list).getByRole("button", { name: "Edit Logistics" }));
    const sheet = await screen.findByRole("dialog", { name: "Edit Logistics" });
    expect(within(sheet).getByText("Stored in the system keychain.")).toBeInTheDocument();
    expect(within(sheet).queryByLabelText("Connection string")).not.toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Test connection" }));
    expect(await within(sheet).findByText(/listed 4 queue/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete…" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete Logistics" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit Logistics" })).not.toBeInTheDocument());
    expect(screen.queryByRole("button", { name: /^Logistics\b.*sb-logistics-prod/ })).not.toBeInTheDocument();
  });

  it("signs in with a device code and then adds an Entra profile", async () => {
    renderShell("empty");
    await userEvent.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Use a device code instead" }));
    expect(await within(sheet).findByText("QX4T-9KLM")).toBeInTheDocument();
    expect(await within(sheet).findByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    await userEvent.type(within(sheet).getByLabelText("Name"), "Ops");
    await userEvent.type(within(sheet).getByLabelText("Namespace"), "sb-ops");
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("button", { name: /Connection: Ops/ })).toBeInTheDocument();
  });
});
```

Run: `npm test -- src/shell/sheets src/ui/tokens`
Expected: FAIL.

- [ ] **Step 2: Implement the ui additions**

`app/src/ui/tokens.ts` (append):

```ts
export type LabelColor = "slate" | "blue" | "green" | "violet" | "rose";

/** User-chosen connection labels: an 8px swatch only, never text or surfaces. */
export const labelColors: Record<LabelColor, Record<ThemeName, string>> = {
  slate: { dark: "#8a94a6", light: "#5f6b7d" },
  blue: { dark: "#6f9fd8", light: "#2f6db5" },
  green: { dark: "#6fb88a", light: "#2f7d4f" },
  violet: { dark: "#a08ad8", light: "#6b4fb5" },
  rose: { dark: "#d88a9f", light: "#b04a66" },
};
```

`app/src/ui/theme.ts` — inside `applyTheme`, after the roles loop:

```ts
  for (const [name, c] of Object.entries(labelColors)) root.style.setProperty(`--label-${name}`, c[themeName]);
```

(rename the `name` parameter of `applyTheme` to `themeName` to avoid the clash, and import `labelColors`).

`app/src/ui/components/label-swatch.tsx`:

```tsx
import { labelColors, type LabelColor } from "../tokens";

export function LabelSwatch({ color }: { color: string | null | undefined }) {
  if (!color || !(color in labelColors)) return null;
  return <span aria-hidden="true" className="inline-block size-2 shrink-0 rounded-[2px]" style={{ background: `var(--label-${color as LabelColor})` }} />;
}
```

`app/src/ui/components/segmented.tsx`:

```tsx
import { ToggleGroup } from "radix-ui";
import { cn } from "../lib/cn";

export function Segmented<T extends string>({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <ToggleGroup.Root
      type="single"
      aria-label={label}
      value={value}
      onValueChange={(v) => v && onChange(v as T)}
      className="inline-flex h-6 items-stretch rounded-ctl border border-line bg-field p-px"
    >
      {options.map((o) => (
        <ToggleGroup.Item
          key={o.value}
          value={o.value}
          className={cn(
            "rounded-[2px] px-2 text-12 text-muted outline-none hover:text-fg",
            "data-[state=on]:bg-selection data-[state=on]:text-fg focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg",
          )}
        >
          {o.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  );
}
```

`app/src/ui/components/input.tsx` (append):

```tsx
export function Textarea({ className, mono, ...props }: ComponentProps<"textarea"> & { mono?: boolean }) {
  return (
    <textarea
      spellCheck={false}
      className={cn(
        "min-h-16 w-full resize-y rounded-ctl border border-line bg-field px-2 py-1 text-12 text-fg outline-none placeholder:text-faint",
        "focus-visible:border-fg aria-invalid:border-accent",
        mono && "font-mono text-11",
        className,
      )}
      {...props}
    />
  );
}
```

Export `segmented`, `label-swatch` from `ui/index.ts`. Add a catalogue row with a `Segmented` (System/Dark/Light), a `Textarea`, and the five swatches.

- [ ] **Step 3: Implement the sign-in panel**

`app/src/shell/sheets/SignInPanel.tsx`:

```tsx
import { useEffect, useState } from "react";
import { commands, onSignInPrompt, useAccount, useSignIn, type EngineError, type SignInPrompt } from "@/engine";
import { Button } from "@/ui";

export function SignInPanel() {
  const account = useAccount();
  const signIn = useSignIn();
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    let live = true;
    void onSignInPrompt((p) => setPrompt(p)).then((f) => (live ? (off = f) : f()));
    return () => {
      live = false;
      off?.();
    };
  }, []);

  const start = (deviceCode: boolean) => {
    setPrompt(null);
    signIn.mutate({ tenant: null, deviceCode }, { onSettled: () => setPrompt(null) });
  };

  if (signIn.isPending) {
    return (
      <div className="flex flex-col gap-2 border border-line p-3">
        {prompt?.kind === "device-code" ? (
          <>
            <p className="m-0 text-12">Open <span className="font-mono text-11" data-selectable>{prompt.url}</span> and enter this code:</p>
            <p className="m-0 font-mono text-14 tracking-[0.08em]" data-selectable>{prompt.user_code}</p>
            <div className="flex gap-2">
              <Button size="sm" onClick={() => void navigator.clipboard.writeText(prompt.user_code ?? "")}>Copy code</Button>
              <Button size="sm" variant="quiet" onClick={() => void commands.signInCancel()}>Cancel</Button>
            </div>
          </>
        ) : (
          <>
            <p className="m-0 text-12">{prompt?.message ?? "Opening your browser…"}</p>
            <div className="flex gap-2">
              <Button size="sm" variant="quiet" onClick={() => void commands.signInCancel()}>Cancel</Button>
            </div>
          </>
        )}
      </div>
    );
  }

  if (account.data) {
    return (
      <div className="flex items-center justify-between gap-2 text-12">
        <span>Signed in as <span className="font-semibold">{account.data.user_principal}</span></span>
        <Button size="sm" variant="quiet" onClick={() => start(false)}>Use another account</Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-start gap-1.5">
      <Button onClick={() => start(false)}>Sign in with Entra ID</Button>
      <Button size="sm" variant="quiet" onClick={() => start(true)}>Use a device code instead</Button>
      {signIn.error && (signIn.error as EngineError).kind !== "cancelled" && <p className="m-0 text-12 text-fg">{signIn.error.message}</p>}
    </div>
  );
}
```

- [ ] **Step 4: Implement the profile sheets**

`app/src/shell/sheets/ProfileSheet.tsx`:

```tsx
import { CircleAlert, CircleCheck } from "lucide-react";
import { useState } from "react";
import {
  useAccount, useDeleteProfile, useProfiles, useSaveProfile, useTestProfile,
  type Check, type EngineError, type Profile, type ProfileDraft, type Transport,
} from "@/engine";
import {
  Button, Field, Input, LabelSwatch, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, Textarea,
  labelColors,
} from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";
import { SignInPanel } from "./SignInPanel";

type AuthKind = "connection-string" | "entra";

interface Form {
  name: string;
  auth: AuthKind;
  connectionString: string;
  replaceSecret: boolean;
  namespace: string;
  tenant: string;
  production: boolean;
  color: string;
  transport: Transport;
  proxy: string;
  pins: string;
}

type Errors = Partial<Record<"name" | "connectionString" | "namespace" | "proxy" | "pins" | "general", string>>;

function initial(p: Profile | undefined, auth: AuthKind | undefined): Form {
  return {
    name: p?.name ?? "",
    auth: p ? (p.auth.kind === "entra" ? "entra" : "connection-string") : (auth ?? "connection-string"),
    connectionString: "",
    replaceSecret: false,
    namespace: p ? shortHost(p.host) : "",
    tenant: p?.auth.kind === "entra" ? (p.auth.tenant ?? "") : "",
    production: p?.production ?? false,
    color: p?.color ?? "none",
    transport: p?.transport ?? "amqp",
    proxy: p?.proxy ?? "",
    pins: (p?.pinned_entities ?? []).join("\n"),
  };
}

/** Put a server error under the field it is about. */
function placeError(e: EngineError): Errors {
  const m = e.message;
  if (/already exists|name/i.test(m) && e.kind === "invalid") return { name: m };
  if (/connection string|Endpoint|SharedAccess/i.test(m)) return { connectionString: m };
  if (/namespace/i.test(m)) return { namespace: m };
  if (/proxy/i.test(m)) return { proxy: m };
  if (/pinned|entity/i.test(m)) return { pins: m };
  return { general: m };
}

function CheckLine({ label, c }: { label: string; c: Check }) {
  return (
    <li className="flex items-start gap-1.5 text-12">
      {c.status === "failed" ? <CircleAlert aria-hidden="true" className="mt-px size-3.5 text-accent" /> : <CircleCheck aria-hidden="true" className="mt-px size-3.5 text-icon" />}
      <span>
        <span className="font-semibold">{label}: </span>
        {c.status === "ok" ? c.detail : c.status === "failed" ? c.message : <span className="text-muted">{c.reason}</span>}
      </span>
    </li>
  );
}

export function ProfileSheet({ profileId, initialAuth }: { profileId: string | null; initialAuth?: AuthKind }) {
  const { closeSheet, conn } = useShell();
  const profile = useProfiles().data?.find((p) => p.id === profileId);
  const editing = !!profile;
  const [f, setF] = useState<Form>(() => initial(profile, initialAuth));
  const [errors, setErrors] = useState<Errors>({});
  const [confirmDelete, setConfirmDelete] = useState(false);
  const save = useSaveProfile();
  const del = useDeleteProfile();
  const test = useTestProfile();
  const account = useAccount();
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setF((x) => ({ ...x, [k]: v }));

  function validate(): Errors {
    const e: Errors = {};
    if (!f.name.trim()) e.name = "Enter a name for this connection.";
    if (f.auth === "connection-string" && (!editing || f.replaceSecret) && !f.connectionString.trim())
      e.connectionString = "Paste the namespace's connection string.";
    if (f.auth === "entra" && !f.namespace.trim()) e.namespace = "Enter the namespace name, like sb-orders, or its full host.";
    if (f.auth === "entra" && !editing && !account.data) e.general = "Sign in with Entra ID first.";
    return e;
  }

  async function onSave() {
    const e = validate();
    setErrors(e);
    if (Object.keys(e).length) return;
    const draft: ProfileDraft = {
      id: profile?.id ?? null,
      name: f.name.trim(),
      auth:
        f.auth === "entra"
          ? { kind: "entra", namespace: f.namespace.trim(), tenant: f.tenant.trim() || null }
          : { kind: "connection-string", connection_string: f.replaceSecret || !editing ? f.connectionString.trim() : null },
      production: f.production,
      color: f.color === "none" ? null : f.color,
      transport: f.transport,
      proxy: f.proxy.trim() || null,
      pinned_entities: f.pins.split("\n").map((s) => s.trim()).filter(Boolean),
    };
    try {
      const saved = await save.mutateAsync(draft);
      setF((x) => ({ ...x, connectionString: "" }));
      closeSheet();
      if (!editing || saved.id === conn.profileId) void conn.connect(saved.id);
    } catch (err) {
      setErrors(placeError(err as EngineError));
    }
  }

  async function onDelete() {
    if (!profile) return;
    try {
      await del.mutateAsync(profile.id);
      if (conn.profileId === profile.id) await conn.disconnect();
      closeSheet();
    } catch (err) {
      setErrors({ general: (err as EngineError).message });
    }
  }

  const title = editing ? `Edit ${profile.name}` : "Add connection";
  return (
    <Sheet
      open
      onOpenChange={(o) => !o && closeSheet()}
      title={title}
      description={editing ? undefined : "Saved on this computer. Secrets go to the system keychain."}
      footer={
        confirmDelete && profile ? (
          <div className="flex w-full flex-col gap-2">
            <p className="m-0 text-12">
              Delete {profile.name}?{" "}
              {profile.auth.kind === "connection-string" ? "Its saved connection string is removed from the keychain too." : "Your Entra sign-in stays."}
            </p>
            <div className="flex justify-end gap-2">
              <Button variant="quiet" onClick={() => setConfirmDelete(false)}>Keep</Button>
              <Button variant="danger" onClick={() => void onDelete()} disabled={del.isPending}>Delete {profile.name}</Button>
            </div>
          </div>
        ) : (
          <>
            {editing && <Button variant="quiet" onClick={() => setConfirmDelete(true)}>Delete…</Button>}
            {editing && <Button onClick={() => test.mutate(profile!.id)} disabled={test.isPending}>{test.isPending ? "Testing…" : "Test connection"}</Button>}
            <div className="flex-1" />
            <Button variant="quiet" onClick={closeSheet}>Cancel</Button>
            <Button variant="primary" onClick={() => void onSave()} disabled={save.isPending}>{save.isPending ? "Saving…" : "Save"}</Button>
          </>
        )
      }
    >
      <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); void onSave(); }}>
        {errors.general && (
          <p className="m-0 flex items-start gap-1.5 text-12"><CircleAlert aria-hidden="true" className="mt-px size-3.5 text-accent" />{errors.general}</p>
        )}
        <Field label="Name" error={errors.name}>{(ids) => <Input {...ids} value={f.name} onChange={(e) => set("name", e.target.value)} placeholder="Logistics production" />}</Field>

        {!editing && (
          <div className="flex flex-col gap-1">
            <span className="text-12 font-semibold text-muted">Sign-in method</span>
            <Segmented label="Sign-in method" value={f.auth} onChange={(v) => set("auth", v)} options={[{ value: "connection-string", label: "Connection string" }, { value: "entra", label: "Entra ID" }]} />
          </div>
        )}

        {f.auth === "connection-string" ? (
          editing && !f.replaceSecret ? (
            <div className="flex items-center justify-between gap-2 text-12">
              <span className="text-muted">Stored in the system keychain.</span>
              <Button size="sm" onClick={() => set("replaceSecret", true)}>Replace</Button>
            </div>
          ) : (
            <Field label="Connection string" hint="From Shared access policies in the Azure Portal. It is stored in the system keychain and never shown again." error={errors.connectionString}>
              {(ids) => <Input {...ids} type="password" mono value={f.connectionString} onChange={(e) => set("connectionString", e.target.value)} placeholder="Endpoint=sb://…" />}
            </Field>
          )
        ) : (
          <>
            {!editing && <SignInPanel />}
            <Field label="Namespace" hint="The namespace name, like sb-orders, or its full host." error={errors.namespace}>
              {(ids) => <Input {...ids} value={f.namespace} onChange={(e) => set("namespace", e.target.value)} placeholder="sb-orders" />}
            </Field>
            <Field label="Tenant" hint="Only if the namespace is in another tenant than your account's home tenant.">
              {(ids) => <Input {...ids} value={f.tenant} onChange={(e) => set("tenant", e.target.value)} placeholder="contoso.onmicrosoft.com" />}
            </Field>
          </>
        )}

        <label className="flex items-start gap-2 text-12">
          <input type="checkbox" className="mt-0.5 size-3.5 accent-[var(--c-fg)]" checked={f.production} onChange={(e) => set("production", e.target.checked)} />
          <span>
            <span className="font-semibold">Production</span>
            <span className="block text-muted">Marked PROD everywhere. Destructive actions will ask you to type the entity name.</span>
          </span>
        </label>

        <div className="grid grid-cols-2 gap-3">
          <div className="flex flex-col gap-1">
            <span className="text-12 font-semibold text-muted">Label</span>
            <Select value={f.color} onValueChange={(v) => set("color", v)}>
              <SelectTrigger aria-label="Label"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                {Object.keys(labelColors).map((c) => (
                  <SelectItem key={c} value={c}><span className="flex items-center gap-2"><LabelSwatch color={c} />{c[0].toUpperCase() + c.slice(1)}</span></SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-12 font-semibold text-muted">Transport</span>
            <Select value={f.transport} onValueChange={(v) => set("transport", v as Transport)}>
              <SelectTrigger aria-label="Transport"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="amqp">AMQP</SelectItem>
                <SelectItem value="websockets">AMQP over WebSockets</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <Field label="Proxy" hint="http://host:port. Uses WebSockets. Leave empty to use HTTPS_PROXY from the environment." error={errors.proxy}>
          {(ids) => <Input {...ids} mono value={f.proxy} onChange={(e) => set("proxy", e.target.value)} placeholder="http://proxy.corp:8080" />}
        </Field>
        <Field label="Pinned entities" hint="One per line, like orders or events/subscriptions/audit. Shown when this identity can't list the namespace." error={errors.pins}>
          {(ids) => <Textarea {...ids} mono value={f.pins} onChange={(e) => set("pins", e.target.value)} />}
        </Field>

        {test.data && (
          <ul aria-label="Connection test" className="m-0 flex list-none flex-col gap-1 border-t border-line p-0 pt-3">
            <CheckLine label="Management" c={test.data.management} />
            <CheckLine label="Messaging" c={test.data.messaging} />
          </ul>
        )}
        {test.error && <p className="m-0 text-12">{test.error.message}</p>}
      </form>
    </Sheet>
  );
}
```

`app/src/shell/sheets/ProfilesSheet.tsx`:

```tsx
import { useProfiles } from "@/engine";
import { Button, LabelSwatch, ProdBadge, Sheet } from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";

export function ProfilesSheet() {
  const { closeSheet, openSheet, conn } = useShell();
  const profiles = useProfiles().data ?? [];
  return (
    <Sheet
      open
      onOpenChange={(o) => !o && closeSheet()}
      title="Connections"
      footer={<Button variant="primary" onClick={() => openSheet({ kind: "profile", profileId: null })}>Add connection</Button>}
    >
      <ul className="m-0 flex list-none flex-col border-y border-line p-0">
        {profiles.map((p) => (
          <li key={p.id} className="flex h-9 items-center gap-2 px-1 text-12">
            <LabelSwatch color={p.color} />
            <span className="font-semibold">{p.name}</span>
            <span className="truncate text-muted">{shortHost(p.host)}</span>
            {p.production && <ProdBadge />}
            <span className="flex-1" />
            <Button size="sm" variant="quiet" aria-label={`Edit ${p.name}`} onClick={() => openSheet({ kind: "profile", profileId: p.id })}>Edit</Button>
            <Button size="sm" aria-label={`Connect to ${p.name}`} disabled={conn.profileId === p.id && conn.status === "connected"} onClick={() => { closeSheet(); void conn.connect(p.id); }}>
              Connect
            </Button>
          </li>
        ))}
      </ul>
    </Sheet>
  );
}
```

`app/src/shell/sheets/SheetHost.tsx`:

```tsx
import { useShell } from "../context";
import { ProfileSheet } from "./ProfileSheet";
import { ProfilesSheet } from "./ProfilesSheet";

export function SheetHost() {
  const { sheet } = useShell();
  if (!sheet) return null;
  switch (sheet.kind) {
    case "profile":
      return <ProfileSheet key={sheet.profileId ?? `new-${sheet.auth ?? ""}`} profileId={sheet.profileId} initialAuth={sheet.auth} />;
    case "profiles":
      return <ProfilesSheet />;
    default:
      return null;
  }
}
```

Show `LabelSwatch` before the profile name in `ConnectionSelector` (trigger and items) and in `Welcome`'s list.

Append to `shots`: `{ name: "profile-add", path: "/?scenario=empty" }` (prepare: click "Add connection string"), `{ name: "profile-entra", path: "/?scenario=empty" }` (prepare: "Sign in with Entra ID", then "Use a device code instead"), `{ name: "profile-edit", path: "/?scenario=default" }` (prepare: open Manage connections, Edit Logistics, Test connection, Delete…).

- [ ] **Step 5: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 6: Finish review**

Run the Finish review procedure for: add sheet (both methods), device-code prompt, signed-in state, edit sheet with stored secret, test results, delete confirmation, connections list, label swatches.

- [ ] **Step 7: Commit**

```bash
git add app/src app/e2e
git commit -m "Shell: connection sheets with Entra sign-in, test and delete (finish review: <verdict>)"
```

---

### Task 19: Settings sheet and the update prompt

**Files:**
- Create: `app/src/engine/updater.ts`, `app/src/shell/sheets/SettingsSheet.tsx`, `app/src/shell/sheets/UpdateSheet.tsx`, `app/src/shell/sheets/useUpdate.ts`
- Modify: `app/eslint.config.js` (allow `src/engine/updater.ts` to import Tauri), `app/src/test/boundaries.test.ts`, `app/src/engine/index.ts`, `app/src/engine/mock/transport.ts` (`?update=1`), `app/src/shell/sheets/SheetHost.tsx`, `app/src/shell/StatusBar.tsx`, `app/e2e/shots.spec.ts`
- Test: `app/src/shell/sheets/settings.test.tsx`

**Interfaces:**
- Consumes: `@tauri-apps/plugin-updater` (`check`, `Update`), `@tauri-apps/plugin-process` (`relaunch`); `Segmented`, `Sheet`, `Button`, `EmptyState` (ui); `useShell`, `useAppInfo` (shell, engine).
- Produces:
  - engine: `UpdateInfo { version: string; notes: string | null; date: string | null }`, `Updater { check(): Promise<UpdateInfo | null>; install(onProgress: (pct: number | null) => void): Promise<void> }`, `setUpdater(u)`, `checkForUpdate()`, `installUpdate(onProgress)`
  - shell: `useUpdate()` → TanStack query `["update"]` that first runs 3 s after start, then only on demand; `SettingsSheet()`, `UpdateSheet()`

- [ ] **Step 1: Write the failing tests**

Add to `app/src/test/boundaries.test.ts` inside "only engine/ipc.ts imports Tauri":

```ts
    expect(await restricted("src/engine/updater.ts", `import { check } from "@tauri-apps/plugin-updater";\nexport const b = check;`)).toHaveLength(0);
```

`app/src/shell/sheets/settings.test.tsx`:

```tsx
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { setUpdater } from "@/engine";
import { renderShell } from "../test-utils";

describe("SettingsSheet", () => {
  it("changes theme and density and saves them", async () => {
    const { mock } = renderShell("default");
    await screen.findByRole("list", { name: "Saved connections" });
    await userEvent.keyboard("{Control>},{/Control}");
    const sheet = await screen.findByRole("dialog", { name: "Settings" });
    await userEvent.click(within(sheet).getByRole("radio", { name: "Light" }));
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("light"));
    await userEvent.click(within(sheet).getByRole("radio", { name: "Comfortable" }));
    await waitFor(() => expect(document.documentElement.style.getPropertyValue("--row")).toBe("26px"));
    await userEvent.click(within(sheet).getByRole("radio", { name: "60 s" }));
    expect(mock.calls).toContainEqual({ cmd: "settings_set", args: { key: "ui", value: { theme: "light", density: "comfortable", autoRefresh: 60 } } });
    expect(within(sheet).getByText("0.0.1-mock")).toBeInTheDocument();
  });
});

describe("update prompt", () => {
  it("announces an update in the status bar and installs it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const install = vi.fn(async (cb: (p: number | null) => void) => cb(100));
    setUpdater({ check: async () => ({ version: "0.1.0", notes: "Faster tree loading.", date: "2026-10-20" }), install });
    renderShell("default");
    vi.advanceTimersByTime(3500);
    const notice = await screen.findByRole("button", { name: "Update 0.1.0 available" });
    vi.useRealTimers();
    await userEvent.click(notice);
    const sheet = await screen.findByRole("dialog", { name: "Update available" });
    expect(within(sheet).getByText("Faster tree loading.")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    expect(install).toHaveBeenCalledOnce();
  });
});
```

(Radix ToggleGroup items have role `radio` when `type="single"`. If they report `button` with `aria-pressed`, query `getByRole("button", { name: "Light" })` instead; keep the test asserting the observable theme change.)

Run: `npm test -- src/shell/sheets/settings src/test/boundaries`
Expected: FAIL.

- [ ] **Step 2: Implement the updater module and lint exception**

`app/eslint.config.js`: change the ipc exception to `{ files: ["src/engine/ipc.ts", "src/engine/updater.ts"], rules: { "no-restricted-imports": "off" } }` and update the `tauri` message to "Only src/engine/ipc.ts and src/engine/updater.ts talk to Tauri."

`app/src/engine/updater.ts`:

```ts
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { inTauri } from "./ipc";

export interface UpdateInfo { version: string; notes: string | null; date: string | null }
export interface Updater {
  check(): Promise<UpdateInfo | null>;
  install(onProgress: (pct: number | null) => void): Promise<void>;
}

let pending: Update | null = null;

const tauriUpdater: Updater = {
  async check() {
    if (!inTauri()) return null;
    pending = await check();
    return pending ? { version: pending.version, notes: pending.body ?? null, date: pending.date ?? null } : null;
  },
  async install(onProgress) {
    if (!pending) return;
    let total = 0;
    let got = 0;
    await pending.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? 0;
      else if (e.event === "Progress") {
        got += e.data.chunkLength;
        onProgress(total ? Math.round((got / total) * 100) : null);
      }
    });
    await relaunch();
  },
};

let impl: Updater = tauriUpdater;

/** Swap the updater (tests and mock builds). */
export function setUpdater(u: Updater): void {
  impl = u;
}
export const checkForUpdate = () => impl.check();
export const installUpdate = (onProgress: (pct: number | null) => void) => impl.install(onProgress);
```

Export from `engine/index.ts`: `export * from "./updater";`. In `installMock()`, when `params.get("update") === "1"`:

```ts
  if (params.get("update") === "1") {
    setUpdater({
      check: async () => ({ version: "0.1.0", notes: "Faster tree loading.\nThe PROD badge now shows in every theme.", date: "2026-10-20" }),
      install: async (cb) => {
        for (const p of [20, 60, 100]) {
          await wait(150);
          cb(p);
        }
      },
    });
  }
```

(import `setUpdater` from `../updater`).

- [ ] **Step 3: Implement the hook and sheets**

`app/src/shell/sheets/useUpdate.ts`:

```ts
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { checkForUpdate } from "@/engine";

/** Checks once, 3 s after start; `refetch()` checks again on demand. */
export function useUpdate() {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setArmed(true), 3000);
    return () => clearTimeout(t);
  }, []);
  return useQuery({ queryKey: ["update"], queryFn: checkForUpdate, enabled: armed, staleTime: Infinity, retry: false });
}
```

`app/src/shell/sheets/SettingsSheet.tsx`:

```tsx
import { useAppInfo } from "@/engine";
import { Button, Segmented, Sheet } from "@/ui";
import { useShell } from "../context";
import type { AutoRefresh } from "../state";

function Row({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4 py-2">
      <div className="flex flex-col">
        <span className="text-12 font-semibold">{label}</span>
        {hint && <span className="text-11 text-muted">{hint}</span>}
      </div>
      {children}
    </div>
  );
}

export function SettingsSheet() {
  const { prefs, setPrefs, closeSheet, openSheet } = useShell();
  const info = useAppInfo().data;
  return (
    <Sheet open onOpenChange={(o) => !o && closeSheet()} title="Settings" width={440}>
      <div className="flex flex-col divide-y divide-line">
        <Row label="Theme">
          <Segmented label="Theme" value={prefs.theme} onChange={(theme) => setPrefs({ theme })} options={[{ value: "system", label: "System" }, { value: "dark", label: "Dark" }, { value: "light", label: "Light" }]} />
        </Row>
        <Row label="Density" hint="Row height in the tree and grids.">
          <Segmented label="Density" value={prefs.density} onChange={(density) => setPrefs({ density })} options={[{ value: "compact", label: "Compact" }, { value: "comfortable", label: "Comfortable" }]} />
        </Row>
        <Row label="Auto-refresh" hint="Refreshes counts for what is on screen.">
          <Segmented
            label="Auto-refresh"
            value={String(prefs.autoRefresh)}
            onChange={(v) => setPrefs({ autoRefresh: Number(v) as AutoRefresh })}
            options={[{ value: "0", label: "Off" }, { value: "30", label: "30 s" }, { value: "60", label: "60 s" }]}
          />
        </Row>
        <Row label="Version">
          <div className="flex items-center gap-3">
            <span className="font-mono text-11 text-muted">{info?.version}</span>
            <Button size="sm" onClick={() => openSheet({ kind: "update" })}>Check for updates</Button>
          </div>
        </Row>
      </div>
    </Sheet>
  );
}
```

`app/src/shell/sheets/UpdateSheet.tsx`:

```tsx
import { useState } from "react";
import { installUpdate, type EngineError } from "@/engine";
import { Button, EmptyState, Sheet } from "@/ui";
import { useShell } from "../context";
import { useUpdate } from "./useUpdate";

export function UpdateSheet() {
  const { closeSheet } = useShell();
  const update = useUpdate();
  const [progress, setProgress] = useState<number | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const u = update.data;
  const installing = progress !== undefined;

  async function install() {
    setError(null);
    setProgress(null);
    try {
      await installUpdate(setProgress);
    } catch (e) {
      setProgress(undefined);
      setError((e as EngineError).message ?? String(e));
    }
  }

  const title = update.isFetching ? "Checking for updates" : u ? "Update available" : "Dispatched is up to date";
  return (
    <Sheet
      open
      onOpenChange={(o) => !o && !installing && closeSheet()}
      title={title}
      footer={
        u ? (
          <>
            <Button variant="quiet" onClick={closeSheet} disabled={installing}>Later</Button>
            <Button variant="primary" onClick={() => void install()} disabled={installing}>
              {installing ? (progress == null ? "Downloading…" : `Downloading ${progress}%`) : "Install and restart"}
            </Button>
          </>
        ) : (
          <Button onClick={() => void update.refetch()} disabled={update.isFetching}>Check again</Button>
        )
      }
    >
      {update.error ? (
        <EmptyState tone="error" title="Couldn't check for updates" body={(update.error as Error).message} />
      ) : u ? (
        <div className="flex flex-col gap-2">
          <p className="m-0 text-13"><span className="font-semibold">Version {u.version}</span>{u.date && <span className="text-muted">, released {u.date}</span>}</p>
          {u.notes && <p className="m-0 whitespace-pre-wrap text-12">{u.notes}</p>}
          {error && <p className="m-0 text-12">{error}</p>}
        </div>
      ) : (
        !update.isFetching && <p className="m-0 text-12 text-muted">You have the latest version.</p>
      )}
    </Sheet>
  );
}
```

Opening the update sheet from the palette or Settings must check now: in `UpdateSheet`, add `useEffect(() => { void update.refetch(); }, [])` (with an eslint-disable for the dependency list) so it does not wait for the 3 s start delay.

`SheetHost`: add `case "settings": return <SettingsSheet />;` and `case "update": return <UpdateSheet />;`.

`StatusBar.tsx`: call `useUpdate()`; when `data` is set, show on the right before the refresh time: `<Button size="sm" variant="quiet" onClick={() => openSheet({ kind: "update" })}>Update {data.version} available</Button>`.

Append to `shots`: `{ name: "settings", path: "/?scenario=default&autoconnect=p-logistics" }` (prepare: `ControlOrMeta+,`) and `{ name: "update", path: "/?scenario=default&autoconnect=p-logistics&update=1" }` (prepare: wait 3.5 s, click the update notice).

- [ ] **Step 4: Run tests**

Run: `npm test && npm run typecheck && npm run lint`
Expected: PASS.

- [ ] **Step 5: Finish review**

Run the Finish review procedure for: settings sheet in both themes and densities, status bar update notice, update sheet (available, downloading, up to date).

- [ ] **Step 6: Commit**

```bash
git add app/src app/e2e app/eslint.config.js
git commit -m "Shell: settings sheet and update prompt (finish review: <verdict>)"
```

---
### Task 20: End-to-end shell flows and CI

**Files:**
- Create: `app/e2e/shell.spec.ts`
- Modify: `.github/workflows/ci.yml` (new `frontend` job)

**Interfaces:**
- Consumes: the mock build (`npm run build:mock`), scenarios and `autoconnect` from Task 12, every screen from Tasks 13–19.
- Produces: a CI job named `frontend` that runs lint, typecheck, unit tests and Playwright (Chromium and WebKit).

- [ ] **Step 1: Write the flows**

`app/e2e/shell.spec.ts`:

```ts
import { expect, test, type Page } from "@playwright/test";

const go = (page: Page, query: string) => page.goto(`/?${query}`);
const tree = (page: Page) => page.getByRole("tree", { name: "Namespace" });

test("browse: counts in the tree, open a subscription, read its views", async ({ page }) => {
  await go(page, "scenario=default&autoconnect=p-logistics");
  const t = tree(page);
  const invoice = t.getByRole("treeitem", { name: "invoice-generate" });
  await expect(invoice).toBeVisible();
  await expect(invoice.getByText("37")).toHaveClass(/text-accent/);
  await t.getByRole("treeitem", { name: "shipment-events" }).click();
  await page.keyboard.press("ArrowRight");
  await t.getByRole("treeitem", { name: "tracking-sync" }).click();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { level: 1, name: "tracking-sync" })).toBeVisible();
  await expect(page.getByRole("tablist", { name: "Entity views" }).getByText("3,417")).toBeVisible();
  await expect(page.getByRole("banner").getByText("PROD")).toBeVisible();
});

test("palette opens an entity", async ({ page }) => {
  await go(page, "scenario=default&autoconnect=p-logistics");
  await expect(tree(page)).toBeVisible();
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByPlaceholder("Go to an entity or run a command").fill("label-print");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true");
});

test("first connection from an empty start", async ({ page }) => {
  await go(page, "scenario=empty");
  await page.getByRole("button", { name: "Add connection string" }).click();
  const sheet = page.getByRole("dialog", { name: "Add connection" });
  await sheet.getByLabel("Name").fill("Orders");
  await sheet.getByLabel("Connection string").fill("Endpoint=sb://orders.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0");
  await sheet.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("button", { name: /Connection: Orders/ })).toBeVisible();
  await expect(page.getByText("c2VjcmV0")).toHaveCount(0);
});

test("limited listing explains itself and shows pinned entities", async ({ page }) => {
  await go(page, "scenario=forbidden&autoconnect=p-logistics");
  await expect(page.getByText(/can't list the namespace/)).toBeVisible();
  await expect(tree(page).getByRole("treeitem", { name: "orders" })).toBeVisible();
});

test("missing secret shows the fix", async ({ page }) => {
  await go(page, "scenario=missing-secret&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Couldn't connect to Logistics" })).toBeVisible();
  await expect(page.getByText(/no stored connection string/)).toBeVisible();
  await page.getByRole("button", { name: "Edit connection" }).click();
  await expect(page.getByRole("dialog", { name: "Edit Logistics" })).toBeVisible();
});

test("listing errors offer a retry and land in the activity log", async ({ page }) => {
  await go(page, "scenario=network&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Can't list this namespace" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Jobs and activity" }).getByText(/connection reset by peer/)).toBeVisible();
});

test("theme follows settings", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await go(page, "scenario=default&autoconnect=p-logistics");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.keyboard.press("ControlOrMeta+,");
  await page.getByRole("dialog", { name: "Settings" }).getByText("Light", { exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("2,000 queues stay navigable from the keyboard", async ({ page }) => {
  await go(page, "scenario=many&autoconnect=p-logistics");
  const t = tree(page);
  await t.getByRole("treeitem", { name: "queue-0000" }).click();
  await page.keyboard.press("End");
  await expect(t.getByRole("treeitem", { name: "stock-levels" })).toBeVisible();
  expect(await t.getByRole("treeitem").count()).toBeLessThan(120);
});

test("drawer and tabs follow the keyboard", async ({ page }) => {
  await go(page, "scenario=default&autoconnect=p-logistics");
  await tree(page).getByRole("treeitem", { name: "label-print" }).dblclick();
  await expect(page.getByRole("tab", { name: /label-print/ })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+j");
  await expect(page.getByRole("region", { name: "Jobs and activity" })).toHaveCount(0);
  await page.getByRole("tree", { name: "Namespace" }).focus();
  await page.keyboard.press("ControlOrMeta+w");
  await expect(page.getByRole("tab", { name: /label-print/ })).toHaveCount(0);
});
```

(Browsers reserve ⌘W/Ctrl+W for closing the page and may not deliver it to the page; if the last two lines fail only for that reason in Playwright, replace the key press with clicking "Close label-print" and keep ⌘W for the Task 21 live check in the real window.)

- [ ] **Step 2: Run them**

Run (disk check first): `cd app && df -h /System/Volumes/Data && npm run e2e`
Expected: PASS in Chromium and WebKit.

- [ ] **Step 3: Add the CI job**

Append to `.github/workflows/ci.yml` under `jobs:`:

```yaml
  frontend:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: dispatched/app
    steps:
      - uses: actions/checkout@v4
        with: { path: dispatched }
      - uses: actions/setup-node@v4
        with: { node-version: 22, cache: npm, cache-dependency-path: dispatched/app/package-lock.json }
      - run: npm ci
      - run: npm run lint
      - run: npm run typecheck
      - run: npm test
      - run: npx playwright install --with-deps chromium webkit
      - run: npm run e2e
        env: { CI: "1" }
      - if: failure()
        uses: actions/upload-artifact@v4
        with: { name: playwright-report, path: dispatched/app/playwright-report, retention-days: 7 }
```

(The existing `app` job keeps building the frontend before `cargo clippy -p dispatched-app`.)

- [ ] **Step 4: Commit**

```bash
git add app/e2e .github/workflows/ci.yml
git commit -m "CI: frontend job with lint, unit tests and Playwright shell flows"
```

---

### Task 21: Live verification, DESIGN.md, cleanup and the PR

**Files:**
- Create: `docs/live-verification-m1b.md`, `app/DESIGN.md` (impeccable documenter)
- Modify: memory (`dispatched-service-bus-explorer.md`), the Notion page log (with the user's yes)

This task needs the user at three points: approving the real-namespace check, any Azure sign-in prompt, and pushing/opening the PR.

- [ ] **Step 1: Full local check**

```bash
cd /Users/hamzaabdagic/Development/Horizon-Tech-doo/dispatched && df -h /System/Volumes/Data
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test
(cd app && npm run lint && npm run typecheck && npm test && npm run e2e)
```

Expected: everything PASSES.

- [ ] **Step 2: Emulator in the real window**

```bash
testing/emulator/emulator.sh up
cd app && npm run tauri dev
```

In the app: add a connection with the emulator connection string from `testing/emulator/` (as the M1a emulator tests use it), connect, check the tree (queues/topics/subscriptions, `~N` approximate counts), open tabs, ⌘K, ⌘J, ⌘W (note whether the macOS menu takes ⌘W; if it closes the window, add a custom app menu without the Close Window accelerator in `lib.rs` and re-check), F5, theme switch, density switch, restart the app and confirm tabs and layout come back. Then `testing/emulator/emulator.sh down` and remove the emulator images this session pulled (`docker image rm` the two images; never `docker system prune`).

- [ ] **Step 3: Many entities and cold start**

- Run `VITE_DISPATCHED_MOCK=1 npm run tauri dev` with the app's URL query set to `?scenario=many&autoconnect=p-logistics` (open the devtools console and run `location.search = "?scenario=many&autoconnect=p-logistics"`), scroll the tree fast and hold ↓: no visible stutter. Record the result.
- Build a release app once (`npm run tauri build`, disk check first), launch it from Finder three times and time window-open to interactive shell (tree visible after reconnecting to the last profile on the emulator or the welcome screen) with a stopwatch; target under 1.5 s. Record the three numbers and the machine. Delete the release bundle output under `target/release/bundle/` afterwards.

- [ ] **Step 4: Real namespace, read-only (ask the user first)**

Ask the user: "Can I sign in to <tenant> in the app and browse <namespace> read-only (listing and counts only)?" With a yes:
- Entra profile: sign in from the profile sheet (browser flow), add the namespace, connect, browse; mark it production and confirm PROD shows everywhere.
- SAS profile: the user pastes the connection string into the sheet themselves (never through chat or a command line); confirm the edit sheet shows "Stored in the system keychain." and nothing else.
- A Listen-only or entity-scoped key if one exists: confirm the limited banner and pinned entities.
- Nothing is sent, received, moved or deleted.

- [ ] **Step 5: Write `docs/live-verification-m1b.md`**

Same shape as `docs/live-verification-m1a.md`: date, machine, what was checked (emulator, real namespace with Entra and SAS, limited identity, many entities, cold start numbers, ⌘W result), what was not checked and why, follow-ups.

- [ ] **Step 6: Final design review and DESIGN.md**

- Run `npm run shots` and dispatch `impeccable-finish-reviewer` once more over every shot together, with the direction contract and the mockup PNGs, asking specifically about consistency across screens. Fix material findings.
- Dispatch the `impeccable-documenter` agent to write `app/DESIGN.md` from the built result (tokens in `src/ui/tokens.ts`, `styles.css`, components, catalogue screenshots). It is the starting point for `@horizon-tech/explorer-ui` after M1.

- [ ] **Step 7: Clean up**

```bash
rm -rf app/dist-mock app/playwright-report app/test-results
git status --short
```

Ask the user whether to keep the Playwright browsers in `~/Library/Caches/ms-playwright` (about 500 MB; useful for future UI work) or remove them with `npx playwright uninstall --all`.

- [ ] **Step 8: Commit, then ask before pushing**

```bash
git add docs/live-verification-m1b.md app/DESIGN.md
git commit -m "M1b: live verification and DESIGN.md"
```

Ask the user for a yes, then push `m1b` and open the PR "M1b: design system and app shell" (body: what shipped, test evidence, live verification summary, follow-ups; no attribution lines). Merge only on green CI and the user's yes. Update the memory file and add an M1b entry to the Notion page log, as was done for M1a.

---

## Self-review notes (for the executor)

- Spec §2 scope → Tasks 7–19 (design system 7–11, shell 13–17, profiles 18, settings/update 19, catalogue 8–11).
- Spec §5 tokens and contrast test → Task 7 (values may move to pass the test; the test is the authority).
- Spec §6.3 profiles, keychain, Entra → Tasks 2, 3, 4, 18. §6.4 tree → 14. §6.5 tabs → 15. §6.6 palette/keys → 16. §6.7 drawer/status bar → 13, 17. §6.8 settings/updates → 19.
- Spec §7 commands → Tasks 3–4. §8 testing → every task, plus 20. §9 design process → Task 6, every UI task's finish review, Task 21. §10 performance → Tasks 10, 20, 21. §11 done → Task 21.
