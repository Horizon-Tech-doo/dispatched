import { useQueryClient } from "@tanstack/react-query";
import type { ColumnDef } from "@tanstack/react-table";
import { useMemo, useState } from "react";
import { useSessions, type SessionListing, type SessionSummary } from "@/engine";
import { Button, DataGrid, EmptyState, Input, formatNumber, rowHeight, type GridSort } from "@/ui";
import { useShell } from "../context";
import { peekSession } from "../messages/peekSession";
import type { EntityTab } from "../state";
import { formatTimestamp } from "../time";

/** Where the session list came from and what that means for it; null when there is nothing to say. */
export function sessionNote(l: Pick<SessionListing, "broker" | "scanned" | "truncated">): string | null {
  const counts = l.truncated ? `Message counts are lower bounds: only the first ${formatNumber(l.scanned)} messages were counted.` : null;
  switch (l.broker.state) {
    case "listed":
      return [l.broker.capped ? "The service's list was cut at 10,000 sessions." : null, counts].filter(Boolean).join(" ") || null;
    case "empty":
      return `The service listed no sessions; showing sessions found in the first ${formatNumber(l.scanned)} messages.${l.truncated ? " Message counts are lower bounds." : ""}`;
    case "unavailable":
      return `Couldn't ask the service for sessions; showing sessions found by peeking.${l.truncated ? " Message counts are lower bounds." : ""}`;
  }
}

export function SessionsView({ tab, requiresSession }: { tab: EntityTab; requiresSession: boolean | null }) {
  const { conn, prefs, dispatch } = useShell();
  const qc = useQueryClient();
  const q = useSessions(conn.profileId, tab.id, requiresSession !== false);
  const [filter, setFilter] = useState("");
  const [sorting, setSorting] = useState<GridSort | null>(null);
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  const truncated = q.data?.truncated ?? false;
  const columns = useMemo<ColumnDef<SessionSummary, unknown>[]>(
    () => [
      { id: "id", header: "Session", accessorFn: (s) => s.id, meta: { mono: true, width: "minmax(160px, 1fr)" } },
      {
        id: "messages",
        header: "Messages",
        accessorFn: (s) => s.messages ?? -1,
        meta: { align: "end", width: "96px" },
        // A lower bound when the tally was cut short; unknown when no tally ran.
        cell: (c) => {
          const n = c.row.original.messages;
          return n == null ? "?" : truncated ? `${formatNumber(n)}+` : formatNumber(n);
        },
      },
      { id: "first", header: "First enqueued", accessorFn: (s) => s.first_enqueued ?? "", meta: { mono: true, width: "150px" }, cell: (c) => formatTimestamp(c.row.original.first_enqueued) },
      { id: "last", header: "Last enqueued", accessorFn: (s) => s.last_enqueued ?? "", meta: { mono: true, width: "150px" }, cell: (c) => formatTimestamp(c.row.original.last_enqueued) },
    ],
    [truncated],
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
  const note = q.data ? sessionNote(q.data) : null;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div role="toolbar" aria-label="Sessions toolbar" className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3">
        <Input aria-label="Filter sessions" placeholder="Filter sessions" className="w-[220px]" value={filter} onChange={(e) => setFilter(e.target.value)} />
        <Button disabled={!sel.focus} onClick={() => sel.focus && peek(sel.focus)}>
          Peek messages
        </Button>
        {q.data && <span className="ml-auto text-11 tabular-nums text-muted">{needle ? `${formatNumber(rows.length)} of ${formatNumber(q.data.sessions.length)}` : formatNumber(q.data.sessions.length)} sessions</span>}
      </div>
      {note && (
        <p role="status" className="m-0 shrink-0 border-b border-line px-3 py-1 text-11 text-muted">
          {note}
        </p>
      )}
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
      <p className="m-0 shrink-0 border-t border-line px-3 py-1 text-11 text-muted">Session state needs a session lock, so it is read and edited with the operate features.</p>
    </div>
  );
}
