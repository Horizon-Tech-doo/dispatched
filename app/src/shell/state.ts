import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import { commands, keys, useSetting } from "@/engine";
import type { Density, ThemePref } from "@/ui";

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const oneOf = <T,>(v: unknown, options: readonly T[], fallback: T): T => (options.includes(v as T) ? (v as T) : fallback);
const num = (v: unknown, min: number, max: number, fallback: number) =>
  typeof v === "number" && Number.isFinite(v) && v >= min && v <= max ? v : fallback;

// ---- prefs (global, key "ui") ----

export type AutoRefresh = 0 | 30 | 60;
/** Messages each view reads at first (its Top N until the user picks another). */
export type PeekSize = 50 | 100 | 500 | 1000;
export const PEEK_SIZES = [50, 100, 500, 1000] as const;
export interface Prefs { theme: ThemePref; density: Density; autoRefresh: AutoRefresh; peekSize: PeekSize }
export const defaultPrefs: Prefs = { theme: "system", density: "compact", autoRefresh: 0, peekSize: 100 };

export function parsePrefs(v: unknown): Prefs {
  const o = isObj(v) ? v : {};
  return {
    theme: oneOf(o.theme, ["system", "dark", "light"] as const, defaultPrefs.theme),
    density: oneOf(o.density, ["compact", "comfortable"] as const, defaultPrefs.density),
    autoRefresh: oneOf(o.autoRefresh, [0, 30, 60] as const, defaultPrefs.autoRefresh),
    peekSize: oneOf(o.peekSize, PEEK_SIZES, defaultPrefs.peekSize),
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

export interface OpenTab { id: string; kind: EntityKind; subTab: SubTabId }
/** A tab showing an entity. The same as `OpenTab` until the dashboard tab arrives; entity views take this type so
 *  they never have to handle the dashboard case. */
export type EntityTab = OpenTab & { kind: EntityKind };
export interface Layout { treeSize: number; drawerOpen: boolean; drawerSize: number; drawerTab: "jobs" | "activity" }
export interface ProfileShell { tabs: OpenTab[]; activeTab: string | null; expanded: string[]; selected: string | null; layout: Layout }

export const MAX_TABS = 30;
export const emptyShell: ProfileShell = {
  tabs: [],
  activeTab: null,
  expanded: ["group:queues", "group:topics"],
  selected: null,
  layout: { treeSize: 18, drawerOpen: true, drawerSize: 18, drawerTab: "activity" },
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
    case "layout": {
      const layout = { ...s.layout, ...a.patch };
      // Panel groups report their sizes on mount; unchanged sizes must not look like an edit.
      return (Object.keys(layout) as (keyof Layout)[]).every((k) => layout[k] === s.layout[k]) ? s : { ...s, layout };
    }
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
