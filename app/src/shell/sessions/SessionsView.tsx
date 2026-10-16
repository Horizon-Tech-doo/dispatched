import { useQueryClient } from "@tanstack/react-query";
import type { ColumnDef } from "@tanstack/react-table";
import { useMemo, useState } from "react";
import { useSessions, type SessionListing, type SessionSummary } from "@/engine";
import { Button, DataGrid, EmptyState, Input, formatNumber, rowHeight, type GridSort } from "@/ui";
import { useShell } from "../context";
import { peekSession } from "../messages/peekSession";
import type { EntityTab } from "../state";
import { formatShortTimestamp, yearOf } from "../time";

/** Where the session list came from and what that means for it: the sentences that go in the status line. */
export function sessionNote(l: Pick<SessionListing, "broker" | "scanned" | "truncated">): string[] {
  const n = formatNumber(l.scanned);
  switch (l.broker.state) {
    case "listed":
      return [l.broker.capped ? "The service's list was cut at 10,000 sessions." : null, l.truncated ? `counts from the first ${n} messages are lower bounds` : null].filter((x): x is string => x !== null);
    // The sessions come from the tally alone: when it was cut short, sessions further on may be missing as well.
    case "empty":
      return [`The service listed no sessions; showing sessions found in the first ${n} messages.${l.truncated ? " Some sessions may be missing." : ""}`, ...(l.truncated ? ["counts are lower bounds"] : [])];
    case "unavailable":
      return [`Couldn't ask the service for sessions; showing sessions found by peeking.${l.truncated ? " Some sessions may be missing." : ""}`, ...(l.truncated ? ["counts are lower bounds"] : [])];
  }
}

const plural = (n: number, one: string) => `${formatNumber(n)} ${n === 1 ? one : `${one}s`}`;

/** A timestamp column as the message grid draws one: compact local time, the full value on hover; sorts by the full value. */
const time = (id: string, header: string, get: (s: SessionSummary) => string | null, withYear: boolean): ColumnDef<SessionSummary, unknown> => ({
  id,
  header,
  accessorFn: (s) => get(s) ?? "",
  meta: { mono: true, width: withYear ? "150px" : "118px" },
  cell: (c) => {
    const iso = get(c.row.original);
    const shown = formatShortTimestamp(iso, withYear);
    return shown === "" ? null : <span title={iso!}>{shown}</span>;
  },
});

export function SessionsView({ tab, requiresSession }: { tab: EntityTab; requiresSession: boolean | null }) {
  const { conn, prefs, dispatch } = useShell();
  const qc = useQueryClient();
  const q = useSessions(conn.profileId, tab.id, requiresSession !== false);
  const [filter, setFilter] = useState("");
  const [sorting, setSorting] = useState<GridSort | null>(null);
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  const truncated = q.data?.truncated ?? false;
  // The year shows only when a session is from another year; the columns keep one width.
  const withYear = useMemo(() => {
    const year = new Date().getFullYear();
    return (q.data?.sessions ?? []).some((x) => [x.first_enqueued, x.last_enqueued].some((t) => { const y = yearOf(t); return y !== null && y !== year; }));
  }, [q.data]);
  const columns = useMemo<ColumnDef<SessionSummary, unknown>[]>(
    () => [
      { id: "id", header: "Session", accessorFn: (s) => s.id, meta: { mono: true, width: "minmax(160px, 280px)" } },
      {
        id: "messages",
        header: "Messages",
        accessorFn: (s) => s.messages ?? -1,
        meta: { align: "end", width: "96px" },
        // A lower bound when the tally was cut short; unknown when no tally ran.
        cell: (c) => {
          const { messages: n, in_broker } = c.row.original;
          if (n == null) return "?";
          if (truncated) return `~${formatNumber(n)}+`;
          // A session the service lists without a message in the queue: say why it is here.
          return n === 0 && in_broker ? <span title="No messages in the queue. The service still lists this session.">0</span> : formatNumber(n);
        },
      },
      time("first", "First enqueued", (x) => x.first_enqueued, withYear),
      time("last", "Last enqueued", (x) => x.last_enqueued, withYear),
      {
        id: "listed",
        header: "Listed by service",
        accessorFn: (s) => (s.in_broker ? 1 : 0),
        // The last column takes the spare width. A session only the tally found has a blank cell, named for screen readers.
        meta: { width: "minmax(130px, 1fr)" },
        cell: (c) => (c.row.original.in_broker ? <span className="text-muted">Yes</span> : <span className="sr-only">Not listed by the service</span>),
      },
    ],
    [truncated, withYear],
  );
  const needle = filter.trim().toLowerCase();
  const rows = useMemo(() => (q.data?.sessions ?? []).filter((s) => s.id.toLowerCase().includes(needle)), [q.data, needle]);
  const noun = tab.kind === "subscription" ? "subscription" : "queue";
  if (requiresSession === false)
    return <EmptyState title={`This ${noun} doesn't use sessions`} body="Sessions appear here for queues and subscriptions created with sessions enabled." />;
  const peek = (id: string) => {
    peekSession(qc, conn.profileId, tab.id, id, prefs.peekSize);
    dispatch({ type: "subTab", id: tab.id, subTab: "messages" });
  };
  const listedCount = q.data?.sessions.filter((x) => x.in_broker).length ?? 0;
  // The focused session can be filtered out of view: it is not the one the user is looking at, so it is not peeked.
  const focused = sel.focus !== null && rows.some((r) => r.id === sel.focus) ? sel.focus : null;
  const status = q.data
    ? [
        needle ? `${formatNumber(rows.length)} of ${formatNumber(q.data.sessions.length)} sessions` : plural(q.data.sessions.length, "session"),
        ...(q.data.broker.state === "listed" ? [`${formatNumber(listedCount)} listed by the service`] : []),
        ...sessionNote(q.data),
      ].join(" · ")
    : "";
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div role="toolbar" aria-label="Sessions toolbar" className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3">
        <Input aria-label="Filter sessions" placeholder="Filter sessions" className="w-[220px]" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <Button disabled={focused === null} onClick={() => focused !== null && peek(focused)}>
          Peek messages
        </Button>
      </div>
      <div className="min-h-0 flex-1">
        <DataGrid<SessionSummary>
          label="Sessions"
          columns={columns}
          rows={rows}
          getRowId={(s) => s.id}
          rowLabel={(s) => s.id}
          selectedIds={sel.ids}
          focusId={sel.focus}
          onSelectionChange={(ids, focus) => setSel({ ids, focus })}
          onActivate={peek}
          rowHeight={rowHeight[prefs.density]}
          sorting={sorting}
          onSortingChange={setSorting}
          loading={q.isPending}
          error={
            q.isError ? (
              <EmptyState
                tone="error"
                title="Couldn't list sessions"
                body="Try again. If it keeps failing, the detail below says why."
                detail={q.error.message}
                action={<Button onClick={() => void q.refetch()}>Try again</Button>}
              />
            ) : undefined
          }
          empty={
            needle ? (
              <EmptyState title="No session matches" body="Change the filter to see more sessions." action={<Button onClick={() => setFilter("")}>Clear filter</Button>} />
            ) : (
              <EmptyState title="No sessions" body="No session has messages or state right now." />
            )
          }
          initialRect={{ width: 900, height: 600 }}
        />
      </div>
      <div role="status" aria-label="Sessions status" className="flex min-h-[22px] shrink-0 items-center border-t border-line px-3 py-0.5 text-11 text-muted">
        <span className="tabular-nums">{status}</span>
      </div>
    </div>
  );
}
