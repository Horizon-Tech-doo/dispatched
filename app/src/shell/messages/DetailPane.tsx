import { Columns3, Copy, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useMessageDetail, type MessageDetail, type PeekView } from "@/engine";
import {
  BodyState, Button, CodeView, EmptyState, HexView, Hint, IconButton, Segmented, SkeletonRows, Tabs, TabsContent, TabsList, TabsTrigger,
  formatBody, formatBytes,
} from "@/ui";
import { VIEW_LABEL } from "../tabs/subtabs";
import { formatTimestamp } from "../time";
import { formatTtl } from "../duration";
import { propText } from "./columns";
import { decodeBody, toExportJson } from "./exportJson";

const DEBOUNCE_MS = 120;
const COPIED_MS = 2000;
const TRUNCATED = "The body is cut at 4 MiB. Export the message to copy all of it.";

type Mode = "formatted" | "text" | "hex";

// ---- the one property table (System and Custom share it) ----

interface PropRow {
  key: string;
  /** What is shown; null for a property with no value (shown as a faint "none"). */
  value: string | null;
  /** Shown whole on hover (the full timestamp, the milliseconds of a duration). */
  title?: string;
  /** The AMQP type, faint, after the value. */
  type?: string;
  /** Custom property names are machine text. */
  mono?: boolean;
  /** A button over the end of the row. */
  action?: ReactNode;
}

/** Key column `keyCh` wide (the longest key of both tables, never more than 40% of the table), values to its right on one line each,
 *  cut with an ellipsis and whole in the tooltip (and selectable). Rows keep the grid's row height.
 *  A CSS grid with table roles: `min()` sizes the key column, which a table's `col` cannot take. */
