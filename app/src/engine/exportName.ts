import type { PeekView } from "./types";

/** As Rust `suggested_file_name`, which names the file the save dialog offers: `{entity-slug}-{view}-{yyyyMMdd-HHmm}.{ext}`
 *  in the user's local time (the engine gets the offset from `localOffsetMinutes`), anything but letters, digits, `.`, `_`
 *  and `-` in the entity becoming `-`. The sheet shows it before the dialog opens; the dialog's own stamp is the minute it
 *  opens. */
export function suggestedFileName(entity: string, view: PeekView, format: string, now = new Date()): string {
  const p = (n: number) => String(n).padStart(2, "0");
  const stamp = `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}-${p(now.getHours())}${p(now.getMinutes())}`;
  return `${entity.replace(/[^A-Za-z0-9._-]/g, "-")}-${view}-${stamp}.${format.toLowerCase() === "ndjson" ? "ndjson" : "json"}`;
}

/** The user's zone as minutes east of UTC, for the engine to stamp export names in local time. */
export const localOffsetMinutes = (now = new Date()): number => -now.getTimezoneOffset();
