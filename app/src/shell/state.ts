import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import { commands, keys, useSetting } from "@/engine";
import { labelColors, type Density, type LabelColor, type ThemePref } from "@/ui";

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const oneOf = <T,>(v: unknown, options: readonly T[], fallback: T): T => (options.includes(v as T) ? (v as T) : fallback);
const num = (v: unknown, min: number, max: number, fallback: number) =>
  typeof v === "number" && Number.isFinite(v) && v >= min && v <= max ? v : fallback;

// ---- prefs (global, key "ui") ----

export type AutoRefresh = 0 | 30 | 60;
/** Messages each view reads at first (its Top N until the user picks another). */
export type PeekSize = 50 | 100 | 500 | 1000;
export const PEEK_SIZES = [50, 100, 500, 1000] as const;
/** Seconds between the Dashboard's reads; 0 is off. */
export type DashboardRefresh = 0 | 30 | 60 | 300;
export const DASHBOARD_REFRESH = [0, 30, 60, 300] as const;
export interface Prefs { theme: ThemePref; density: Density; autoRefresh: AutoRefresh; peekSize: PeekSize; dashboardRefresh: DashboardRefresh }
export const defaultPrefs: Prefs = { theme: "system", density: "compact", autoRefresh: 0, peekSize: 100, dashboardRefresh: 0 };

export function parsePrefs(v: unknown): Prefs {
  const o = isObj(v) ? v : {};
  return {
    theme: oneOf(o.theme, ["system", "dark", "light"] as const, defaultPrefs.theme),
    density: oneOf(o.density, ["compact", "comfortable"] as const, defaultPrefs.density),
    autoRefresh: oneOf(o.autoRefresh, [0, 30, 60] as const, defaultPrefs.autoRefresh),
    peekSize: oneOf(o.peekSize, PEEK_SIZES, defaultPrefs.peekSize),
    dashboardRefresh: oneOf(o.dashboardRefresh, DASHBOARD_REFRESH, defaultPrefs.dashboardRefresh),
  };
}

export function usePrefs(): [Prefs, (patch: Partial<Prefs>) => void] {
  const q = useSetting("ui");
  const qc = useQueryClient();
  const prefs = parsePrefs(q.data);
  // Read the latest value from the cache so two quick updates compose instead of the second overwriting the first.
  const update = useCallback(
    (patch: Partial<Prefs>) => {
      const next = { ...parsePrefs(qc.getQueryData(keys.setting("ui"))), ...patch };
      qc.setQueryData(keys.setting("ui"), next);
      void commands.settingsSet("ui", next).catch(() => {});
    },
    [qc],
  );
  return [prefs, update];
}

// ---- per-profile shell state (key "shell:<profile id>") ----

export const SUB_TABS = ["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "rules", "listener"] as const;
export type SubTabId = (typeof SUB_TABS)[number];
export type EntityKind = "queue" | "topic" | "subscription";

/** What a tab shows: an entity, or the namespace-wide dashboard. */
export type TabKind = EntityKind | "dashboard";
/** The dashboard's tab id: no entity can have it (entity names cannot contain a colon). */
export const DASHBOARD_ID = "::dashboard";
export interface OpenTab { id: string; kind: TabKind; subTab: SubTabId }
/** A tab showing an entity; entity views take this type so they never have to handle the dashboard case. */
export type EntityTab = OpenTab & { kind: EntityKind };
export const isEntityTab = (t: OpenTab): t is EntityTab => t.kind !== "dashboard";
export interface Layout { treeSize: number; drawerOpen: boolean; drawerSize: number; drawerTab: "jobs" | "activity"; detailSize: number }
/** A count the tree can show beside a row and a highlight rule can test. There is no transfer count here: transfer
 *  messages are in flight to another entity and cannot be browsed, so only their dead letters are offered. */
export type CountKind = "active" | "dead-letter" | "scheduled" | "transfer-dlq";
export const COUNT_KINDS: readonly CountKind[] = ["active", "scheduled", "transfer-dlq", "dead-letter"];
export type HighlightRule = {
  id: string;
  color: LabelColor;
  match: { kind: "name"; pattern: string } | { kind: "count"; count: CountKind; atLeast: number };
};
export type OpenOn = "overview" | "messages" | "dead-letter";
/** How the tree and a newly opened entity look for one connection. */
export interface ExplorerOptions { counts: CountKind[]; rules: HighlightRule[]; openOn: OpenOn }
export const MAX_RULES = 20;
export const MAX_PATTERN_LENGTH = 200;
export const MAX_FILTER_LENGTH = 200;
export const defaultExplorer: ExplorerOptions = { counts: ["active", "dead-letter"], rules: [], openOn: "overview" };
export interface ProfileShell {
  tabs: OpenTab[];
  activeTab: string | null;
  expanded: string[];
  selected: string | null;
  layout: Layout;
  explorer: ExplorerOptions;
  /** The tree's filter text. */
  treeFilter: string;
}

export const MAX_TABS = 30;
export const emptyShell: ProfileShell = {
  tabs: [],
  activeTab: null,
  expanded: ["group:queues", "group:topics"],
  selected: null,
  layout: { treeSize: 18, drawerOpen: true, drawerSize: 18, drawerTab: "activity", detailSize: 38 },
  explorer: defaultExplorer,
  treeFilter: "",
};

function parseRule(v: unknown): HighlightRule | null {
  if (!isObj(v) || typeof v.id !== "string" || v.id.length === 0 || v.id.length > 64) return null;
  if (typeof v.color !== "string" || !Object.prototype.hasOwnProperty.call(labelColors, v.color)) return null;
  const color = v.color as LabelColor;
  const m = v.match;
  if (!isObj(m)) return null;
  if (m.kind === "name") return typeof m.pattern === "string" && m.pattern.length <= MAX_PATTERN_LENGTH ? { id: v.id, color, match: { kind: "name", pattern: m.pattern } } : null;
  if (m.kind === "count" && COUNT_KINDS.includes(m.count as CountKind) && typeof m.atLeast === "number" && Number.isFinite(m.atLeast) && m.atLeast >= 0)
    return { id: v.id, color, match: { kind: "count", count: m.count as CountKind, atLeast: m.atLeast } };
  return null;
}

function parseExplorer(v: unknown): ExplorerOptions {
  if (!isObj(v)) return defaultExplorer;
  const known = Array.isArray(v.counts) ? COUNT_KINDS.filter((k) => (v.counts as unknown[]).includes(k)) : [];
  const rules = (Array.isArray(v.rules) ? v.rules : []).map(parseRule).filter((r): r is HighlightRule => r !== null).slice(0, MAX_RULES);
  return {
    // Kept in the tree's own order (active, scheduled, transfer DLQ, dead-letter); none known means the default.
    counts: known.length ? known : defaultExplorer.counts,
    rules,
    openOn: oneOf(v.openOn, ["overview", "messages", "dead-letter"] as const, defaultExplorer.openOn),
  };
}

export function parseProfileShell(v: unknown): ProfileShell {
  if (!isObj(v)) return emptyShell;
  const tabs: OpenTab[] = (Array.isArray(v.tabs) ? v.tabs : [])
    .filter(isObj)
    .filter((t) => typeof t.id === "string" && t.id.length > 0 && (t.kind === "dashboard" ? t.id === DASHBOARD_ID : ["queue", "topic", "subscription"].includes(t.kind as string)))
    .slice(0, MAX_TABS)
    .map((t) => ({ id: t.id as string, kind: t.kind as TabKind, subTab: oneOf(t.subTab, SUB_TABS, "overview") }));
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
      detailSize: num(l.detailSize, 20, 70, d.detailSize),
    },
    explorer: parseExplorer(v.explorer),
    treeFilter: typeof v.treeFilter === "string" && v.treeFilter.length <= MAX_FILTER_LENGTH ? v.treeFilter : "",
  };
}

