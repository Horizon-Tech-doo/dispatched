import { useQueryClient } from "@tanstack/react-query";
import { useId, useMemo, useRef, useState, type ReactNode } from "react";
import { TAIL_MAX, normalizeFilter, usePeekStream, type EntityCounts, type FilterSpec, type MessageSummary, type PeekView } from "@/engine";
import { Button, DataGrid, EmptyState, Panel, PanelGroup, Problem, ResizeHandle, formatNumber, rowHeight } from "@/ui";
import { yearOf } from "../time";
import { useShell } from "../context";
import type { EntityTab } from "../state";
import { VIEW_LABEL } from "../tabs/subtabs";
import { buildColumns, optionalColumns, propColumn, useColumnPrefs } from "./columns";
import { DetailPane } from "./DetailPane";
import { FilterBar } from "./FilterBar";
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

/** The view's count in the entity listing, to explain an empty peek of a view the listing says is not empty, and why
 *  the two can differ. */
const LISTED: Partial<Record<PeekView, { count: (c: EntityCounts) => number; since: string }>> = {
  active: { count: (c) => c.active, since: "they may have been received since" },
  "dead-letter": { count: (c) => c.dead_letter, since: "they may have been received since" },
  "transfer-dead-letter": { count: (c) => c.transfer_dead_letter, since: "they may have been received since" },
  scheduled: { count: (c) => c.scheduled, since: "they may have become active since" },
};

/** The grid holds at most this many rows: more than this belongs in an export, not a scrolling grid. */
export const ROW_LIMIT = 100_000;

export const filterCount = (f: FilterSpec) =>
  [f.property, f.body_contains || f.body_regex || f.body_json_path, f.session_id].filter((v) => typeof v === "string" && v.trim() !== "").length;

const plural = (n: number, one: string, many = `${one}s`) => `${formatNumber(n)} ${n === 1 ? one : many}`;

