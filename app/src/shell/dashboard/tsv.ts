import type { DashboardRow, EntityCounts } from "@/engine";

const cell = (v: string | number | null | undefined) => (v == null ? "" : String(v).replace(/[\t\r\n]+/g, " "));

/** A count as the grid shows it, without the thousands separators: `~N` when it was counted by peeking, `~N+` when
 *  that stopped at its cap. Unknown stays empty. */
const count = (c: EntityCounts | null, pick: (c: EntityCounts) => number): string => {
  if (!c) return "";
  const n = pick(c);
  return c.truncated ? `~${n}+` : c.approximate ? `~${n}` : String(n);
};

/** Header plus one line per row, in the order given; numbers raw, unknown counts empty.
 *  `kindsKnown: false` is the limited view, where a queue row can be a topic: its kind is unknown, so it is empty. */
export function toTsv(rows: DashboardRow[], opts: { kindsKnown?: boolean } = {}): string {
  const kindsKnown = opts.kindsKnown ?? true;
  const head = ["Entity", "Topic", "Kind", "Status", "Active", "Dead-letter", "Scheduled", "Transfer", "Transfer DLQ"];
  const lines = rows.map((r) => {
    const c = r.counts;
    const kind = kindsKnown || r.kind === "subscription" ? r.kind : null;
    return [
      cell(r.name), cell(r.topic), cell(kind), cell(r.status),
      count(c, (x) => x.active), count(c, (x) => x.dead_letter), count(c, (x) => x.scheduled), count(c, (x) => x.transfer), count(c, (x) => x.transfer_dead_letter),
    ].join("\t");
  });
  return [head.join("\t"), ...lines].join("\n");
}
