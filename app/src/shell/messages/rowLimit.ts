/** The grid holds at most this many rows: more than this belongs in an export, not a scrolling grid. */
export const ROW_LIMIT = 100_000;

let limit = ROW_LIMIT;

/** The row limit in force: [`ROW_LIMIT`], unless a test set a smaller one. */
export const rowLimit = (): number => limit;

/** Tests only: a smaller limit, so the cap is exercised without 100,000 rows; `null` restores `ROW_LIMIT`. */
export function setRowLimitForTests(n: number | null): void {
  limit = n ?? ROW_LIMIT;
}
