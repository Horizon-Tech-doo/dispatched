const pad = (n: number) => String(n).padStart(2, "0");

/** `2026-10-15 09:41:12` in local time; empty for null or unparsable input. */
export function formatTimestamp(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `15 Oct 09:41:07` in local time, `15 Oct 2025 09:41:07` with `withYear`; empty for null or unparsable input.
 *  A grid column decides `withYear` once for all its rows, so the column keeps one width. */
export function formatShortTimestamp(iso: string | null, withYear: boolean): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const day = `${pad(d.getDate())} ${MONTHS[d.getMonth()]}${withYear ? ` ${d.getFullYear()}` : ""}`;
  return `${day} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** The local year of a timestamp, or null. */
export function yearOf(iso: string | null): number | null {
  if (!iso) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d.getFullYear();
}
