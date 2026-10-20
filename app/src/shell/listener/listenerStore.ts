import { useSyncExternalStore } from "react";
import type { JobEvent, ReceivedView } from "@/engine";

/** The grid keeps the newest this many messages; the counters count them all. */
export const ROLLING_ROWS = 1000;
/** The rate chart's window, in one-second buckets. */
export const RATE_WINDOW = 60;

export interface Listened {
  /** Newest first, at most ROLLING_ROWS. */
  rows: ReceivedView[];
  total: number;
  bytes: number;
  /** Per-second counts, oldest first, the last RATE_WINDOW seconds that saw a message. */
  buckets: { at: number; n: number; bytes: number }[];
}

/** What listeners received, by job id: fed from job events, read by the Listener tab. */
export function createListenerStore() {
  const listeners = new Set<() => void>();
  const jobs = new Map<string, Listened>();
  const emit = () => listeners.forEach((f) => f());
  return {
    /** A job event; `now` in milliseconds. */
    apply(e: JobEvent, now = Date.now()) {
      if (e.type === "started" && e.kind === "listen") {
        jobs.set(e.id, { rows: [], total: 0, bytes: 0, buckets: [] });
        emit();
        return;
      }
      if (e.type !== "received") return;
      const l = jobs.get(e.id);
      if (!l) return;
      const sec = Math.floor(now / 1000);
      const size = e.message.body_size ?? 0;
      const last = l.buckets.at(-1);
      const buckets = (last?.at === sec ? [...l.buckets.slice(0, -1), { at: sec, n: last.n + 1, bytes: last.bytes + size }] : [...l.buckets, { at: sec, n: 1, bytes: size }]).filter(
        (b) => b.at > sec - RATE_WINDOW,
      );
      jobs.set(e.id, { rows: [e.message, ...l.rows].slice(0, ROLLING_ROWS), total: l.total + 1, bytes: l.bytes + size, buckets });
      emit();
    },
    get: (id: string) => jobs.get(id),
    subscribe(f: () => void) {
      listeners.add(f);
      return () => void listeners.delete(f);
    },
  };
}

/** Messages and bytes per second for the RATE_WINDOW seconds ending at `nowSec`, oldest first. */
export function ratesOf(l: Listened, nowSec: number): { messages: number[]; bytes: number[] } {
  const messages = new Array<number>(RATE_WINDOW).fill(0);
  const bytes = new Array<number>(RATE_WINDOW).fill(0);
  for (const b of l.buckets) {
    const i = RATE_WINDOW - 1 - (nowSec - b.at);
    if (i >= 0 && i < RATE_WINDOW) {
      messages[i] = b.n;
      bytes[i] = b.bytes;
    }
  }
  return { messages, bytes };
}

export const listenerStore = createListenerStore();

export function useListened(jobId: string | null): Listened | undefined {
  return useSyncExternalStore(listenerStore.subscribe, () => (jobId ? listenerStore.get(jobId) : undefined));
}

// ---- Which listener each tab runs ----

const tabListeners = new Map<string, string>();
const tabSubs = new Set<() => void>();
/** A tab's key: its connection and entity. */
export const tabKey = (profileId: string | null, entity: string) => `${profileId ?? ""}|${entity}`;
export const tabListener = {
  get: (key: string) => tabListeners.get(key) ?? null,
  set(key: string, jobId: string | null) {
    if (jobId) tabListeners.set(key, jobId);
    else tabListeners.delete(key);
    tabSubs.forEach((f) => f());
  },
  /** Every tab key with a listener. */
  keys: () => [...tabListeners.keys()],
  subscribe(f: () => void) {
    tabSubs.add(f);
    return () => void tabSubs.delete(f);
  },
};
export const useTabListener = (key: string) => useSyncExternalStore(tabListener.subscribe, () => tabListener.get(key));
