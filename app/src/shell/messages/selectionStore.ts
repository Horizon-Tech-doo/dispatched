import type { FilterSpec, PeekView } from "@/engine";

/** What a message view holds now, for commands outside it (the palette): its view, selected rows, filter and count. */
export interface ViewSelection {
  view: PeekView;
  sequences: number[];
  filter: FilterSpec;
  total: number | null;
}

const byTab = new Map<string, ViewSelection>();

/** Set by the message view that is showing; cleared when it goes. */
export const setViewSelection = (tabId: string, s: ViewSelection | null) => (s ? byTab.set(tabId, s) : byTab.delete(tabId));
export const viewSelection = (tabId: string): ViewSelection | null => byTab.get(tabId) ?? null;
