import { useQueryClient } from "@tanstack/react-query";
import { useMemo, useState, type ReactNode } from "react";
import { TAIL_MAX, usePeekStream, type EntityCounts, type FilterSpec, type MessageSummary, type PeekView } from "@/engine";
import { Button, DataGrid, EmptyState, Problem, formatNumber, rowHeight } from "@/ui";
import { useShell } from "../context";
import type { EntityTab } from "../state";
import { VIEW_LABEL } from "../tabs/subtabs";
import { buildColumns, useColumnPrefs } from "./columns";
import { describePeekError } from "./peekError";
import { Toolbar } from "./Toolbar";
import { useViewState, viewKey, type ViewState } from "./viewStore";

const EMPTY: Record<PeekView, string> = {
  active: "No active messages",
  "dead-letter": "No dead letters",
  "transfer-dead-letter": "No transfer dead letters",
  scheduled: "No scheduled messages",
  deferred: "No deferred messages",
};

/** The view's count in the entity listing, to explain an empty peek of a view the listing says is not empty. */
const LISTED: Partial<Record<PeekView, (c: EntityCounts) => number>> = {
  active: (c) => c.active,
  "dead-letter": (c) => c.dead_letter,
  "transfer-dead-letter": (c) => c.transfer_dead_letter,
  scheduled: (c) => c.scheduled,
};

/** The grid holds at most this many rows: the row-number gutter fits five digits, and more rows than this belong in
 *  an export, not a scrolling grid. */
export const ROW_LIMIT = 99_999;

export const filterCount = (f: FilterSpec) =>
  [f.property, f.body_contains || f.body_regex || f.body_json_path, f.session_id].filter((v) => typeof v === "string" && v.trim() !== "").length;

const plural = (n: number, one: string, many = `${one}s`) => `${formatNumber(n)} ${n === 1 ? one : many}`;

