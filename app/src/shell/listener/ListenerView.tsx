import { createColumnHelper } from "@tanstack/react-table";
import { useEffect, useMemo, useState } from "react";
import { commands, type ListenMode, type PeekView, type ReceivedView, type SubQueue } from "@/engine";
import { Button, DataGrid, Field, Input, Problem, Segmented, formatBytes, formatClock, formatNumber } from "@/ui";
import { useShell } from "../context";
import { displayName } from "../entityId";
import { DetailPane } from "../messages/DetailPane";
import { ConfirmPanel, answerFor, canConfirm } from "../operate/ConfirmPanel";
import { sendTarget } from "../operate/controls";
import { useOperation } from "../operate/useOperation";
import { confirmLabel, endedText, neutralEnd } from "../operate/words";
import type { EntityTab } from "../state";
import { RateChart } from "./RateChart";
import { ratesOf, tabKey, tabListener, useListened, useTabListener } from "./listenerStore";

const MODE_HINT: Record<ListenMode, string> = {
  watch: "Shows new messages and leaves them: nothing is locked or removed.",
  complete: "Locks each new message and completes it: it is removed from the entity, and kept here for the report.",
  delete: "Receives and deletes each new message at once: it is removed, and kept here for the report.",
};
const VIEW_OF: Record<SubQueue, PeekView> = { active: "active", "dead-letter": "dead-letter", "transfer-dead-letter": "transfer-dead-letter" };

const col = createColumnHelper<ReceivedView>();
const COLUMNS = [
  col.accessor("sequence_number", { header: "Seq", meta: { width: "minmax(72px, 112px)", mono: true, align: "end" } }),
  col.accessor((m) => (m.enqueued_time ? formatClock(m.enqueued_time) : ""), { id: "enqueued", header: "Enqueued", meta: { width: "84px", mono: true } }),
  col.accessor((m) => m.message_id ?? "", { id: "message_id", header: "Message ID", meta: { width: "minmax(120px, 200px)", mono: true } }),
  col.accessor((m) => m.subject ?? "", { id: "subject", header: "Subject", meta: { width: "minmax(100px, 160px)" } }),
  col.accessor((m) => formatBytes(m.body_size), { id: "size", header: "Size", meta: { width: "72px", align: "end" } }),
  col.accessor((m) => m.body_preview ?? "", { id: "preview", header: "Body", meta: { width: "minmax(160px, 1fr)", mono: true } }),
];

/** A clock that ticks every second while `on`, so the rate chart moves on when nothing arrives. */
function useSeconds(on: boolean): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    if (!on) return;
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(t);
  }, [on]);
  return on ? now : Math.floor(Date.now() / 1000);
}

/**
 * The live receive stream of a queue or subscription. Watch shows new messages and leaves them; the two removing modes
 * confirm first (on production with the entity's name) and keep every message they removed for the report and for
 * "Send again". It runs until Stop, or until the tab closes. The mode always starts at Watch.
 */
