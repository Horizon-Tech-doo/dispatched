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

/** An entry as logged: the id only grows, so it stays a stable key after the cap trims the oldest rows. */
export type LoggedEntry = ActivityEntry & { id: number };

export function createActivityStore() {
  const s = createStore<LoggedEntry>();
  let nextId = 1;
  return {
    add: (e: ActivityEntry) => s.set([...s.get(), { ...e, id: nextId++ }].slice(-ACTIVITY_CAP)),
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
      if (e.type === "started") {
        const job: Job = { id: e.id, title: e.title, done: 0, total: e.total, error: null, finished: false };
        // A job that starts again under the same id replaces the old one.
        return s.set(jobs.some((j) => j.id === e.id) ? jobs.map((j) => (j.id === e.id ? job : j)) : [...jobs, job]);
      }
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
  const results = await Promise.allSettled([onActivity((e) => activityStore.add(e)), onJob((e) => jobStore.apply(e))]);
  const offs = results.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
  const failed = results.find((r): r is PromiseRejectedResult => r.status === "rejected");
  if (failed) {
    offs.forEach((off) => off());
    throw failed.reason;
  }
  return () => offs.forEach((off) => off());
}
