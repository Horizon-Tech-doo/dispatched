import type { JobEvent } from "@/engine";

type Of<T extends JobEvent["type"]> = Extract<JobEvent, { type: T }>;

/** Job events with the fields most callers do not care about filled in (tests, and anything that fakes the engine). */
export const started = (id: string, title: string, total: number | null = null, file_name: string | null = null): Of<"started"> => ({ type: "started", id, title, total, file_name });
export const progress = (id: string, done: number, total: number | null = null, scanned: number = done): Of<"progress"> => ({ type: "progress", id, done, total, scanned });
export const finished = (id: string, p: Partial<Omit<Of<"finished">, "type" | "id">> = {}): Of<"finished"> => ({
  type: "finished", id, error: null, summary: null, output: null, written: null, missing: null, file_name: null, ...p,
});