export function ListenerView({ tab, requiresSession }: { tab: EntityTab; requiresSession: boolean }) {
  const { conn, openSheet } = useShell();
  const key = tabKey(conn.profileId, tab.id);
  const following = useTabListener(key);
  const op = useOperation(following);
  const [mode, setMode] = useState<ListenMode>("watch");
  const [subQueue, setSubQueue] = useState<SubQueue>("active");
  const [sessions, setSessions] = useState<"each" | "id">("each");
  const [session, setSession] = useState("");
  const [receivers, setReceivers] = useState("1");
  const [prefetch, setPrefetch] = useState("0");
  const [typed, setTyped] = useState({ id: "", text: "" });
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);

  // The tab remembers its listener, so it is there again after a visit to another sub-tab.
  useEffect(() => {
    if (op.jobId && op.jobId !== following) tabListener.set(key, op.jobId);
  }, [op.jobId, following, key]);
  // Once it runs, leaving the sub-tab does not stop it: only Stop, closing the tab or the connection does.
  useEffect(() => {
    if (op.stage === "running") op.detach();
  }, [op.stage, op]);

  const listened = useListened(op.jobId);
  const job = op.job;
  const running = op.stage === "running";
  const now = useSeconds(running);
  const rates = useMemo(() => (listened ? ratesOf(listened, now) : null), [listened, now]);
  const name = displayName(tab.id);
  const removing = job ? !job.title.startsWith("Watch") : mode !== "watch";
  const view = VIEW_OF[subQueue];

  const n = Number(receivers);
  const p = Number(prefetch);
  const formIssue =
    mode !== "watch" && !(Number.isInteger(n) && n >= 1 && n <= 8)
      ? "Use 1 to 8 receivers."
      : mode === "delete" && !(Number.isInteger(p) && p >= 0 && p <= 100)
        ? "Prefetch is 0 to 100."
        : requiresSession && sessions === "id" && !session.trim()
          ? "Name the session."
          : null;
  const start = () =>
    void op.start(() =>
      commands.opListen(conn.profileId!, {
        entity: tab.id,
        sub_queue: subQueue,
        mode,
        sessions: !requiresSession ? { type: "none" } : sessions === "each" ? { type: "each" } : { type: "id", id: session.trim() },
        concurrency: mode === "watch" ? 1 : n,
        prefetch: mode === "delete" ? p : 0,
      }),
    );
  const fresh = () => {
    tabListener.set(key, null);
    op.reset();
    setSelected(new Set());
    setFocus(null);
  };

  const rows = listened?.rows ?? [];
  const picked = rows.filter((r) => selected.has(r.sequence_number));
  const sendAgain = async () => {
    const m = picked[0];
    if (!m || !op.jobId) return;
    const d = await commands.jobMessage(op.jobId, m.sequence_number);
    if (d) openSheet({ kind: "operation", op: "send", entity: sendTarget(tab.id), clone: d });
  };
  const onRows = (op2: "delete" | "move" | "resubmit") =>
    openSheet({ kind: "operation", op: op2, entity: tab.id, view, sequences: picked.map((r) => Number(r.sequence_number)), filter: {} });

  const ask = op.stage === "confirming" ? (job?.confirm ?? null) : null;
  const text = ask && typed.id === ask.id ? typed.text : "";

  return (
    <section aria-label="Listener" className="flex h-full min-h-0 flex-col">
      {op.stage === "form" ? (
        <div className="flex max-w-[560px] flex-col gap-4 overflow-y-auto px-3.5 py-3">
          {op.failure && <Problem role="alert" title="Couldn't start the listener" body="Nothing was received." detail={op.failure.message} />}
          <div className="flex flex-col gap-1.5">
            <span className="text-12 font-semibold text-muted">Mode</span>
            <Segmented<ListenMode>
              label="Mode"
              value={mode}
              onChange={setMode}
              options={[
                { value: "watch", label: "Watch" },
                { value: "complete", label: "Receive and complete" },
                { value: "delete", label: "Receive and delete" },
              ]}
            />
            <p className="m-0 text-12 text-muted">{MODE_HINT[mode]}</p>
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="text-12 font-semibold text-muted">From</span>
            <Segmented<SubQueue>
              label="Sub-queue"
              value={subQueue}
              onChange={setSubQueue}
              options={[
                { value: "active", label: "Active" },
                { value: "dead-letter", label: "Dead-letter" },
                { value: "transfer-dead-letter", label: "Transfer DLQ" },
              ]}
            />
          </div>
          {requiresSession && (
            <div className="flex flex-col gap-1.5">
              <span className="text-12 font-semibold text-muted">Sessions</span>
              <Segmented<"each" | "id">
                label="Sessions"
                value={sessions}
                onChange={setSessions}
                options={[
                  { value: "each", label: "Each session in turn" },
                  { value: "id", label: "One session" },
                ]}
              />
              {sessions === "id" && (
                <Field label="Session ID" inline={96}>
                  {(ids) => <Input {...ids} mono value={session} onChange={(e) => setSession(e.target.value)} />}
                </Field>
              )}
            </div>
          )}
          {mode !== "watch" && (
            <div className="flex gap-4">
              <Field label="Receivers" hint="1 to 8 at once.">
                {(ids) => <Input {...ids} mono inputMode="numeric" className="w-20" value={receivers} onChange={(e) => setReceivers(e.target.value)} />}
              </Field>
              {mode === "delete" && (
                <Field label="Prefetch" hint="Messages each receiver takes ahead, 0 to 100.">
                  {(ids) => <Input {...ids} mono inputMode="numeric" className="w-20" value={prefetch} onChange={(e) => setPrefetch(e.target.value)} />}
                </Field>
              )}
            </div>
          )}
          {formIssue && <p className="m-0 text-12 text-fg">{formIssue}</p>}
          <div>
            <Button variant="primary" disabled={!conn.profileId || !!formIssue || op.starting} aria-busy={op.starting} onClick={start}>
              {mode === "watch" ? "Start watching" : "Start listening"}
            </Button>
          </div>
        </div>
      ) : ask ? (
        <div className="flex max-w-[560px] flex-col gap-3 px-3.5 py-3">
          <ConfirmPanel key={ask.id} request={ask} typed={text} onTyped={(t) => setTyped({ id: ask.id, text: t })} />
          <div className="flex gap-2">
            <Button onClick={() => op.cancel()}>Cancel</Button>
            <Button variant="primary" disabled={!canConfirm(ask, text)} onClick={() => void op.answer(answerFor(ask, text))}>
              {confirmLabel(ask.verb, ask.target, ask.step)}
            </Button>
          </div>
        </div>
      ) : (
        <>
          <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-3.5 py-2">
            <div className="flex min-w-0 flex-col">
              <span className="text-12 font-semibold">{job?.title ?? `Listening to ${name}`}</span>
              <span role="status" className="text-11 tabular-nums text-muted">
                {formatNumber(listened?.total ?? 0)} received
                {listened ? ` · ${formatBytes(listened.bytes)}` : ""}
                {running ? "" : job?.finished ? ` · ${job.error && !neutralEnd(job.error.kind) ? job.error.message : (endedText(job) ?? job.summary ?? "Stopped")}` : ""}
              </span>
            </div>
            {rates && <RateChart messages={rates.messages} bytes={rates.bytes} />}
            <div className="ml-auto flex items-center gap-2">
              {running ? (
                <Button variant="primary" onClick={() => op.cancel()}>
                  Stop
                </Button>
              ) : (
                <>
                  {job?.report && (
                    <Button
                      onClick={async () => {
                        const pick = await commands.pickReportPath(op.jobId!, "json");
                        if (pick) await commands.jobReportSave(op.jobId!, pick.pick, "json");
                      }}
                    >
                      Save report…
                    </Button>
                  )}
                  <Button variant="primary" onClick={fresh}>
                    New listener
                  </Button>
                </>
              )}
            </div>
          </div>
          <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-3.5">
            {removing ? (
              <Button disabled={picked.length !== 1} onClick={() => void sendAgain()}>
                Send again…
              </Button>
            ) : (
              <>
                <Button disabled={!picked.length} onClick={() => onRows("resubmit")}>
                  Resubmit…
                </Button>
                <Button disabled={!picked.length} onClick={() => onRows("move")}>
                  Move…
                </Button>
                <Button disabled={!picked.length} onClick={() => onRows("delete")}>
                  Delete…
                </Button>
              </>
            )}
            <span className="text-11 text-muted">
              {removing ? "These messages were removed from the entity; the report keeps them all." : "Watched messages are still on the entity."} The grid shows the newest 1,000.
            </span>
          </div>
          <div className="flex min-h-0 flex-1">
            <div className="min-w-0 flex-1">
              <DataGrid<ReceivedView>
                label="Messages received"
                columns={COLUMNS}
                rows={rows}
                getRowId={(r) => r.sequence_number}
                selectedIds={selected}
                focusId={focus}
                onSelectionChange={(ids, f) => {
                  setSelected(ids);
                  setFocus(f);
                }}
                rowHeight={24}
                initialRect={{ width: 900, height: 600 }}
                empty={<p className="m-0 p-3 text-12 text-muted">{running ? "Waiting for messages…" : "No messages arrived."}</p>}
              />
            </div>
            {focus && picked.length === 1 && (
              <div className="w-[420px] shrink-0 border-l border-line">
                <DetailPane
                  profileId={conn.profileId}
                  entity={tab.id}
                  view={view}
                  sequence={Number(focus)}
                  jobId={removing ? op.jobId : null}
                  onClose={() => setFocus(null)}
                  onShowColumn={() => {}}
                />
              </div>
            )}
          </div>
        </>
      )}
    </section>
  );
}
