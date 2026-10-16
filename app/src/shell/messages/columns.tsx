import { useQueryClient } from "@tanstack/react-query";
import type { ColumnDef } from "@tanstack/react-table";
import { useCallback, useMemo } from "react";
import { commands, keys, useSetting, type MessageSummary, type PeekView, type PropValue } from "@/engine";
import { formatBytes } from "@/ui";
import { formatShortTimestamp } from "../time";

export type ColumnId =
  | "seq" | "enqueued" | "scheduled" | "subject" | "messageId" | "correlationId" | "session" | "contentType" | "to" | "replyTo"
  | "delivery" | "size" | "ttl" | "reason" | "description" | "source" | "state" | "preview" | `prop:${string}`;

type BuiltIn = Exclude<ColumnId, `prop:${string}`>;

export const COLUMN_LABEL: Record<BuiltIn, string> = {
  seq: "Seq", enqueued: "Enqueued", scheduled: "Scheduled for", subject: "Subject", messageId: "Message ID", correlationId: "Correlation ID",
  session: "Session", contentType: "Content type", to: "To", replyTo: "Reply to", delivery: "Deliveries", size: "Size", ttl: "Time to live",
  reason: "Reason", description: "Description", source: "Dead-letter source", state: "State", preview: "Body",
};

/** Every optional column, in menu order (Seq is always shown: rows are identified by it). */
export const OPTIONAL: BuiltIn[] = [
  "enqueued", "scheduled", "subject", "messageId", "correlationId", "session", "contentType", "to", "replyTo",
  "delivery", "size", "ttl", "reason", "description", "source", "state", "preview",
];

type Group = "active" | "dead-letter" | "scheduled" | "deferred";
const groupOf = (v: PeekView): Group => (v === "transfer-dead-letter" ? "dead-letter" : v);

export function defaultColumns(view: PeekView, sessions: boolean): ColumnId[] {
  switch (groupOf(view)) {
    case "dead-letter":
      return ["seq", "enqueued", "subject", "messageId", "reason", "description", "delivery", "size"];
    case "scheduled":
      return ["seq", "scheduled", "subject", "messageId", "size", "preview"];
    case "deferred":
      return ["seq", "enqueued", "subject", "messageId", "delivery", "size", "preview"];
    default:
      return ["seq", "enqueued", "subject", "messageId", ...(sessions ? (["session"] as ColumnId[]) : []), "delivery", "size", "preview"];
  }
}

/** The columns a view can fill, in menu order: no column that would be blank on every row (dead-letter fields
 *  outside the dead-letter queues, a session column for an entity without sessions, deliveries of messages that
 *  were never delivered). */
export function optionalColumns(view: PeekView, sessions: boolean): BuiltIn[] {
  const g = groupOf(view);
  const offered = (id: BuiltIn): boolean => {
    switch (id) {
      case "scheduled": return g === "scheduled";
      case "session": return sessions;
      case "delivery": return g !== "scheduled";
      case "reason": case "description": case "source": case "state": return g === "dead-letter";
      default: return true;
    }
  };
  return OPTIONAL.filter(offered);
}

export const propColumn = (name: string): ColumnId => `prop:${name}`;
const isProp = (v: string) => v.startsWith("prop:") && v.length > 5;
// An own key, not `in`: "toString" and friends are on every object's prototype. (`Object.hasOwn` needs an ES2022
// lib, which this project's TypeScript target does not include.)
const isId = (v: unknown): v is ColumnId => typeof v === "string" && (Object.prototype.hasOwnProperty.call(COLUMN_LABEL, v) || isProp(v));

/** The saved list as written: every known column, including ones this entity cannot fill. `null` when there is
 *  nothing usable (not a list, empty, or an unknown entry), which means the defaults. */
function savedList(saved: unknown): ColumnId[] | null {
  return Array.isArray(saved) && saved.length > 0 && saved.every(isId) ? (saved as ColumnId[]) : null;
}

/** The columns of `list` this view can fill. */
function fillable(list: ColumnId[], view: PeekView, sessions: boolean): ColumnId[] {
  const offered = new Set<string>(["seq", ...optionalColumns(view, sessions)]);
  return list.filter((id) => isProp(id) || offered.has(id));
}

/** The columns to show for this view from a saved list: unknown entries (or none) fall back to the defaults, and
 *  columns the view can't fill are left out of what is shown (they stay in the saved list: see `mergeColumns`). */
