# M1b — Design system and app shell

**Status:** Draft for review · **Date:** 2026-10-15 · **Owner:** Hamza Abdagić (Horizon Tech d.o.o.)
**Parent spec:** [Dispatched v1 design](2026-10-13-dispatched-v1-design.md), section 6 (Desktop app) and the M1 row of the milestone table.
**Visual reference:** the mockup canvas "Dispatched visual directions" (private claude.ai artifact, https://claude.ai/artifact/85qsVo8NmMQagPRXus4MwX), artboard **A. Workbench**.

## 1. Intent

M1 is split in three. M1a (core and CLI browse) shipped on 2026-10-14. M1b sets the desktop app's visual system and its shell, wired to real data. M1c then fills the entity tabs with the browse features (message grid, peek, detail pane, filters, export, dashboard).

**What the user said:**
- Arkived's UI is "too AI slop". ServiceBusExplorer is the feature reference only ("we dont need to follow ServiceBusExplorers lead on it").
- The desired feel is **IDE workbench** plus **native DB client** (picked from: native DB client, IDE workbench, crisp minimal, ops console).
- Keep the shadcn stack and the other packages used around Arkived (Tailwind 4, lucide-react, motion, clsx, tailwind-merge).
- Chose direction **A. Workbench** from three mockups (A Workbench, B Ledger, C Split).
- Chose to build the design system inside the app now and extract it after M1 (option 1 of 3).

**Success:** the user opens the app against a real namespace and it looks and behaves like mockup A: dense, quiet, keyboard-first, amber only where attention is needed. Every screen has passed an impeccable finish review.

## 2. Scope

**In M1b:**
1. **Design system** in `app/src/ui/`: tokens (dark, light, follow OS; compact and comfortable density), bundled JetBrains Mono, entity glyphs, restyled shadcn components, a virtualised Tree and a virtualised DataGrid.
2. **Shell on real data**: top bar, namespace tree, centre tabs with sub-tab rows, Jobs/Activity drawer, status bar, command palette, settings sheet, update prompt.
3. **Profile management** in side sheets: add, edit and remove connection-string and Entra profiles; mark production; colour label; test connection.
4. **Component catalogue**: a dev-only screen with every component in both themes and both densities, used by the impeccable reviews.

**Left for M1c:** everything inside an entity's sub-tabs (message grid with peek, detail pane, property and body filters, export, Overview properties and live counts, Sessions, Rules), the Dashboard tab's content, and concurrent Entra sign-ins across tenants (including MFA guest tenants). Until M1c, each sub-tab shows a designed empty state that says what will appear there.

**Out of scope for M1:** anything that changes data (receive, resubmit, move, delete, compose, entity create and edit) is M2 or later, per the parent spec.

## 3. Stack

| Concern | Choice |
|---|---|
| Framework | React 19, TypeScript, Vite 7 (already in `app/`) |
| Styling | Tailwind CSS 4 with `@theme` mapped to the colour roles; `clsx` + `tailwind-merge` via `cn()` |
| Components | shadcn/ui on Radix, added with the shadcn CLI into `app/src/ui/components/` and restyled; none ship in their default look |
| Grid and tree | TanStack Table and TanStack Virtual |
| Palette | cmdk (through shadcn's Command) |
| Server state | TanStack Query (caching, refresh, loading and error states) |
| Icons | lucide-react for general UI; own SVG glyphs for entities |
| Motion | `motion`, only for side sheets, the drawer and popovers |
| Fonts | OS UI font; JetBrains Mono (OFL) bundled as woff2, no network fonts |
| Tests | Vitest + Testing Library; Playwright against the Vite build with Tauri IPC mocked |

## 4. Code layout and boundaries

- `app/src/ui/` — the design system: `tokens.css`, `theme.ts` (theme and density switching), `glyphs/`, `components/`, `lib/cn.ts`. **It imports nothing from the rest of the app.** An ESLint `no-restricted-imports` rule enforces this, so extracting it into `@horizon-tech/explorer-ui` after M1 is a copy-and-publish job.
- `app/src/shell/` — layout, top bar, tree, tabs, palette, drawer, status bar, sheets.
- `app/src/engine/` — typed wrappers for the Tauri commands and the TanStack Query hooks. The only place that calls `invoke` or listens to Tauri events.
- `app/src/dev/` — the component catalogue, excluded from production builds.
- `app/src-tauri/src/commands/` — thin command wrappers over `dispatched-core`. No business logic in the app crate.
- `crates/dispatched-core` gains a settings table in `dispatched.db` (section 7).

## 5. Design system

### 5.1 Colour

Components use named roles only, never raw hex values.

| Role | Dark | Light | Use |
|---|---|---|---|
| `chrome` | `#1d1f23` | `#eef0f3` | top bar, tab strip, status bar |
| `side` | `#191b1e` | `#f6f7f9` | tree, detail pane, drawer |
| `content` | `#141518` | `#ffffff` | main content |
| `field` | `#121316` | `#ffffff` | inputs |
| `line` | `#2a2d32` | `#dde0e5` | 1px separators and control borders |
| `fg` | `#d9dce1` | `#1d2127` | primary text, focus ring |
| `muted` | `#8d929b` | `#59606b` | secondary text, labels |
| `faint` | `#5f646c` | `#8a919b` | tertiary text (not for anything that must be read) |
| `icon` | `#9aa0a9` | `#59606b` | glyphs |
| `hover` | `#1c1e22` | `#f3f5f8` | row hover |
| `selection` | `#262c36` | `#e2e8f1` | selected row, tree node, tab |
| `accent` | `#f0a537` | `#9e5800` | dead-letter counts, PROD, errors |
| `accent-ink` | `#1d1406` | `#ffffff` | text on an accent fill |
| `accent-tint` | `rgba(240,165,55,.14)` | `#fbf0e2` | background behind accent text where needed |

Rules:
- **Amber marks attention only:** dead-letter counts above zero, PROD, errors. Errors also carry an icon and text, so colour is never the only signal.
- **Selection is neutral** (`selection`), never amber.
- **Keyboard focus** is a 1px `fg` outline (inset in grids and trees), visible in both themes.
- **Success has no colour.**
- `faint` is for decoration-level text only (separators, the third line of metadata). Anything a user must read is `muted` or stronger.
- A unit test computes WCAG contrast for every text-on-surface pair the components use: at least 4.5:1, or 3:1 for text 18px and larger. Values above may be adjusted to pass; the test is the authority.

### 5.2 Type

- UI font: `-apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif`.
- Scale: 11, 12, 13, 14px; base 12px with 16px line height. Weights 400 and 600 only.
- Monospace: JetBrains Mono 400 and 500, bundled, `font-variant-ligatures: none`. Used for IDs, sequence numbers, timestamps in grids, bodies, filter expressions and the activity log. Never as decoration.
- Tabular numerals (`font-variant-numeric: tabular-nums`) on every count and number column.

### 5.3 Space, shape, depth

- 4px grid.
- Row height is the `--row` token: 22px compact (default), 26px comfortable. Trees, grids and lists use it.
- Corner radius 3px on controls; panels and the window layout are square.
- No shadows on surfaces. One shadow token (offset and blur, tinted to the theme) for floating layers: popovers, menus, the palette, side sheets.
- No gradients, glows, glass, decorative cards, pills everywhere, eyebrows, status dots without meaning, or emoji.

### 5.4 Motion

Only side sheets, the drawer and popovers animate: 150ms, exponential ease-out. With `prefers-reduced-motion`, nothing animates. Loading never animates layout; counts arriving cause no layout shift (fixed-width, right-aligned number columns).

### 5.5 Icons and glyphs

- Entity glyphs (namespace, queue, topic, subscription, rule) as inline SVG components on a 16px grid, 1.3 stroke, `currentColor`. The four drawn in mockup A are the starting point; rule is new.
- General icons from lucide-react at 14px, stroke 1.5.

### 5.6 Components

Restyled from shadcn/Radix unless noted: Button (default, quiet, destructive-confirm), IconButton, Input, Select, Tabs (entity tabs and sub-tabs), Tooltip, DropdownMenu, ContextMenu, Sheet (side sheet, right edge), Command (palette), Badge (count, PROD), Kbd, Skeleton, EmptyState (own), Resizable panels, **Tree** (own, TanStack Virtual, roving focus, type-ahead) and **DataGrid** (own, TanStack Table + Virtual; built and tested in M1b, first used with data in M1c).

Every interactive component has default, hover, focus, active and disabled states; data components also have loading (skeleton rows in the real shape), empty and error states.

### 5.7 Window chrome

macOS: `titleBarStyle: "Overlay"` with the top bar inside the title bar (left inset for the window buttons, drag region on empty top-bar space). Windows and Linux: native title bar, the app's top bar below it.

## 6. Shell

### 6.1 Layout

As in mockup A: top bar (38px), then a row of tree | centre; centre holds the entity tab strip, the entity header, the sub-tab row and the content area; the Jobs/Activity drawer sits at the bottom of the centre; a 22px status bar closes the window. The tree and drawer are resizable and collapsible; sizes persist.

### 6.2 Top bar

Connection selector (profile name, host, PROD badge for production profiles, colour label), search field that opens the palette (shows ⌘K), refresh. The PROD badge is visible on every screen while a production profile is connected.

### 6.3 Connections and profiles

- One active connection at a time. Choosing another profile closes the current connection and opens the new one.
- Profile sheet (side sheet): name, auth kind (connection string or Entra), namespace host, tenant (Entra), production flag, colour label, transport (AMQP or AMQP over WebSockets), proxy, pinned entities. *Test connection* runs the core's diagnostics and shows each check inline.
- Connection strings pass from the sheet to one command, go straight to the OS keychain, and never return to the UI. Editing a profile shows "Stored in keychain" with *Replace*.
- Entra sign-in uses the kit's `login_browser` flow in the default browser, with `login_device_code` as a fallback link in the sheet (for locked-down machines); the sheet shows progress and the signed-in account.
- Deleting a profile asks for confirmation in the sheet (no modal) and removes its keychain secret when it has one.

### 6.4 Namespace tree

- Lazy: connecting lists queues and topics with counts; expanding a topic lists its subscriptions.
- Counts from the management API: active and dead-letter shown; dead-letter in amber when above zero.
- Filter field (⌘F) narrows by name across loaded entities.
- F5 refreshes; optional auto-refresh (off, 30s, 60s; default off) refreshes only visible nodes.
- Errors stay in the failing node, in plain words with the fix (for example "Can't list subscriptions: this identity needs Listen or Manage on shipment-events").
- When the identity cannot list the namespace, the tree shows the profile's pinned entities.
- Context menu with the same actions as the palette (open, open in new tab, copy name, copy entity path, refresh).

### 6.5 Tabs

- Opening an entity adds or focuses its tab. Sub-tabs follow the parent spec: Overview, Messages, Dead-letter, Transfer DLQ, Scheduled, Deferred, Sessions, Rules (subscriptions only), Listener. Counts appear next to sub-tabs that have them.
- Open tabs, the selected sub-tab per tab, panel sizes and drawer state are saved per profile and restored on restart.
- In M1b each sub-tab shows its empty state; M1c fills them.

### 6.6 Palette and keyboard

Palette commands: go to entity (fuzzy over everything loaded), switch connection, add profile, refresh, toggle drawer, theme, density, settings, check for updates. Each shows its shortcut.

| Keys (Ctrl on Windows/Linux) | Action |
|---|---|
| ⌘K | Palette |
| ⌘F | Filter the tree |
| j/k, arrows, Home/End | Move in tree and lists |
| Enter | Open the selected entity |
| ⌘1 … ⌘9 | Switch tabs |
| ⌘W | Close tab |
| ⌘J | Toggle drawer |
| ⌘, | Settings |
| F5 | Refresh |

### 6.7 Drawer and status bar

- **Activity:** every engine call, as `time  action  duration`, plus errors with their reason. The Rust layer emits these as Tauri events. Entries never contain secrets, connection strings, tokens or message bodies. Kept in memory, capped at 1,000 entries.
- **Jobs:** list with progress and cancel, wired to an event channel. Empty in M1b (its empty state says exports and bulk operations appear here); M1c's export is the first producer.
- **Status bar:** transport, auth kind, connection state, last refresh time. A lost connection shows here and on the connection selector with *Reconnect*; the tree keeps its last data, marked stale.

### 6.8 Settings and updates

Settings sheet: theme (system, dark, light), density, tree auto-refresh. The update prompt uses the Tauri updater and the signing key set up in M0: a quiet notice in the status bar, then a sheet with release notes and *Install and restart*.

## 7. Rust side

New Tauri commands, each a thin wrapper over `dispatched-core`:

| Area | Commands |
|---|---|
| Profiles | `profiles_list`, `profile_save`, `profile_delete`, `profile_test` |
| Sign-in | `sign_in_start`, `sign_in_status` |
| Connection | `connect(profile_id)`, `disconnect` |
| Tree | `list_entities` (queues and topics with counts), `list_subscriptions(topic)` |
| Settings | `settings_get`, `settings_set` |

- `dispatched-core` gains a `settings` table (key, JSON value) in `dispatched.db` via a migration, with get and set by key. Shell state is stored per profile id.
- Errors cross the boundary as the existing `CommandError { kind, message }`.
- Activity and job events use two Tauri event channels with typed payloads mirrored in `app/src/engine/`.
- The existing M0 commands (`connect_connection_string`, `list_queues`, `peek`) are replaced; `peek` stays available for M1c.

## 8. Testing

- **Rust:** command tests, including that `profiles_list` and `profile_save` responses never contain a secret; error-kind mapping; settings store against an in-memory DB; migration from the M1a schema.
- **Frontend unit (Vitest, Testing Library):** keyboard behaviour of Tree, Tabs and palette; theme and density switching; the token contrast test; the `ui/` import-boundary lint as a CI step.
- **Shell flows (Playwright):** against the Vite build with Tauri IPC mocked: connect, browse, open tabs, palette, profile sheets, error and empty states, both themes. Runs in CI.
- **Live check** recorded in `docs/live-verification-m1b.md`: the emulator; one real namespace read-only with a SAS profile and an Entra profile; a namespace with many entities to confirm the tree stays smooth.

## 9. Design process

- Before building: impeccable setup writes `PRODUCT.md` (product context) and records the direction contract for direction A in the shell's surface brief.
- Every UI task runs the impeccable and taste skills. A screen is done only after an impeccable finish review passes.
- At the end, the impeccable documenter writes `DESIGN.md` from the built result. The `@horizon-tech/explorer-ui` extraction after M1 starts from it.

## 10. Performance targets

- The tree stays smooth (no dropped frames while scrolling) with 2,000 entities.
- Cold start to an interactive shell in under 1.5 s on an Apple Silicon Mac (release build).
- No layout shift when counts arrive.

## 11. Done when

- Sections 2 to 10 are met and CI is green.
- `docs/live-verification-m1b.md` is recorded.
- The user opens the app against a real namespace and it matches mockup A.
