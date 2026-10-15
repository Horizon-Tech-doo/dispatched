---
name: Dispatched
description: A quiet IDE-style workbench for Azure Service Bus where colour appears only when something needs attention.
colors:
  dark-chrome: "#1d1f23"
  dark-side: "#191b1e"
  dark-content: "#141518"
  dark-field: "#121316"
  dark-line: "#2a2d32"
  dark-fg: "#d9dce1"
  dark-muted: "#959aa3"
  dark-faint: "#727780"
  dark-icon: "#9aa0a9"
  dark-hover: "#1c1e22"
  dark-selection: "#262c36"
  dark-accent: "#f0a537"
  dark-accent-ink: "#1d1406"
  dark-accent-tint: "#372e22"
  light-chrome: "#eef0f3"
  light-side: "#f6f7f9"
  light-content: "#ffffff"
  light-field: "#ffffff"
  light-line: "#dde0e5"
  light-fg: "#1d2127"
  light-muted: "#59606b"
  light-faint: "#767d86"
  light-icon: "#59606b"
  light-hover: "#f3f5f8"
  light-selection: "#e2e8f1"
  light-accent: "#8f4f00"
  light-accent-ink: "#ffffff"
  light-accent-tint: "#fbf0e2"
  label-slate-dark: "#8a94a6"
  label-slate-light: "#5f6b7d"
  label-blue-dark: "#6f9fd8"
  label-blue-light: "#2f6db5"
  label-green-dark: "#6fb88a"
  label-green-light: "#2f7d4f"
  label-violet-dark: "#a08ad8"
  label-violet-light: "#6b4fb5"
  label-rose-dark: "#d88a9f"
  label-rose-light: "#b04a66"
typography:
  title:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: "20px"
  heading:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 600
    lineHeight: "18px"
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "16px"
  caption:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: "14px"
  data:
    fontFamily: "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: "16px"
rounded:
  ctl: "3px"
  inner: "2px"
spacing:
  row-compact: "22px"
  row-comfortable: "26px"
  topbar: "38px"
  statusbar: "22px"
  tabstrip: "30px"
  subtabs: "28px"
  drawer-tabs: "26px"
  sheet: "420px"
components:
  button-default:
    backgroundColor: "{colors.dark-field}"
    textColor: "{colors.dark-fg}"
    rounded: "{rounded.ctl}"
    height: "24px"
    padding: "0 8px"
  button-primary:
    backgroundColor: "{colors.dark-field}"
    textColor: "{colors.dark-fg}"
    rounded: "{rounded.ctl}"
    height: "24px"
    padding: "0 8px"
  button-quiet:
    textColor: "{colors.dark-fg}"
    rounded: "{rounded.ctl}"
  input:
    backgroundColor: "{colors.dark-field}"
    textColor: "{colors.dark-fg}"
    rounded: "{rounded.ctl}"
    height: "24px"
    padding: "0 8px"
  badge-prod:
    backgroundColor: "{colors.dark-accent}"
    textColor: "{colors.dark-accent-ink}"
    rounded: "{rounded.inner}"
    height: "16px"
    padding: "0 4px"
  sheet:
    backgroundColor: "{colors.dark-side}"
    textColor: "{colors.dark-fg}"
    width: "420px"
---

# Design System: Dispatched (desktop app)

Recorded from the shipped M1b build (direction A, "Workbench"). Sources: `src/ui/tokens.ts` (colour roles, authoritative; hex above mirrors it), `src/ui/styles.css`, `src/ui/theme.ts`, `src/ui/components/*`, `src/shell/**`, and the WebKit dark and light shots in `.superpowers/sdd/2026-10-15-m1b-design-system-shell/shots/fixwave/`. Frontmatter lists dark-theme component colours; light uses the same roles with the light values. Motion, shadow and focus values live in `.impeccable/design.json`.

## Overview

**Creative North Star: "The Workbench"**

The data is the interface. Graphite (dark) or paper-white (light) in three close surface tiers, 1px lines, square panels, 3px controls, 12px OS UI text with JetBrains Mono for data. Colour is withheld until something needs the operator: one amber accent, only on dead-letter counts above zero, PROD, and errors. It is not a dashboard of cards, not an Azure-portal blade, and carries none of the earlier Arkived look (gradients, glows, pills, emoji, roomy spacing).

**Key Characteristics:**
- Density first: 22px rows (26px optional), 12px body, no decoration.
- Neutral selection, neutral focus (1px `fg` outline), neutral buttons; amber is never an interaction colour.
- Errors, PROD and dead letters use amber plus an icon or word, never colour alone.
- Window never scrolls (`overflow: clip` on html/root); panels scroll internally.
- Dark and light follow the OS; no network fonts (offline).