function PropTable({ label, rows, keyCh }: { label: string; rows: PropRow[]; keyCh: number }) {
  return (
    // Both tables use the same font here, so one `ch` is one width: the System and Custom values line up.
    <div role="table" aria-label={label} className="grid text-12" style={{ gridTemplateColumns: `minmax(0, min(${keyCh}ch, 40%)) minmax(0, 1fr)` }}>
      {rows.map((r) => (
        <div key={r.key} role="row" className="group/prop contents">
          <div role="rowheader" title={r.key} className="flex h-row items-center pr-3 text-muted">
            <span className={`truncate ${r.mono ? "font-mono text-11" : ""}`}>{r.key}</span>
          </div>
          <div role="cell" className="relative flex h-row min-w-0 items-center gap-2 font-sans">
            {r.value === null ? (
              <span className="text-faint">none</span>
            ) : (
              <span title={r.title ?? r.value} className="min-w-0 truncate font-mono text-11 tabular-nums text-fg" data-selectable>
                {r.value}
              </span>
            )}
            {r.type && <span className="shrink-0 text-11 text-faint">{r.type}</span>}
            {r.action && (
              <span className="absolute inset-y-0 right-0 flex items-center bg-side pl-1 opacity-0 focus-within:opacity-100 group-hover/prop:opacity-100">
                {r.action}
              </span>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

const orNone = (v: string | number | null | undefined) => (v === null || v === undefined || v === "" ? null : String(v));

function systemRows(d: MessageDetail): PropRow[] {
  const when = (iso: string | null): Pick<PropRow, "value" | "title"> => ({ value: orNone(formatTimestamp(iso)), title: iso ?? undefined });
  const ttl = d.time_to_live_ms;
  const rows: PropRow[] = [
    // Identity: always listed, so an absent id reads as absent.
    { key: "Message ID", value: orNone(d.message_id) },
    { key: "Sequence number", value: String(d.sequence_number) },
    { key: "Correlation ID", value: orNone(d.correlation_id) },
    { key: "Session ID", value: orNone(d.session_id) },
    { key: "Subject", value: orNone(d.subject) },
    { key: "Content type", value: orNone(d.content_type) },
    // Lifetime: always listed.
    { key: "State", value: d.state },
    { key: "Enqueued", ...when(d.enqueued_time) },
    { key: "Scheduled for", ...when(d.scheduled_enqueue_time) },
    { key: "Expires at", ...when(d.expires_at) },
    { key: "Time to live", value: ttl != null ? formatTtl(ttl) : null, title: ttl != null ? `${ttl} ms` : undefined },
    { key: "Delivery count", value: String(d.delivery_count) },
  ];
  // The rest only when set.
  const optional: [string, string | null][] = [
    ["To", orNone(d.to)], ["Reply to", orNone(d.reply_to)], ["Reply to session ID", orNone(d.reply_to_session_id)], ["Partition key", orNone(d.partition_key)],
  ];
  for (const [key, value] of optional) if (value !== null) rows.push({ key, value });
  rows.push({ key: "Body size", value: formatBytes(d.body_size) });
  return rows;
}

/** A custom property's row. The dead-letter reason and description are properties too: they are the header's, so they
 *  are not listed again when they say the same. */
function customRows(d: MessageDetail, shown: ReadonlySet<string>, onShowColumn: (name: string) => void): PropRow[] {
  return Object.entries(d.application_properties)
    .filter(([k, v]) => !(k === "DeadLetterReason" && propText(v) === d.dead_letter_reason) && !(k === "DeadLetterErrorDescription" && propText(v) === d.dead_letter_description))
    .map(([k, v]) => ({
      key: k,
      value: propText(v, "null"),
      type: v.type,
      mono: true,
      action: shown.has(k) ? null : (
        <IconButton label={`Show ${k} as a column`} className="size-5" onClick={() => onShowColumn(k)}>
          <Columns3 />
        </IconButton>
      ),
    }));
}

function Properties({ d, shown, onShowColumn }: { d: MessageDetail; shown: ReadonlySet<string>; onShowColumn(name: string): void }) {
  const custom = customRows(d, shown, onShowColumn);
  const system = systemRows(d);
  // One key column for both tables.
  const keyCh = Math.max(...[...system, ...custom].map((r) => r.key.length), 4) + 2;
  return (
    <div className="px-3 py-2">
      <h3 className="m-0 mb-1 text-12 font-semibold">System</h3>
      <PropTable label="System properties" rows={system} keyCh={keyCh} />
      <h3 className="m-0 mb-1 mt-3 text-12 font-semibold">Custom</h3>
      {custom.length === 0 ? <p className="m-0 text-12 text-muted">No custom properties.</p> : <PropTable label="Custom properties" rows={custom} keyCh={keyCh} />}
    </div>
  );
}

// ---- copy ----

/** A text button that never leaves the tab order: when it cannot act it says so in its tooltip (`aria-disabled`, no-op),
 *  which a `disabled` button could not show. */
function CopyButton({ label, hint, reason, onCopy }: { label: string; hint: string; reason: string | null; onCopy(): void }) {
  return (
    <Hint label={reason ?? hint}>
      <Button
        size="sm"
        aria-disabled={reason !== null ? true : undefined}
        className={reason !== null ? "opacity-45" : undefined}
        onClick={() => reason === null && onCopy()}
      >
        <Copy aria-hidden="true" />
        {label}
      </Button>
    </Hint>
  );
}

// ---- body ----

function Body({
  d, text, bytes, onExport, onCopy,
}: {
  d: MessageDetail;
  text: string | null;
  bytes: Uint8Array | null;
  onExport?: () => void;
  onCopy(as: "text" | "base64"): void;
}) {
  // The formatted view is for reading only; copy always uses `text`.
  const fm = useMemo(() => (text ? formatBody(text, d.content_type) : null), [text, d.content_type]);
  const pretty = fm?.formatted ? fm.text : null;
  const [mode, setMode] = useState<Mode>(pretty !== null ? "formatted" : text !== null ? "text" : "hex");

  if (d.body.kind === "empty" || text === "") {
    return <div className="px-3 py-2"><BodyState kind="empty" /></div>;
  }
  if (text === null && bytes === null) {
    return <p className="m-0 px-3 py-2 text-12 text-muted">This body could not be decoded. Export the message to inspect it.</p>;
  }

  const options: { value: Mode; label: string }[] = [
    ...(pretty !== null ? [{ value: "formatted" as const, label: "Formatted" }] : []),
    ...(text !== null ? [{ value: "text" as const, label: "Text" }] : []),
    ...(bytes ? [{ value: "hex" as const, label: "Hex" }] : []),
  ];
  const meta =
    text === null
      ? d.body_truncated ? "Binary, shown as hex" : "Binary, not UTF-8"
      : fm && fm.format !== "text" ? fm.format.toUpperCase() : "Text";
  const view =
    mode === "hex" && bytes ? (
      <HexView bytes={bytes} label="Hex" />
    ) : (
      <CodeView text={mode === "formatted" && pretty !== null ? pretty : (text ?? "")} label="Body" />
    );
  const shown = bytes ? bytes.length : (text?.length ?? 0);
  // A binary body that is not UTF-8 has no text to copy: its bytes go as base64 (the form the engine sends them in).
  const base64 = text === null && d.body.kind === "binary";
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 px-3">
        {options.length > 1 && <Segmented label="Body view" value={mode} onChange={setMode} options={options} />}
        <span className="min-w-0 truncate text-11 tabular-nums text-muted">{meta} · {formatBytes(d.body_size)}</span>
        <span className="ml-auto shrink-0">
          {base64 ? (
            <CopyButton label="Copy body as base64" hint="Copy the body as base64" reason={d.body_truncated ? TRUNCATED : null} onCopy={() => onCopy("base64")} />
          ) : (
            <CopyButton
              label="Copy body"
              hint={mode === "formatted" ? "Copy the original text, not the indented view" : "Copy the body as text"}
              reason={d.body_truncated ? TRUNCATED : null}
              onCopy={() => onCopy("text")}
            />
          )}
        </span>
      </div>
      {d.body_truncated && (
        <p className="m-0 flex shrink-0 flex-wrap items-center gap-x-2 gap-y-1 px-3 pb-1.5 text-12 text-fg">
          <span>Showing the first {formatBytes(shown)} of {formatBytes(d.body_size)}.{onExport ? "" : " Export the message for the whole body."}</span>
          {onExport && <Button size="sm" onClick={onExport}>Export message</Button>}
        </p>
      )}
      {fm?.reason === "too-large" && (
        <div className="shrink-0 px-3 pb-1.5"><BodyState kind="too-large" /></div>
      )}
      <div className="min-h-0 flex-1 border-t border-line">{view}</div>
    </div>
  );
}

// ---- the pane ----

/** One message in full: the body (formatted, text or hex), its properties, and copy actions. Follows the focused row;
 *  `sequence` null means nothing is selected. `onExport` (when given) is offered for a body that was cut;
 *  `onPeekAgain` for a message that has gone; `shownProps` are the custom properties already shown as grid columns. */
export function DetailPane(p: {
  profileId: string | null;
  entity: string;
  view: PeekView;
  sequence: number | null;
  onClose(): void;
  onShowColumn(name: string): void;
  shownProps?: readonly string[];
  onExport?(sequence: number): void;
  onPeekAgain?(): void;
}) {
  // Holding an arrow key walks the grid faster than messages can be read: the pane waits for the focus to rest.
  const [debounced, setDebounced] = useState(p.sequence);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(p.sequence), DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [p.sequence]);
  const settled = debounced === p.sequence;
  const q = useMessageDetail(p.profileId, p.entity, p.view, debounced);
  const d = settled ? q.data : undefined;
  const label = VIEW_LABEL[p.view];
  const [tab, setTab] = useState("body");
  // The body is decoded once; copy, the views and the export shape all read this.
  const { bytes, text } = useMemo(() => (d ? decodeBody(d.body, d.body_truncated) : { bytes: null, text: null }), [d]);
  const shownProps = useMemo(() => new Set(p.shownProps ?? []), [p.shownProps]);

  const [copied, setCopied] = useState("");
  const clear = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(clear.current), []);
  const copy = (what: string, s: string) => {
    const done = (msg: string) => {
      setCopied(msg);
      clearTimeout(clear.current);
      clear.current = setTimeout(() => setCopied(""), COPIED_MS);
    };
    if (!navigator.clipboard) return done("Couldn't copy");
    navigator.clipboard.writeText(s).then(() => done(`Copied ${what}`), () => done("Couldn't copy"));
  };

  // Why a copy action cannot act, in the words its tooltip uses.
  const unavailable =
    p.sequence == null ? "Select a message first."
      : !settled || q.isPending ? "The message is loading."
        : q.isError ? "The message could not be read."
          : !d ? `This message is no longer in the ${label}.`
            : null;
  const jsonReason = unavailable ?? (d?.body_truncated ? TRUNCATED : null);
  const dlq = p.view === "dead-letter" || p.view === "transfer-dead-letter";
  const source = d?.dead_letter_source;

  return (
    <section
      aria-label={p.sequence != null ? `Message ${p.sequence}` : "Message"}
      className="flex h-full min-h-0 flex-col bg-side"
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) p.onClose();
      }}
    >
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3">
        <h2 className="m-0 flex min-w-0 items-baseline gap-2 text-11 font-normal">
          {p.sequence != null && <span className="font-mono tabular-nums">Seq {p.sequence}</span>}
          <span className="truncate text-muted">{label}</span>
        </h2>
        <span role="status" className="ml-auto shrink-0 text-11 text-muted">{copied}</span>
        <CopyButton label="Copy JSON" hint="Copy the message as JSON, the way an export writes it" reason={jsonReason} onCopy={() => d && copy("JSON", toExportJson(d))} />
        <IconButton label="Close message" onClick={p.onClose}>
          <X />
        </IconButton>
      </div>
      {p.sequence == null ? (
        <EmptyState title="No message selected" body="Select a row to read it here." />
      ) : !settled || q.isPending ? (
        <SkeletonRows rows={8} />
      ) : q.isError ? (
        <EmptyState
          tone="error"
          title="Couldn't read this message"
          body="Select it again, or try once more."
          detail={q.error.message}
          action={<Button onClick={() => void q.refetch()}>Try again</Button>}
        />
      ) : !d ? (
        <EmptyState
          title={`This message is no longer in the ${label}`}
          body="It may have been received, expired or moved since it was peeked. Peek again to see what is there now."
          action={p.onPeekAgain && <Button onClick={p.onPeekAgain}>Peek again</Button>}
        />
      ) : (
        <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col">
          {dlq && (d.dead_letter_reason || d.dead_letter_description || source) && (
            <div className="shrink-0 border-b border-line px-3 py-2">
              {d.dead_letter_reason && <h3 className="m-0 text-12 font-semibold" data-selectable>{d.dead_letter_reason}</h3>}
              {d.dead_letter_description && <p className="m-0 text-12 text-muted" data-selectable>{d.dead_letter_description}</p>}
              {source && <p className="m-0 text-11 text-muted">Dead-lettered from <span className="font-mono" data-selectable>{source}</span></p>}
            </div>
          )}
          <TabsList variant="underline" aria-label="Message parts" className="shrink-0 border-b border-line [--tabs-fade:var(--c-side)]">
            <TabsTrigger value="body">Body</TabsTrigger>
            <TabsTrigger value="properties">Properties</TabsTrigger>
          </TabsList>
          <TabsContent value="body" className="flex min-h-0 flex-1 flex-col pt-1">
            <Body
              key={d.sequence_number}
              d={d}
              text={text}
              bytes={bytes}
              onExport={p.onExport ? () => p.onExport!(d.sequence_number) : undefined}
              onCopy={(as) => d && copy("body", as === "base64" && d.body.kind === "binary" ? d.body.value : (text ?? ""))}
            />
          </TabsContent>
          <TabsContent value="properties" className="min-h-0 flex-1 overflow-y-auto">
            <Properties d={d} shown={shownProps} onShowColumn={p.onShowColumn} />
          </TabsContent>
        </Tabs>
      )}
    </section>
  );
}
