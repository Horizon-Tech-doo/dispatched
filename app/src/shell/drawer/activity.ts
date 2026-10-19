import { useSyncExternalStore } from "react";
import { onActivity, onJob, type ActivityEntry, type ActivityError, type ExportFormat, type FilterSpec, type JobEvent, type PeekView } from "@/engine";

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

/** What a retry of an export needs: the sheet opens on it again. Set by the export sheet when the job starts. */
export interface ExportRetry {
  entity: string;
  view: PeekView;
  filter: FilterSpec;
  scope: "all" | "selected";
  /** The selected sequence numbers (empty for the whole view). */
  selected: number[];
  format: ExportFormat;
  total: number | null;
}

export interface Job {
  id: string;
  /** What the job is: `Export orders (dead-letter queue)`. */
  title: string;
  /** The file it writes, by name. */
  fileName: string | null;
  done: number;
  total: number | null;
  /** Messages examined so far (an export with a filter examines more than it writes). */
  scanned: number;
  error: ActivityError | null;
  finished: boolean;
  /** The activity log's line for a finished job, e.g. `Exported 37 messages to x.json`. */
  summary: string | null;
  /** The file a finished export wrote, in full (for Show in folder). */
  output: string | null;
  /** Messages a finished job wrote; null while running and after a failure. */
  written: number | null;
  /** Selected messages that were no longer there; null while running and after a failure. */
  missing: number | null;
  /** How to start the same export again. */
  retry: ExportRetry | null;
}

export function createJobStore() {
  const s = createStore<Job>();
  // A retry that arrived before its job's `started` event.
  const pendingRetry = new Map<string, ExportRetry>();
  return {
    apply(e: JobEvent) {
      const jobs = s.get();
      if (e.type === "started") {
        const job: Job = {
          id: e.id, title: e.title, fileName: e.file_name, done: 0, total: e.total, scanned: 0, error: null, finished: false,
          summary: null, output: null, written: null, missing: null, retry: pendingRetry.get(e.id) ?? null,
        };
        pendingRetry.delete(e.id);
        // A job that starts again under the same id replaces the old one.
        return s.set(jobs.some((j) => j.id === e.id) ? jobs.map((j) => (j.id === e.id ? job : j)) : [...jobs, job]);
      }
      if (!jobs.some((j) => j.id === e.id)) return;
      s.set(
        jobs.map((j) =>
          j.id !== e.id
            ? j
            : e.type === "progress"
              ? { ...j, done: e.done, total: e.total, scanned: e.scanned }
              : { ...j, finished: true, error: e.error, summary: e.summary, output: e.output, written: e.written, missing: e.missing, fileName: e.file_name ?? j.fileName },
        ),
      );
    },
    /** Remembers how to start a job's export again (the job may not have been announced yet). */
    setRetry(id: string, retry: ExportRetry) {
      if (s.get().some((j) => j.id === id)) s.set(s.get().map((j) => (j.id === id ? { ...j, retry } : j)));
      else pendingRetry.set(id, retry);
    },
    /** Forgets the jobs that have finished; running ones stay. */
    clearFinished: () => s.set(s.get().filter((j) => !j.finished)),
    all: s.get,
    clear: () => {
      pendingRetry.clear();
      s.set([]);
    },
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