## Colors

Fourteen roles, per theme, set as `--c-*` variables on `<html>` by `applyTheme`; only these exist as Tailwind colours (`--color-*: initial`, so palette utilities like `bg-zinc-800` do not compile).

### Neutral (surfaces, three tiers)
- **chrome** (dark #1d1f23 / light #eef0f3): top bar, tab strip, status bar.
- **side** (#191b1e / #f6f7f9): tree panel, drawer, sheets, menus, palette, tooltips (anything that frames content).
- **content** (#141518 / #ffffff): the working area, grid header.
- **field** (#121316 / #ffffff): inputs, buttons, selector, palette field (slightly inset from content in dark).
- **line** (#2a2d32 / #dde0e5): every 1px border and divider; skeleton bars; resize handles.
- **hover** (#1c1e22 / #f3f5f8), **selection** (#262c36 / #e2e8f1): row/menu/button states. Selection is a neutral tint, never amber or blue.

### Text
- **fg** (#d9dce1 / #1d2127) default and focus ring; **muted** (#959aa3 / #59606b) secondary, labels, placeholders; **faint** (#727780 / #767d86) timestamps, kinds, auth line; **icon** (#9aa0a9 / #59606b) glyphs and chevrons. Contrast floors from `tokens.ts` tests: fg, muted, accent 4.5:1 on every surface; faint and icon 3:1 and never the only carrier of must-read text.

### Accent (amber, attention only)
- **accent** (#f0a537 dark / #8f4f00 light): text and icon colour for dead-letter counts above zero and errors; fill for PROD. **accent-ink** (#1d1406 / #ffffff) text on the accent fill. **accent-tint** (#372e22 / #fbf0e2) quiet attention background (`Badge tone="attention"`).

### User colour
- Label swatches (slate, blue, green, violet, rose; per-theme values in the frontmatter) are the one user-chosen colour: an 8px (`size-2`, 2px radius) square beside a connection name, via `LabelSwatch`. Never text, surface, or border colour.

### Named Rules
**The Earned Amber Rule.** Amber appears only on: dead-letter count above zero, PROD, error icon+title (`Problem`), invalid-field border, connection-trouble alert mark. Never on a button, tab, selection, link or decoration. If nothing is wrong, a screen has no amber.
**The Two-Channel Rule.** Amber always travels with a second carrier: the `PROD` word, a `CircleAlert` icon with text, or a number.
**The Neutral Interaction Rule.** Hover, active, selected, focused and primary all resolve to neutral roles (hover, selection, fg).

## Typography

**UI face:** OS stack (`-apple-system`, Segoe UI Variable Text, Segoe UI, system-ui). **Data face:** JetBrains Mono 400/500, bundled locally, ligatures off. Only weights 400 and 600 exist (`--font-weight-*` reset).

### Hierarchy
- **Title** 14/20, 600: sheet titles, entity name (`h1`).
- **Heading** 13/18, 600: `Problem`/`EmptyState` title; 13px also for palette input and the `lg` button.
- **Body** 12/16, 400: default everywhere (rows, buttons, tabs, inputs). Labels 12/600 muted (`Field`); panel headers ("Explorer") 12/600 muted.
- **Caption** 11/14: hints, counts in status bar, menu group labels, badges (600, 0.04em tracking).
- **Data (mono)** 11/16: raw machine text: engine error `detail`, activity log, job progress/ETA, shortcuts (`Kbd`), mono inputs, grid columns flagged `mono`. Not for names in the tree or titles.
- Numbers: `tabular-nums` on every count, duration, clock and grid number column.

### Named Rules
**The Mono-Is-Data Rule.** Mono marks machine output (timestamps, durations, IDs, raw errors, shortcuts). Entity names and prose stay in the UI face.
**The Two-Weights Rule.** 400 or 600. Hierarchy comes from size step and muted/fg, not extra weights.

## Layout

IDE workbench, full-window, no page scroll. Anatomy (top to bottom, left to right):
- **Top bar** 38px, `chrome`, bottom 1px line; 3-col grid `1fr auto 1fr`. Left: connection selector (26px, bordered, label swatch + name + host + PROD + alert mark + chevron), then auth line in faint ("SAS key reader"). Centre: palette field 24px high, 380px (max 40vw), faint hint + `⌘K`. Right: refresh icon button. Regions are Tauri drag areas.
- **Tree panel** left, `side`, resizable 12-45% (about 260px default). 30px header "Explorer" (muted 600) with summary count at right; Filter input; then `Tree`.
- **Main column**: entity **tab strip** (30px, `chrome`, "strip" variant), **header** (title 14/600 + muted kind line + faint facts, `px-3.5 pt-2.5`), **sub-tab row** (28px underline tabs with `CountBadge`, `mt-1.5`, bottom line), then content.
- **Drawer** bottom of main column, `side`, resizable 8-70%, toggled; 26px underline tab row (Jobs, Activity) with hide chevron. Jobs rows: title, 3px progress bar (`fg` on `line`), mono ETA. Activity: mono 11/18, columns 64px time, message, 72px duration right-aligned.
- **Status bar** 22px, `chrome`, top line, 11px muted: transport, connection state, recovery button (18px), activity toggle, update notice, "Refreshed HH:MM:SS" right.
- **Sheets** right edge, 420px always (see Components).
- **Palette** 560px wide, 72px from top, centred.
- Resize handles: 1px `line`, 5px invisible hit area; `muted` on hover, `fg` when dragged or keyboard-active.

### Spacing and density
- Row height token `--row`: **22px compact (default), 26px comfortable** (Settings). Applied to tree, grid, menu, palette items, skeleton rows.
- Horizontal padding: 10px (`px-2.5`) grid/tree rows; 12px (`px-3`) bars, menus, banners; 14px (`px-3.5`) header, drawer; 16px (`px-4`) sheets and `EmptyState` (`py-5`).
- Gaps: 6px (`gap-1.5`) icon-to-text; 8px between buttons; 4px label-to-field. Tree indent 14px per depth.
- Control heights: icon button 24px (square); button sm 22 / md 24 / lg 28; status-bar and tree-row retry buttons 18px; input 24px; segmented 24px.

## Elevation & Depth

Flat. Depth is three surface tiers (chrome / side / content) separated by 1px `line` borders. The only shadow is `shadow-float`, used solely on layers that float above the app: sheets, dropdown/context menus, selects, palette, tooltips. Nothing at rest has a shadow, and overlays do not dim the app (transparent overlay).

- **shadow-float** dark `0 10px 28px -8px rgb(0 0 0/.55), 0 2px 6px -2px rgb(0 0 0/.35)`; light `0 10px 28px -8px rgb(29 33 39/.18), 0 2px 6px -2px rgb(29 33 39/.12)`.

**The Float-Only Rule.** `shadow-float` on floating layers only; never on panels, buttons, badges or rows.

## Shapes

Square panels (radius 0). Controls, menus, sheets' inner elements and the palette use **3px** (`rounded-ctl`). Badges, label swatches, skeleton bars, segmented items inside the 3px group use a 2px inner radius. No pills, no circles. Borders are always 1px `line`; the single stronger border is the primary button's `fg`. Icons: Azure-style line glyphs, 16 viewBox, stroke 1.3, round joins (`glyphs.tsx`: namespace, queue, topic, subscription, rule), shown at 14px; chrome icons are lucide at 14px (`size-3.5`), chevrons 12px.

## Components

Inventory under `src/ui`, exported from `src/ui/index.ts`.

### Buttons
Always bordered or quiet; none use colour.
- **default**: 1px `line`, `field` bg, fg text; hover `hover`, active `selection`. The standard for actions and all recovery actions (Try again, Reconnect, Edit connection).
- **primary**: same fill, 1px **`fg` border**, 600 weight. One per sheet footer or empty state, for the main action only (Save, Add connection).
- **quiet**: no border; hover bg only. Toolbar-ish actions (Show activity, Update available).
- **danger**: visually identical to default (neutral); the danger lives in the label ("Delete {name}") and in an in-context confirmation, never a modal and never amber.
- Sizes sm 22 / md 24 (default) / lg 28 (13px text). Disabled: 45% opacity. Focus: 1px `fg` outline (primary offsets 1px).
- **IconButton**: 24px square, icon colour, requires `label` (aria-label). Wrap in `Hint` for tooltip + shortcut.

### Inputs and fields
`Input`/`Textarea`: 24px, `field` bg, 1px `line`, 3px radius, placeholder muted; focus 1px `fg` outline inset; `aria-invalid` turns border amber. `mono` prop for hosts, keys, IDs. `Field` wraps label (12/600 muted), control, hint (11 muted) and error (amber `CircleAlert` + fg text 12). `Select`, `Segmented` (24px, selected = `selection`), `Kbd` (mono 11 muted, platform-aware symbols).

### Problem and EmptyState
- **Problem**: the one error form. Amber `CircleAlert` + 13/600 title, then a plain sentence (what happened and what to do, 12 muted), then raw engine `detail` on its own mono 11 line (selectable), then actions (default buttons). Use in banners, sheets, and updates; `role="alert"` when it appears while the user is looking.
- **EmptyState**: same layout, max 440px, `px-4 py-5`. `tone="neutral"`: no icon (e.g. "Open an entity"); `tone="error"`: renders the Problem form for whole-pane errors. Left-aligned, no illustration, no centred hero.
- **Skeleton / SkeletonRows**: static `line` bars shaped like the loading content; no shimmer.

### Sheet
Right-edge side sheet, **420px for every sheet**, `side` bg, 1px left line, `shadow-float`, no dimming. Header (title 14/600, optional muted description, Close icon button), scrolling body `px-4 py-4`, footer right-aligned with 1px top line holding buttons (one primary). Slides 24px and fades in 150ms. `dismissible=false` removes Close and ignores Escape/outside for must-finish tasks. `initialFocus="field"` for forms, `"sheet"` (no ring) for read-only.

### Tabs
- **strip** (30px, `chrome`): entity tabs. Cells separated by 1px right borders; active = `content` bg + 2px `fg` top marker. Tab shows name in fg and kind/topic in faint; a 16px close button appears on hover, focus and on the active tab. Overflow scrolls horizontally (wheel maps to sideways), active tab kept in view.
- **underline** (28px, drawer 26px): sub-tabs and drawer tabs on the content/side surface; active = fg text + 2px `fg` bottom marker; others muted. Counts via `CountBadge` with min width 3ch to prevent shift.
- Focus ring on tabs is 1px `fg` inset 2px, suppressed for pointer presses.

### Tree
Virtualised, ARIA tree, rows at `--row`, indent 14px, 14px chevron column (faint, rotates 90deg), 14px entity glyph in `icon`. Selected = `selection` bg (neutral); hover = `hover`. Count column right-aligned 12px: active count in muted, dead-letter count in amber when >0 (`active  DLQ`). Keys: arrows/j/k, Enter, typeahead (500ms). Focus mark sits on the active row, not around the panel. Inline row Retry button is 18px sm default.

### DataGrid
Virtualised, TanStack columns, sticky 12px muted header on `content` with bottom line, rows at `--row`, cells `truncate px-1`; `align:"end"` gives right-aligned tabular numbers; `meta.mono` gives mono 11 tabular. Selection neutral; skeleton rows while loading; focus outline 1px `fg` inset.

### CountBadge / ProdBadge / Badge
- **CountBadge**: plain text (not a pill), 11px (12px in tree) tabular-nums. Muted by default; amber only when `attention` and value > 0. Formats: `?` unknown, `~N` approximate, `~N+` capped, thousands separated.
- **ProdBadge**: 16px tall, 2px radius, accent fill + accent-ink text, `PROD`, 11/600, 0.04em tracking. Shown next to the active connection, in connection menus/lists and on the namespace row. The only filled amber element.
- **Badge** tones: `neutral` (1px line, muted), `accent` (fill), `attention` (accent-tint bg, amber text).

### Menus, palette, tooltip
Dropdown/context/select content: `side`, 1px line, 3px radius, `shadow-float`, min 200px, rows at `--row` with 3px-gap icons (icon colour), highlighted = `selection`, group labels 11 muted, shortcuts mono 11 muted right. Command palette (`cmdk`): 36px input, rows `--row`, list max 360px. Tooltip: `side`, 450ms delay, label + `Kbd`.

## Motion

Only sheets, the drawer and popovers move. Sheet: x 24px + opacity, 150ms, `cubic-bezier(0.16, 1, 0.3, 1)` (ease-out-expo). No transitions on hover, no loading shimmer, no skeleton animation, no layout animation. `prefers-reduced-motion` collapses CSS animations to 0.01ms and transitions to 0s (never a token transition: `transition-property` defaults to `all`, so it would animate every style change), and the sheet appears and leaves at once (no slide and no fade: its motion runs with duration 0).

## Copy voice

- Plain sentences, sentence case, no exclamation, no emoji, no marketing.
- Errors state what happened then what to do: title "Couldn't check for updates", body "Check your network connection, then check again.", raw detail beneath in mono.
- Name the cause: "Connection lost" (Reconnect), "Sign-in expired" and "Credentials rejected" (Edit connection).
- Buttons are verbs naming the object: "Add connection string", "Delete {name}", "Edit connection", "Reconnect"; "Try again" for retries.
- Unknown is `?`, approximate is `~`; never show 0 for unknown. Counts unavailable are said plainly ("Counts are unavailable while the namespace can't be listed.").
- Status text is terse and factual ("Peek only, nothing locked", "Refreshed 16:31:16").
- Secrets are never echoed back after entry.

## Do's and Don'ts

### Do:
- **Do** use amber only per the Earned Amber Rule, always with icon or text.
- **Do** keep selection, hover, focus and primary neutral (`selection`, `hover`, `fg`).
- **Do** use `Problem` for every error; recovery action as a default button next to it.
- **Do** mark the active connection and PROD in the top bar at all times.
- **Do** put machine text (IDs, timestamps, raw errors, shortcuts) in mono 11 with tabular numerals.
- **Do** confirm destructive actions in context (inline in the sheet), never in a modal.
- **Do** give icon-only controls an `aria-label` (`IconButton label`) and a `Hint` where a shortcut exists.
- **Do** make every theme change pass `contrastPairs` in `tokens.ts` (tests are the authority).

### Don't:
- **Don't** put amber on a button, tab, selection, link or background wash beyond `accent-tint` on attention badges.
- **Don't** add cards, gradients, glows, pills, avatars, illustrations, or centred hero empty states.
- **Don't** add radii above 3px, or shadows other than `shadow-float`, or a shadow on anything at rest.
- **Don't** use a modal dialog for confirmation or forms; use the 420px sheet or inline confirmation.
- **Don't** use label-swatch colours for anything but the 8px swatch.
- **Don't** add font weights other than 400 and 600, or load network fonts.
- **Don't** let the window scroll or let a count shift layout (reserve `min-w-[3ch]`).
- **Don't** add new palette colours; extend `tokens.ts` roles instead.

## Accessibility

- Contrast enforced by test (see Colors). Amber is never the sole signal.
- Focus: `:focus-visible` 1px `fg` outline, offset -1px (tabs -2px, primary +1px); visible on every control; pointer presses on tabs do not draw it.
- Full keyboard model: `⌘K`/`Ctrl+K` palette, tree arrows/j/k/Enter/typeahead, resize handles focusable (turn `fg` on keyboard drag), Escape closes sheets and menus.
- Semantics: tree uses ARIA tree, tabs via Radix, `role="status"` for connection state and loading skeletons, `role="alert"` for late-arriving problems, Field links hint/error with `aria-describedby` and sets `aria-invalid`; decorative icons `aria-hidden`.
- Text is selectable only in inputs and `data-selectable` regions (raw error detail); everything else is `user-select: none`.
- Reduced motion honoured (see Motion).

## Extraction notes (for `@horizon-tech/explorer-ui`)

**Generic, extract as-is:** `tokens.ts` (roles, label colours, contrast pairs and tests), `theme.ts` (applyTheme, density, useTheme), `styles.css` (theme reset and base), `contrast.ts`, `lib/*` (cn, platform, virtual, format: `formatCount` semantics `?`/`~`/`~N+` are Service-Bus-neutral), components: Button, IconButton, Input/Textarea, Field, Select, Segmented, Kbd, Skeleton, Badge, CountBadge, Tabs (both variants), Tooltip/Hint, DropdownMenu, ContextMenu, Sheet, Command, Resizable, ThemeScope, EmptyState/Problem, `Tree`, `DataGrid`, LabelSwatch.

**Dispatched-specific, leave behind or parameterise:**
- `glyphs.tsx`: entity glyph set (namespace/queue/topic/subscription/rule) and `EntityKind`; keep the `Glyph` wrapper (16 viewBox, stroke 1.3) generic, move kinds to the app.
- `ProdBadge`: the "PROD" text and "Production profile" title suit any connection-based explorer but are product vocabulary; make the label a prop.
- `CountBadge` amber-on-attention maps to dead letters here; the `attention` prop is generic, the meaning is not.
- `formatDuration`/`formatClock` en-GB clock choice; localise at extraction.
- `src/shell/**` (ConnectionSelector, Workbench, Drawer jobs/activity, sheets, palette commands, troubleLabel copy) stays in the app; the layout anatomy above is the pattern, not a component.
- Sheet width default 420 and Tauri drag-region attributes (`data-tauri-drag-region`) belong to the app shell.
- Known gap: `Button variant="danger"` equals default, by design; if extracted, keep that or rename to avoid implying red.