export type ShellAction =
  | { type: "open"; tab: { id: string; kind: TabKind; subTab?: SubTabId } }
  | { type: "close"; id: string }
  | { type: "activate"; id: string }
  | { type: "activateIndex"; index: number }
  | { type: "subTab"; id: string; subTab: SubTabId }
  | { type: "toggleExpanded"; nodeId: string; expanded: boolean }
  | { type: "select"; nodeId: string | null }
  | { type: "layout"; patch: Partial<Layout> }
  | { type: "explorer"; options: ExplorerOptions }
  | { type: "treeFilter"; value: string }
  | { type: "reset"; state: ProfileShell };

export function shellReducer(s: ProfileShell, a: ShellAction): ProfileShell {
  switch (a.type) {
    case "open": {
      if (s.tabs.some((t) => t.id === a.tab.id)) return { ...s, activeTab: a.tab.id };
      const at = s.activeTab ? s.tabs.findIndex((t) => t.id === s.activeTab) + 1 : s.tabs.length;
      const tab: OpenTab = { id: a.tab.id, kind: a.tab.kind, subTab: a.tab.subTab ?? (a.tab.kind === "topic" || a.tab.kind === "dashboard" ? "overview" : s.explorer.openOn) };
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
    case "layout": {
      const layout = { ...s.layout, ...a.patch };
      // Panel groups report their sizes on mount; unchanged sizes must not look like an edit.
      return (Object.keys(layout) as (keyof Layout)[]).every((k) => layout[k] === s.layout[k]) ? s : { ...s, layout };
    }
    case "explorer":
      return { ...s, explorer: a.options };
    case "treeFilter":
      return s.treeFilter === a.value ? s : { ...s, treeFilter: a.value };
    case "reset":
      return a.state;
  }
}

/** Shell state for a profile, loaded from and saved (debounced) to settings.
 *  The id must stay stable while the same profile reconnects; changing it flushes the pending save and reloads. */
export function useProfileShell(profileId: string | null): [ProfileShell, Dispatch<ShellAction>, boolean] {
  const [state, dispatch] = useReducer(shellReducer, emptyShell);
  const [loadedFor, setLoadedFor] = useState<string | null>(null);
  const loaded = useRef<ProfileShell | null>(null); // the state as loaded, so it is not written straight back
  const pending = useRef<{ profileId: string; state: ProfileShell } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const flush = useCallback(() => {
    clearTimeout(timer.current);
    const p = pending.current;
    pending.current = null;
    if (p) void commands.settingsSet(`shell:${p.profileId}`, p.state).catch(() => {});
  }, []);

  useEffect(() => {
    if (!profileId) return;
    let live = true;
    const apply = (s: ProfileShell) => {
      if (!live) return;
      loaded.current = s;
      dispatch({ type: "reset", state: s });
      setLoadedFor(profileId);
    };
    commands
      .settingsGet(`shell:${profileId}`)
      .then((v) => apply(parseProfileShell(v)))
      .catch(() => apply(emptyShell));
    return () => {
      live = false;
      flush();
    };
  }, [profileId, flush]);

  useEffect(() => {
    if (!profileId || loadedFor !== profileId || state === loaded.current) return;
    pending.current = { profileId, state };
    clearTimeout(timer.current);
    timer.current = setTimeout(flush, 300);
  }, [state, profileId, loadedFor, flush]);

  return [state, dispatch, loadedFor === profileId];
}
