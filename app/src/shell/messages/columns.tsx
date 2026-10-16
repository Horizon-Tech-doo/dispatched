import { useQueryClient } from "@tanstack/react-query";
import type { ColumnDef } from "@tanstack/react-table";
import { useCallback, useMemo } from "react";
import { commands, keys, useSetting, type MessageSummary, type PeekView, type PropValue } from "@/engine";
import { formatBytes } from "../format";
import { formatTimestamp } from "../time";

export type ColumnId =
  | "seq" | "enqueued" | "scheduled" | "subject" | "messageId" | "correlationId" | "session" | "contentType" | "to" | "replyTo"
  | "delivery" | "size" | "ttl" | "reason" | "description" | "source" | "state" | "preview" | `prop:${string}`;

type BuiltIn = Exclude<ColumnId, `prop:${string}`>;

export const COLUMN_LABEL: Record<BuiltIn, string> = {
  seq: "Seq", enqueued: "Enqueued", scheduled: "Scheduled for", subject: "Subject", messageId: "Message ID", correlationId: "Correlation ID",
  session: "Session", contentType: "Content type", to: "To", replyTo: "Reply to", delivery: "Deliveries", size: "Size", ttl: "Time to live",
  reason: "Reason", description: "Description", source: "Dead-letter source", state: "State", preview: "Body",
};

/** The columns the Columns menu offers, in menu order (Seq is always shown: rows are identified by it). */
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

export const propColumn = (name: string): ColumnId => `prop:${name}`;
const isId = (v: unknown): v is ColumnId => typeof v === "string" && (v in COLUMN_LABEL || (v.startsWith("prop:") && v.length > 5));

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
export const seqWidth = (digits: number) => `${Math.max(56, Math.ceil(digits * 6.6) + 12)}px`;

export function buildColumns(ids: ColumnId[], seqDigits = 5): ColumnDef<MessageSummary, unknown>[] {
  return ids.map((id) => {
    if (id.startsWith("prop:")) {
      const name = id.slice(5);
      return text(id, name, (m) => propText(m.application_properties[name]), { mono: true, width: "minmax(100px, 160px)" });
    }
    switch (id as BuiltIn) {
      // Sequence numbers are identifiers: plain digits, no grouping.
      case "seq": return text(id, "Seq", (m) => m.sequence_number, { mono: true, align: "end", width: seqWidth(seqDigits) });
      case "enqueued": return text(id, "Enqueued", (m) => formatTimestamp(m.enqueued_time), { mono: true, width: "136px" });
      case "scheduled": return text(id, "Scheduled for", (m) => formatTimestamp(m.scheduled_enqueue_time), { mono: true, width: "136px" });
      case "subject": return text(id, "Subject", (m) => m.subject, { width: "minmax(110px, 160px)" });
      case "messageId": return text(id, "Message ID", (m) => m.message_id, { mono: true, width: "minmax(120px, 180px)" });
      case "correlationId": return text(id, "Correlation ID", (m) => m.correlation_id, { mono: true, width: "minmax(120px, 200px)" });
      case "session": return text(id, "Session", (m) => m.session_id, { mono: true, width: "minmax(90px, 140px)" });
      case "contentType": return text(id, "Content type", (m) => m.content_type, { width: "minmax(100px, 150px)" });
      case "to": return text(id, "To", (m) => m.to, { width: "minmax(80px, 140px)" });
      case "replyTo": return text(id, "Reply to", (m) => m.reply_to, { width: "minmax(80px, 140px)" });
      case "delivery": return text(id, "Deliveries", (m) => m.delivery_count, { align: "end", width: "76px" });
      case "size": return shown(text(id, "Size", (m) => m.body_size, { mono: true, align: "end", width: "76px" }), (m) => formatBytes(m.body_size));
      case "ttl": return shown(text(id, "Time to live", (m) => m.time_to_live_ms, { align: "end", width: "96px" }), (m) => formatTtl(m.time_to_live_ms));
      case "reason": return text(id, "Reason", (m) => m.dead_letter_reason, { width: "minmax(150px, 200px)" });
      case "description": return text(id, "Description", (m) => m.dead_letter_description, { width: "minmax(180px, 1fr)" });
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

/** Columns per view group, saved in the `columns` setting; unknown or bad values fall back to the defaults. */
export function useColumnPrefs(view: PeekView, sessions: boolean): [ColumnId[], (ids: ColumnId[]) => void] {
  const q = useSetting("columns");
  const qc = useQueryClient();
  const group = groupOf(view);
  const saved = (q.data as Record<string, unknown> | null | undefined)?.[group];
  // Stable while nothing changes, so the grid's column definitions are not rebuilt every render.
  const ids = useMemo(
    () => (Array.isArray(saved) && saved.length > 0 && saved.every(isId) ? (saved as ColumnId[]) : defaultColumns(view, sessions)),
    [saved, view, sessions],
  );
  const set = useCallback(
    (next: ColumnId[]) => {
      // Read the latest value from the cache so two quick changes compose.
      const current = qc.getQueryData(keys.setting("columns"));
      const all = { ...(current && typeof current === "object" && !Array.isArray(current) ? (current as Record<string, unknown>) : {}), [group]: next };
      qc.setQueryData(keys.setting("columns"), all);
      void commands.settingsSet("columns", all).catch(() => {});
    },
    [qc, group],
  );
  return [ids, set];
}
