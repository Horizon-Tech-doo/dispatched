import { Braces, Columns3, Copy, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useMessageDetail, type MessageDetail, type PeekView } from "@/engine";
import {
  BodyState, Button, CodeView, EmptyState, HexView, Hint, IconButton, Segmented, SkeletonRows, Tabs, TabsContent, TabsList, TabsTrigger,
  formatBody, formatBytes,
} from "@/ui";
import { VIEW_LABEL } from "../tabs/subtabs";
import { formatTimestamp } from "../time";
import { propText } from "./columns";
import { bodyBytes, bodyText, toExportJson } from "./exportJson";

/** The service cuts displayed bodies here (see the engine's `message_detail`). */
const DETAIL_CAP = 4 * 1024 * 1024;
const DEBOUNCE_MS = 120;

type Mode = "formatted" | "text" | "hex";

type Row = [label: string, value: string | number | null | undefined];

function Rows({ label, rows }: { label: string; rows: Row[] }) {
  const shown = rows.filter(([, v]) => v !== null && v !== undefined && v !== "");
  return (
    <table aria-label={label} className="w-full border-collapse text-12">
      <tbody>
        {shown.map(([k, v]) => (
          <tr key={k} className="align-top">
            <th scope="row" className="w-[132px] py-0.5 pr-3 text-left font-normal text-muted">{k}</th>
            <td className="break-all py-0.5 font-mono text-11 tabular-nums text-fg" data-selectable>{String(v)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

const Note = ({ children }: { children: string }) => <p className="m-0 shrink-0 px-3 pb-1.5 text-11 text-muted">{children}</p>;

function Body({ d, text, bytes }: { d: MessageDetail; text: string | null; bytes: Uint8Array | null }) {
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
  const kind = fm && fm.format !== "text" ? fm.format.toUpperCase() : text !== null ? "Text" : "Binary";
  const view =
    mode === "hex" && bytes ? (
      <HexView bytes={bytes} label="Hex" />
    ) : (
      <CodeView text={mode === "formatted" && pretty !== null ? pretty : (text ?? "")} label="Body" />
    );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 shrink-0 items-center gap-2 px-3">
        {options.length > 1 && <Segmented label="Body view" value={mode} onChange={setMode} options={options} />}
        <span className="text-11 tabular-nums text-muted">{kind} · {formatBytes(d.body_size)}</span>
      </div>
      {d.body_truncated && (
        <Note>{`Showing the first ${formatBytes(DETAIL_CAP)} of ${formatBytes(d.body_size)}. Export the message for the whole body.`}</Note>
      )}
      {text === null && (
        <div className="shrink-0 px-3 pb-1.5"><BodyState kind="binary" /></div>
      )}
      {fm?.reason === "too-large" && (
        <div className="shrink-0 px-3 pb-1.5"><BodyState kind="too-large" /></div>
      )}
      <div className="min-h-0 flex-1 border-t border-line">{view}</div>
    </div>
  );
}

function Properties({ d, onShowColumn }: { d: MessageDetail; onShowColumn(name: string): void }) {
  const custom = Object.entries(d.application_properties);
  return (
    <div className="px-3 py-2">
      <h3 className="m-0 mb-1 text-12 font-semibold text-muted">System</h3>
      <Rows
        label="System properties"
        rows={[
          ["Message ID", d.message_id], ["Sequence number", d.sequence_number], ["State", d.state],
          ["Enqueued", formatTimestamp(d.enqueued_time)], ["Scheduled for", formatTimestamp(d.scheduled_enqueue_time)],
          ["Delivery count", d.delivery_count], ["Session ID", d.session_id], ["Correlation ID", d.correlation_id],
          ["Subject", d.subject], ["Content type", d.content_type], ["To", d.to], ["Reply to", d.reply_to],
          ["Time to live", d.time_to_live_ms != null ? `${d.time_to_live_ms} ms` : null], ["Locked until", formatTimestamp(d.locked_until)],
          ["Dead-letter reason", d.dead_letter_reason], ["Dead-letter description", d.dead_letter_description], ["Dead-letter source", d.dead_letter_source],
          ["Body size", formatBytes(d.body_size)],
        ]}
      />
      <h3 className="m-0 mb-1 mt-3 text-12 font-semibold text-muted">Custom</h3>
      {custom.length === 0 ? (
        <p className="m-0 text-12 text-muted">No custom properties.</p>
      ) : (
        <table aria-label="Custom properties" className="w-full border-collapse text-12">
          <tbody>
            {custom.map(([k, v]) => (
              <tr key={k} className="group/prop align-top">
                <th scope="row" className="w-[132px] break-all py-0.5 pr-3 text-left font-mono text-11 font-normal text-fg">{k}</th>
                <td className="w-[56px] py-0.5 pr-3 text-11 text-muted">{v.type}</td>
                <td className="break-all py-0.5 font-mono text-11 tabular-nums text-fg" data-selectable>{propText(v, "null")}</td>
                <td className="w-6 py-0.5 text-right">
                  <IconButton
                    label={`Show ${k} as a column`}
                    className="size-5 opacity-0 focus-visible:opacity-100 group-hover/prop:opacity-100"
                    onClick={() => onShowColumn(k)}
                  >
                    <Columns3 />
                  </IconButton>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

/** One message in full: the body (formatted, text or hex), its properties, and copy actions. Follows the focused row;
 *  `sequence` null means nothing is selected. */
export function DetailPane(p: {
  profileId: string | null;
  entity: string;
  view: PeekView;
  sequence: number | null;
  onClose(): void;
  onShowColumn(name: string): void;
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
  const text = useMemo(() => (d ? bodyText(d.body) : null), [d]);
  const bytes = useMemo(() => (d ? bodyBytes(d.body) : null), [d]);

  const [copied, setCopied] = useState("");
  const clear = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(clear.current), []);
  const copy = (what: string, s: string) => {
    const done = (msg: string) => {
      setCopied(msg);
      clearTimeout(clear.current);
      clear.current = setTimeout(() => setCopied(""), 2000);
    };
    if (!navigator.clipboard) return done("Couldn't copy");
    navigator.clipboard.writeText(s).then(() => done(`Copied ${what}`), () => done("Couldn't copy"));
  };

  const asBase64 = d?.body.kind === "binary" && text === null;
  const bodyLabel = asBase64 ? "Copy body as base64" : "Copy body";
  const empty = d != null && (d.body.kind === "empty" || text === "");
  // A body cut at 4 MiB is not the message: copying it would silently lose data. Export gets the whole body.
  const noCopy = !d || d.body_truncated;
  const dlq = p.view === "dead-letter" || p.view === "transfer-dead-letter";

  return (
    <section
      aria-label={p.sequence != null ? `Message ${p.sequence}` : "Message"}
      className="flex h-full min-h-0 flex-col bg-side"
      onKeyDown={(e) => {
        if (e.key === "Escape" && !e.defaultPrevented) p.onClose();
      }}
    >
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3">
        {p.sequence != null && <span className="font-mono text-11 tabular-nums">Seq {p.sequence}</span>}
        <span className="min-w-0 truncate text-11 text-muted">{label}</span>
        <span role="status" className="ml-auto shrink-0 text-11 text-muted">{copied}</span>
        <div className="flex shrink-0 items-center gap-1">
          <Hint label={bodyLabel}>
            <IconButton
              label={bodyLabel}
              disabled={noCopy || empty}
              onClick={() => d && copy("body", asBase64 && d.body.kind === "binary" ? d.body.value : (text ?? ""))}
            >
              <Copy />
            </IconButton>
          </Hint>
          <Hint label="Copy as JSON">
            <IconButton label="Copy as JSON" disabled={noCopy} onClick={() => d && copy("JSON", toExportJson(d))}>
              <Braces />
            </IconButton>
          </Hint>
          <IconButton label="Close message" onClick={p.onClose}>
            <X />
          </IconButton>
        </div>
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
        />
      ) : (
        <Tabs value={tab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col">
          {dlq && (d.dead_letter_reason || d.dead_letter_description) && (
            <div className="shrink-0 border-b border-line px-3 py-2">
              {d.dead_letter_reason && <p className="m-0 text-12 font-semibold" data-selectable>{d.dead_letter_reason}</p>}
              {d.dead_letter_description && <p className="m-0 text-12 text-muted" data-selectable>{d.dead_letter_description}</p>}
            </div>
          )}
          <TabsList variant="underline" aria-label="Message parts" className="shrink-0 border-b border-line">
            <TabsTrigger value="body">Body</TabsTrigger>
            <TabsTrigger value="properties">Properties</TabsTrigger>
          </TabsList>
          <TabsContent value="body" className="flex min-h-0 flex-1 flex-col pt-1">
            <Body key={d.sequence_number} d={d} text={text} bytes={bytes} />
          </TabsContent>
          <TabsContent value="properties" className="min-h-0 flex-1 overflow-y-auto">
            <Properties d={d} onShowColumn={p.onShowColumn} />
          </TabsContent>
        </Tabs>
      )}
    </section>
  );
}