export function MessagesView({
  tab,
  view,
  counts,
  requiresSession,
  filterBar,
  exportMenu,
  detail,
}: {
  tab: EntityTab;
  view: PeekView;
  counts: EntityCounts | null;
  requiresSession: boolean;
  /** Slots the later features fill: the filter bar under the toolbar, the export menu, the detail pane. */
  filterBar?: ReactNode;
  exportMenu?: ReactNode;
  detail?: ReactNode;
}) {
  const { conn, prefs, dispatch, openSheet } = useShell();
  const profileId = conn.profileId;
  const defaults = useMemo<ViewState>(
    () => ({ from: { kind: "start" }, topN: prefs.peekSize, filter: {}, filterOpen: false, sorting: null, detailOpen: false }),
    [prefs.peekSize],
  );
  const [vs, setVs] = useViewState(viewKey(profileId, tab.id, view), defaults);
  const stream = usePeekStream({ profileId, entity: tab.id, view, from: vs.from, topN: vs.topN, filter: vs.filter });
  const [columnIds, setColumnIds] = useColumnPrefs(view, requiresSession);
  const n = stream.messages.length;
  // Sized to the longest sequence number read so far; changes rarely, so the columns are rarely rebuilt.
  const seqDigits = useMemo(() => {
    let max = 0;
    for (const m of stream.messages) if (m.sequence_number > max) max = m.sequence_number;
    return String(max).length;
  }, [stream.messages]);
  const columns = useMemo(() => buildColumns(columnIds, seqDigits), [columnIds, seqDigits]);
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  const qc = useQueryClient();
  const label = VIEW_LABEL[view];
  const propertyNames = useMemo(() => {
    const seen = new Set<string>();
    for (const m of stream.messages.slice(0, 500)) for (const k of Object.keys(m.application_properties)) if (!k.startsWith("DeadLetter")) seen.add(k);
    return [...seen].sort();
  }, [stream.messages]);

  // A reset (never an invalidate) of every peek of this view: the hook starts a fresh round at Top N.
  const restart = () => void qc.resetQueries({ queryKey: ["peek", profileId, tab.id, view] });
  const tail = vs.from.kind === "end";
  const filtered = filterCount(vs.filter) > 0;
  // The error stays on the query while a retry is on its way: show it only when it can be acted on.
  const failure = stream.error && stream.canRetry ? describePeekError(stream.error.kind, stream.error.message, label) : null;
  const atLimit = n + vs.topN > ROW_LIMIT;
  const canPeekMore = !stream.streaming && stream.canPeekMore && !atLimit;

  const status = (() => {
    if (stream.loading) return "Reading…";
    if (failure && n === 0) return "Nothing read";
    const rows = filtered ? `${plural(n, "matching message")} · scanned ${formatNumber(stream.scanned)}` : plural(n, "message");
    if (tail) {
      if (!stream.exhausted) return `${plural(n, "message")} · the scan stopped before the end, so these are not the newest`;
      if (stream.olderNotExamined)
        return `${plural(n, "matching message")} in the newest ${formatNumber(stream.scanned)}; older messages may not have been examined`;
      return `Last ${plural(n, "message")}`;
    }
    if (failure) return `${rows} · the next page failed`;
    if (stream.streaming) return `${rows} · reading more…`;
    if (stream.budgetHit)
      return n === 0 ? `Scanned ${formatNumber(stream.scanned)} messages, none matched. Peek more to keep scanning.` : `${rows}. Peek more to keep scanning.`;
    if (stream.exhausted) return `${rows} · End of the ${label}`;
    if (stream.stopped) return `${rows} · stopped`;
    if (stream.canPeekMore && atLimit) return `${rows} · the view holds up to ${formatNumber(ROW_LIMIT)} messages`;
    return `${rows} · more available`;
  })();

  const fixAction =
    failure?.fix === "sessions" ? (
      <Button onClick={() => dispatch({ type: "subTab", id: tab.id, subTab: "sessions" })}>Open Sessions</Button>
    ) : failure?.fix === "edit" && profileId ? (
      <Button onClick={() => openSheet({ kind: "profile", profileId })}>Edit connection</Button>
    ) : (
      <Button onClick={stream.retry}>Try again</Button>
    );

  // No rows: the failure is the whole pane. Rows already read stay, with the failure above them.
  const paneError = failure && n === 0 ? <EmptyState tone="error" title={failure.title} body={failure.body} detail={stream.error!.message} action={fixAction} /> : null;
  const listed = counts && LISTED[view]?.(counts);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Toolbar
        from={vs.from}
        onFrom={(from) => setVs(from.kind === "end" && vs.topN > TAIL_MAX ? { from, topN: TAIL_MAX } : { from })}
        topN={vs.topN}
        onTopN={(topN) => setVs({ topN })}
        filterCount={filterCount(vs.filter)}
        filterOpen={vs.filterOpen}
        onToggleFilter={filterBar !== undefined ? () => setVs({ filterOpen: !vs.filterOpen }) : undefined}
        columns={columnIds}
        onColumns={setColumnIds}
        propertyNames={propertyNames}
        onRefresh={restart}
        exportMenu={exportMenu}
      />
      {vs.filterOpen && filterBar}
      {failure && n > 0 && (
        <div className="shrink-0 border-b border-line px-3 py-2">
          <Problem role="alert" title={failure.title} body={failure.body} detail={stream.error!.message} action={fixAction} />
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 min-w-0 flex-1">
          <DataGrid<MessageSummary>
            label={`${label[0].toUpperCase()}${label.slice(1)}`}
            columns={columns}
            rows={stream.messages}
            getRowId={(m) => String(m.sequence_number)}
            selectedIds={sel.ids}
            focusId={sel.focus}
            onSelectionChange={(ids, focus) => setSel({ ids, focus })}
            onActivate={() => setVs({ detailOpen: true })}
            rowHeight={rowHeight[prefs.density]}
            rowNumbers
            sorting={vs.sorting}
            onSortingChange={(sorting) => setVs({ sorting })}
            loading={stream.loading}
            error={paneError}
            empty={
              filtered ? (
                <EmptyState
                  title="No messages match this filter"
                  body={`Scanned ${plural(stream.scanned, "message")}.`}
                  action={<Button onClick={() => setVs({ filter: {} })}>Clear filter</Button>}
                />
              ) : (
                <EmptyState
                  title={EMPTY[view]}
                  body={listed ? `The entity lists ${plural(listed, "message")} here; they may have been received since. Peek again to check.` : undefined}
                  action={listed ? <Button onClick={restart}>Peek again</Button> : undefined}
                />
              )
            }
            initialRect={{ width: 900, height: 600 }}
          />
        </div>
        {vs.detailOpen && detail}
      </div>
      <div role="status" aria-label="Peek status" className="flex h-[22px] shrink-0 items-center gap-2 border-t border-line px-3 text-11 text-muted">
        <span className="truncate tabular-nums">{status}</span>
        {stream.streaming && (
          <Button size="sm" className="h-[18px] text-11" onClick={stream.stop}>
            Stop
          </Button>
        )}
        {canPeekMore && (
          <Button size="sm" className="h-[18px] text-11" onClick={stream.peekMore}>
            Peek more
          </Button>
        )}
        <span className="ml-auto shrink-0">Peek only, nothing locked</span>
      </div>
    </div>
  );
}
