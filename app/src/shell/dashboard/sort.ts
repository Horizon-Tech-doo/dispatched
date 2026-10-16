import type { DashboardRow } from "@/engine";
import type { GridSort } from "@/ui";

const count = (pick: (r: DashboardRow) => number | undefined) => (r: DashboardRow) => pick(r) ?? -1; // unknown sorts below zero

export const pathOf = (r: DashboardRow) => `${r.topic ?? ""}/${r.name}`;

/** What each column sorts by. The grid's columns and `sortRows` both read this, so "Copy as TSV" can give the rows in the
 *  order the grid shows them. */
export const sortValue: Record<string, (r: DashboardRow) => string | number> = {
  entity: pathOf,
  status: (r) => r.status ?? "",
  active: count((r) => r.counts?.active),
  dlq: count((r) => r.counts?.dead_letter),
  scheduled: count((r) => r.counts?.scheduled),
  transfer: count((r) => r.counts?.transfer),
  tdlq: count((r) => r.counts?.transfer_dead_letter),
};

export function compareValues(a: string | number, b: string | number): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return String(a).localeCompare(String(b), "en", { sensitivity: "base", numeric: true });
}

/** The rows in the order of `sort`; ties keep their incoming order, in either direction (as the grid does). */
export function sortRows(rows: DashboardRow[], sort: GridSort | null): DashboardRow[] {
  const get = sort ? sortValue[sort.id] : undefined;
  if (!sort || !get) return rows;
  const dir = sort.desc ? -1 : 1;
  return [...rows].sort((a, b) => dir * compareValues(get(a), get(b)));
}