export function parseColumns(saved: unknown, view: PeekView, sessions: boolean): ColumnId[] {
  const list = savedList(saved);
  return list ? fillable(list, view, sessions) : defaultColumns(view, sessions);
}

/** What to save when the columns shown change to `next`. A saved list belongs to a view group, not to one entity:
 *  columns this entity can't show (Session, on an entity without sessions) stay in it, where they were, so
 *  changing columns here doesn't drop them for the entities that do show them. */
export function mergeColumns(saved: unknown, next: ColumnId[], view: PeekView, sessions: boolean): ColumnId[] {
  // Nothing saved yet: the group's defaults stand, as they would for an entity with sessions, so Session is
  // not lost the first time someone changes columns on an entity without.
  const list = savedList(saved) ?? defaultColumns(view, true);
  const shown = new Set<ColumnId>(fillable(list, view, sessions));
  const out = [...next];
  list.forEach((id, i) => {
    if (shown.has(id) || out.includes(id)) return;
    // After the nearest column before it in the saved list that is in the result, else first.
    let at = 0;
    for (let j = i - 1; j >= 0; j--) {
      const k = out.indexOf(list[j]);
      if (k >= 0) {
        at = k + 1;
        break;
      }
    }
    out.splice(at, 0, id);
  });
  return out;
}

/** A property value as text: `""` when absent, `nullText` for an explicit null. The detail pane uses this one
 *  helper too, with `nullText = "null"`. */
export function propText(v: PropValue | undefined, nullText = ""): string {
  if (!v) return "";
  return v.type === "null" ? nullText : String(v.value);
}

/** Time to live: the largest whole units, `14 d`, `1 h 30 min`, `45 s`; the service's "never" (TimeSpan.MaxValue) as such. */
export function formatTtl(ms: number | null): string {
  if (ms == null) return "";
  if (ms >= 1e14) return "never";
  const units: [number, string][] = [[86_400_000, "d"], [3_600_000, "h"], [60_000, "min"], [1000, "s"]];
  const parts: string[] = [];
  let rest = ms;
  for (const [size, unit] of units) {
    const n = Math.floor(rest / size);
    if (n > 0 && parts.length < 2) parts.push(`${n} ${unit}`);
    if (parts.length > 0) rest -= n * size;
  }
  return parts.length ? parts.join(" ") : `${ms} ms`;
}

type Meta = ColumnDef<MessageSummary, unknown>["meta"];

const text = (id: ColumnId, header: string, get: (m: MessageSummary) => string | number | null, meta: Meta = {}): ColumnDef<MessageSummary, unknown> => ({
  id,
  header,
  accessorFn: (m) => get(m) ?? "",
  meta: { width: "minmax(120px, 1fr)", ...meta },
  cell: (c) => {
    const v = c.getValue();
    return v === "" ? null : <span title={String(v)}>{String(v)}</span>;
  },
});

/** A column whose sort key (the raw value) differs from what it shows. */
const shown = (col: ColumnDef<MessageSummary, unknown>, show: (m: MessageSummary) => string): ColumnDef<MessageSummary, unknown> => ({
  ...col,
  cell: (c) => {
    const s = show(c.row.original);
    return s === "" ? null : <span title={s}>{s}</span>;
  },
});

/** The Seq column's width for sequence numbers of `digits` digits (partitioned entities have 15 to 17): mono 11px
 *  digits are 6.6px wide, plus the cell padding; never narrower than the header and its sort arrow. */
export const seqWidth = (digits: number) => `${Math.max(56, Math.ceil(digits * 6.6) + 16)}px`;

/** How wide data-sized columns are: the longest sequence number's digits, and whether timestamps need their year. */
export interface ColumnFit { seqDigits: number; withYear: boolean }
const FIT: ColumnFit = { seqDigits: 5, withYear: false };

/** A timestamp column: compact local time in the cell, the service's full RFC 3339 value on hover; sorts by that value. */
const time = (id: ColumnId, header: string, get: (m: MessageSummary) => string | null, withYear: boolean): ColumnDef<MessageSummary, unknown> => ({
  id,
  header,
  accessorFn: (m) => get(m) ?? "",
  meta: { mono: true, width: withYear ? "150px" : "118px" },
  cell: (c) => {
    const iso = get(c.row.original);
    const shown = formatShortTimestamp(iso, withYear);
    return shown === "" ? null : <span title={iso!}>{shown}</span>;
  },
});

