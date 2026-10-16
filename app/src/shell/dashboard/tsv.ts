import type { DashboardRow } from "@/engine";

const cell = (v: string | number | null | undefined) => (v == null ? "" : String(v).replace(/[\t\r\n]+/g, " "));

/** Header plus one line per row; numbers raw, unknown counts empty.
 *  `kindsKnown: false` is the limited view, where a queue row can be a topic: its kind is unknown, so it is empty. */
export function toTsv(rows: DashboardRow[], opts: { kindsKnown?: boolean } = {}): string {
  const kindsKnown = opts.kindsKnown ?? true;
  const head = ["Entity", "Topic", "Kind", "Status", "Active", "Dead-letter", "Scheduled", "Transfer", "Transfer DLQ"];
  const lines = rows.map((r) => {
    const c = r.counts;
    const kind = kindsKnown || r.kind === "subscription" ? r.kind : null;
    return [r.name, r.topic, kind, r.status, c?.active, c?.dead_letter, c?.scheduled, c?.transfer, c?.transfer_dead_letter].map(cell).join("\t");
  });
  return [head.join("\t"), ...lines].join("\n");
}
