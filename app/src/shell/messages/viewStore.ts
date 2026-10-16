import { useCallback, useSyncExternalStore } from "react";
import type { FilterSpec, PeekFrom } from "@/engine";
import type { GridSort } from "@/ui";

export interface ViewState {
  from: PeekFrom;
  topN: number;
  filter: FilterSpec;
  filterOpen: boolean;
  sorting: GridSort | null;
  detailOpen: boolean;
}

/** What a view shows before the user changes anything. Built once per Top N default, so the snapshot stays stable. */
export const defaultViewState = (topN: number): ViewState => ({ from: { kind: "start" }, topN, filter: {}, filterOpen: false, sorting: null, detailOpen: false });

/** View state lives for the session (not persisted): switching sub-tabs or tabs keeps it. */
const states = new Map<string, ViewState>();
const subs = new Set<() => void>();
const notify = () => subs.forEach((f) => f());
const subscribe = (cb: () => void) => {
  subs.add(cb);
  return () => void subs.delete(cb);
};

export const viewKey = (profileId: string | null, tabId: string, view: string) => `${profileId ?? ""}|${tabId}|${view}`;

export function setViewState(key: string, defaults: ViewState, patch: Partial<ViewState>) {
  states.set(key, { ...(states.get(key) ?? defaults), ...patch });
  notify();
}

/** Forget a profile's view states (its tabs are gone with the connection). */
export function clearViewStates(profileId: string) {
  for (const k of [...states.keys()]) if (k.startsWith(`${profileId}|`)) states.delete(k);
  notify();
}

/** Forget every view state. Tests call it between renders: this is module state, so a filter left
 *  open or a sort set by one test would otherwise reach the next. */
export function resetViewStates() {
  states.clear();
  notify();
}

/** `defaults` must be stable (build it with `useMemo`): it is the snapshot until the view is first changed. */
export function useViewState(key: string, defaults: ViewState): [ViewState, (patch: Partial<ViewState>) => void] {
  const state = useSyncExternalStore(subscribe, () => states.get(key) ?? defaults);
  const update = useCallback((patch: Partial<ViewState>) => setViewState(key, defaults, patch), [key, defaults]);
  return [state, update];
}