export function buildColumns(ids: ColumnId[], fit: ColumnFit = FIT): ColumnDef<MessageSummary, unknown>[] {
  return ids.map((id) => {
    if (id.startsWith("prop:")) {
      const name = id.slice(5);
      return text(id, name, (m) => propText(m.application_properties[name]), { mono: true, width: "minmax(100px, 160px)" });
    }
    switch (id as BuiltIn) {
      // Sequence numbers are identifiers: plain digits, no grouping.
      case "seq": return text(id, "Seq", (m) => m.sequence_number, { mono: true, width: seqWidth(fit.seqDigits) });
      case "enqueued": return time(id, "Enqueued", (m) => m.enqueued_time, fit.withYear);
      case "scheduled": return time(id, "Scheduled for", (m) => m.scheduled_enqueue_time, fit.withYear);
      case "subject": return text(id, "Subject", (m) => m.subject, { width: "minmax(110px, 160px)" });
      case "messageId": return text(id, "Message ID", (m) => m.message_id, { mono: true, width: "minmax(120px, 200px)" });
      case "correlationId": return text(id, "Correlation ID", (m) => m.correlation_id, { mono: true, width: "minmax(120px, 200px)" });
      case "session": return text(id, "Session", (m) => m.session_id, { mono: true, width: "minmax(90px, 140px)" });
      case "contentType": return text(id, "Content type", (m) => m.content_type, { width: "minmax(100px, 150px)" });
      case "to": return text(id, "To", (m) => m.to, { width: "minmax(80px, 140px)" });
      case "replyTo": return text(id, "Reply to", (m) => m.reply_to, { width: "minmax(80px, 140px)" });
      case "delivery": return text(id, "Deliveries", (m) => m.delivery_count, { align: "end", width: "76px" });
      case "size": return shown(text(id, "Size", (m) => m.body_size, { mono: true, align: "end", width: "76px" }), (m) => formatBytes(m.body_size));
      case "ttl": return shown(text(id, "Time to live", (m) => m.time_to_live_ms, { align: "end", width: "96px" }), (m) => formatTtl(m.time_to_live_ms));
      case "reason": return text(id, "Reason", (m) => m.dead_letter_reason, { width: "minmax(150px, 200px)" });
      case "description": return text(id, "Description", (m) => m.dead_letter_description, { width: "minmax(200px, 1fr)" });
      case "source": return text(id, "Dead-letter source", (m) => m.dead_letter_source, { width: "minmax(120px, 180px)" });
      case "state": return text(id, "State", (m) => m.state, { width: "80px" });
      case "preview":
        return {
          ...text(id, "Body", (m) => m.body_preview ?? "", { mono: true, width: "minmax(200px, 2fr)" }),
          cell: (c) => {
            const m = c.row.original;
            if (m.body_preview) return <span title={m.body_preview}>{m.body_preview}</span>;
            // No text preview: say what the body is, in the UI face (it is not the body itself).
            return m.body_kind === "empty" ? null : <span className="font-sans text-12 text-muted">{m.body_kind === "binary" ? "Binary body" : "No preview"}</span>;
          },
        };
    }
  });
}

/** Columns per view group, saved in the `columns` setting; unknown or bad values fall back to the defaults.
 *  Returns the columns, a setter, and `reset` (back to the defaults: the group's entry is removed). */
export function useColumnPrefs(view: PeekView, sessions: boolean): [ColumnId[], (ids: ColumnId[]) => void, () => void] {
  const q = useSetting("columns");
  const qc = useQueryClient();
  const group = groupOf(view);
  const saved = (q.data as Record<string, unknown> | null | undefined)?.[group];
  // Stable while nothing changes, so the grid's column definitions are not rebuilt every render.
  const ids = useMemo(() => parseColumns(saved, view, sessions), [saved, view, sessions]);
  const write = useCallback(
    (next: ColumnId[] | null) => {
      // Read the latest value from the cache so two quick changes compose.
      const current = qc.getQueryData(keys.setting("columns"));
      const all: Record<string, unknown> = { ...(current && typeof current === "object" && !Array.isArray(current) ? (current as Record<string, unknown>) : {}) };
      if (next) all[group] = mergeColumns(all[group], next, view, sessions);
      else delete all[group];
      qc.setQueryData(keys.setting("columns"), all);
      void commands.settingsSet("columns", all).catch(() => {});
    },
    [qc, group, view, sessions],
  );
  const set = useCallback((next: ColumnId[]) => write(next), [write]);
  const reset = useCallback(() => write(null), [write]);
  return [ids, set, reset];
}