export function MessagesView({
  tab,
  view,
  counts,
  requiresSession,
  exportMenu,
}: {
  tab: EntityTab;
  view: PeekView;
  counts: EntityCounts | null;
  requiresSession: boolean;
  /** Slot the export feature fills. */
  exportMenu?: ReactNode;
}) {
  const { conn, prefs, shell, dispatch, openSheet } = useShell();
  const profileId = conn.profileId;
  const defaults = useMemo<ViewState>(
    () => ({ from: { kind: "start" }, topN: prefs.peekSize, filter: {}, filterOpen: false, sorting: null, detailOpen: false }),
    [prefs.peekSize],
  );
  const [vs, setVs] = useViewState(viewKey(profileId, tab.id, view), defaults);
  const stream = usePeekStream({ profileId, entity: tab.id, view, from: vs.from, topN: vs.topN, filter: vs.filter });
  // The filter bar takes focus only when the user opens it, not when a view with the bar already open is shown again.
  const [focusBar, setFocusBar] = useState(false);
  const filterToggle = useRef<HTMLButtonElement>(null);
  const gridBox = useRef<HTMLDivElement>(null);
  const filterBarId = useId();
  const [columnIds, setColumnIds, resetColumns] = useColumnPrefs(view, requiresSession);
  const n = stream.messages.length;
  // Data-sized columns: Seq as wide as the longest sequence number, timestamps with their year only when a row is
  // from another year. Both change rarely, so the columns are rarely rebuilt.
  const seqDigits = useMemo(() => {
    let max = 0;
    for (const m of stream.messages) if (m.sequence_number > max) max = m.sequence_number;
    return String(max).length;
  }, [stream.messages]);
  const withYear = useMemo(() => {
    const year = new Date().getFullYear();
    return stream.messages.some((m) => [m.enqueued_time, m.scheduled_enqueue_time].some((t) => { const y = yearOf(t); return y !== null && y !== year; }));
  }, [stream.messages]);
  const columns = useMemo(() => buildColumns(columnIds, { seqDigits, withYear }), [columnIds, seqDigits, withYear]);
  const offered = useMemo(() => optionalColumns(view, requiresSession), [view, requiresSession]);
  const noSelection = { ids: new Set<string>(), focus: null };
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>(noSelection);
  // A new peek (Peek again, F5, another From, Top N or filter) starts with nothing selected: the rows are not the
  // ones that were selected. Adjusted during render, so a stale selection is never drawn.
  const selFor = `${JSON.stringify(vs.from)}|${vs.topN}|${JSON.stringify(vs.filter)}|${stream.loading}`;
  const [selKey, setSelKey] = useState(selFor);
  if (selKey !== selFor) {
    setSelKey(selFor);
    setSel(noSelection);
  }
  // Closing the pane hands the keyboard back to the grid, so the arrow keys keep working where the user left off.
  const closeDetail = () => {
    setVs({ detailOpen: false });
    gridBox.current?.querySelector<HTMLElement>('[role="grid"]')?.focus();
  };
  const qc = useQueryClient();
  const label = VIEW_LABEL[view];
  const propertyNames = useMemo(() => {
    const seen = new Set<string>();
    for (const m of stream.messages.slice(0, 500)) for (const k of Object.keys(m.application_properties)) if (!k.startsWith("DeadLetter")) seen.add(k);
    return [...seen].sort();
  }, [stream.messages]);

  // A reset (never an invalidate) of every peek of this view: the hook starts a fresh round at Top N.
  const restart = () => {
    setSel(noSelection);
    // Messages read earlier may have been received or moved since: the detail pane reads them again.
    void qc.invalidateQueries({ queryKey: ["message", profileId, tab.id, view] });
    void qc.resetQueries({ queryKey: ["peek", profileId, tab.id, view] });
  };
  // Applying or clearing a filter is always a fresh read: a recently used filter must not come back from the cache
  // (the rows may have changed since). A filter equal to the one in force restarts it, since its key would not change.
  const applyFilter = (next: FilterSpec) => {
    const filter = normalizeFilter(next);
    void qc.removeQueries({ queryKey: ["peek", profileId, tab.id, view], type: "inactive" });
    if (JSON.stringify(filter) === JSON.stringify(normalizeFilter(vs.filter))) restart();
    else setVs({ filter });
  };
  const tail = vs.from.kind === "end";
  const filtered = filterCount(vs.filter) > 0;
  // The error stays on the query while a retry is on its way: show it only when it can be acted on.
  const failure = stream.error && stream.canRetry ? describePeekError(stream.error.kind, stream.error.message, label) : null;
  const atLimit = n + vs.topN > ROW_LIMIT;
  const canPeekMore = !stream.streaming && stream.canPeekMore && !atLimit;
  const end =
    view === "dead-letter" ? "dead-letter queue" : view === "transfer-dead-letter" ? "transfer dead-letter queue" : tab.kind === "subscription" ? "subscription" : "queue";

  // Segments in lowercase after the first; nothing when the pane itself already says it (empty, or a failed first page).
  const status = (() => {
    if (stream.loading) return "Reading…";
    if (failure && n === 0) return "";
    if (tail) {
      if (!stream.exhausted) return `${plural(n, "message")} · the scan stopped before the end, so these are not the newest`;
      if (stream.olderNotExamined)
        return `${plural(n, "matching message")} in the newest ${formatNumber(stream.scanned)} · older messages may not have been examined`;
      return n === 0 ? "" : `Last ${plural(n, "message")}`;
    }
    const scanned = filtered ? ` · scanned ${formatNumber(stream.scanned)}` : "";
    const rows = `${plural(n, filtered ? "matching message" : "message")}${scanned}`;
    if (failure) return `${rows} · the next page failed`;
    if (stream.streaming) return `${formatNumber(n)} of ${formatNumber(stream.target)}${filtered ? " matching" : ""}${scanned} · reading…`;
    if (stream.budgetHit) return `${rows} · scan limit reached, peek more to keep scanning`;
    if (n === 0 && stream.exhausted) return "";
    if (stream.exhausted) return `${rows} · end of the ${end}`;
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
  const listedView = LISTED[view];
  const listed = counts && listedView ? listedView.count(counts) : 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <Toolbar
        from={vs.from}
        onFrom={(from) => setVs(from.kind === "end" && vs.topN > TAIL_MAX ? { from, topN: TAIL_MAX } : { from })}
        topN={vs.topN}
        onTopN={(topN) => setVs({ topN })}
        filter={vs.filter}
        filterCount={filterCount(vs.filter)}
        filterOpen={vs.filterOpen}
        filterBarId={filterBarId}
        onClearFilter={() => applyFilter({})}
        onToggleFilter={() => {
          setFocusBar(!vs.filterOpen);
          setVs({ filterOpen: !vs.filterOpen });
        }}
        filterToggleRef={filterToggle}
        columns={columnIds}
        offered={offered}
        onColumns={setColumnIds}
        onResetColumns={resetColumns}
        propertyNames={propertyNames}
        streaming={stream.streaming}
        onStop={stream.stop}
        onRefresh={restart}
        exportMenu={exportMenu}
      />
      {vs.filterOpen && (
        <FilterBar
          applied={vs.filter}
          autoFocus={focusBar}
          id={filterBarId}
          onApply={applyFilter}
          onClose={() => {
            setVs({ filterOpen: false });
            filterToggle.current?.focus();
          }}
        />
      )}
      {failure && n > 0 && (
        <div className="shrink-0 border-b border-line px-3 py-2">
          <Problem role="alert" title={failure.title} body={failure.body} detail={stream.error!.message} action={fixAction} />
        </div>
      )}
      <div ref={gridBox} className="min-h-0 flex-1">
        <PanelGroup
          direction="horizontal"
          onLayout={(sizes) => sizes.length === 2 && dispatch({ type: "layout", patch: { detailSize: Math.round(sizes[1]) } })}
        >
          <Panel id="grid" order={1} defaultSize={vs.detailOpen ? 100 - shell.layout.detailSize : 100} minSize={25} className="min-w-0">
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
                    action={<Button onClick={() => applyFilter({})}>Clear filter</Button>}
                  />
                ) : (
                  <EmptyState
                    title={EMPTY[view]}
                    body={listed ? `The entity lists ${plural(listed, "message")} here; ${listedView!.since}. Peek again to check.` : undefined}
                    action={listed ? <Button onClick={restart}>Peek again</Button> : undefined}
                  />
                )
              }
              initialRect={{ width: 900, height: 600 }}
            />
          </Panel>
          {vs.detailOpen && (
            <>
              <ResizeHandle />
              <Panel id="detail" order={2} defaultSize={shell.layout.detailSize} minSize={20} maxSize={70}>
                <DetailPane
                  profileId={profileId}
                  entity={tab.id}
                  view={view}
                  sequence={sel.focus !== null ? Number(sel.focus) : null}
                  onClose={closeDetail}
                  onShowColumn={(name) => {
                    const id = propColumn(name);
                    if (!columnIds.includes(id)) setColumnIds([...columnIds, id]);
                  }}
                />
              </Panel>
            </>
          )}
        </PanelGroup>
      </div>
      <div role="status" aria-label="Peek status" className="flex h-[22px] shrink-0 items-center gap-2 border-t border-line px-3 text-11 text-muted">
        <span className="truncate tabular-nums">{status}</span>
        {canPeekMore && (
          <Button size="sm" className="h-[18px] text-11" onClick={stream.peekMore}>
            Peek more
          </Button>
        )}
      </div>
    </div>
  );
}
