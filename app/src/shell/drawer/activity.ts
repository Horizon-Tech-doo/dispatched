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
