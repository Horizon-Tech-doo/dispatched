import type { ItemView, JobEvent, JobKind, ReportSummary } from "@/engine";

type Of<T extends JobEvent["type"]> = Extract<JobEvent, { type: T }>;

/** Job events with the fields most callers do not care about filled in (tests, and anything that fakes the engine). */
export const started = (id: string, title: string, total: number | null = null, file_name: string | null = null, kind: JobKind = "export"): Of<"started"> => ({
  type: "started", id, kind, title, total, file_name,
});
export const progress = (id: string, done: number, total: number | null = null, scanned: number = done): Of<"progress"> => ({ type: "progress", id, done, total, scanned });
export const finished = (id: string, p: Partial<Omit<Of<"finished">, "type" | "id">> = {}): Of<"finished"> => ({
  type: "finished", id, error: null, report: null, summary: null, output: null, written: null, missing: null, file_name: null, ...p,
});
export const itemEvent = (id: string, item: Partial<ItemView> & Pick<ItemView, "outcome">): Of<"item"> => ({
  type: "item", id, item: { sequence_number: null, message_id: null, session_id: null, entity: "orders", new_sequence_number: null, reason: null, ...item },
});
/** A report summary with every count at zero unless given. */
export const report = (p: Partial<ReportSummary> = {}): ReportSummary => ({
  op_id: "op-1", kind: "delete", entity: "orders", target: null, outcome: "succeeded", cancelled: false, items: 0, succeeded: 0, failed: 0, skipped: 0, missing: 0,
  incomplete: 0, notes: [], summary_line: "", ...p,
});
