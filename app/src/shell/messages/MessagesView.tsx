import { useQueryClient } from "@tanstack/react-query";
import { ArrowRightToLine, CalendarX, CheckCheck, ChevronDown, Download, Inbox, ListTree, Repeat2, SendHorizontal, Trash2 } from "lucide-react";
import { useId, useMemo, useRef, useState, type ReactNode } from "react";
import { TAIL_MAX, normalizeFilter, usePeekStream, type EntityCounts, type FilterSpec, type MessageSummary, type PeekView } from "@/engine";
import {
  Button, DataGrid, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger, EmptyState, Panel, PanelGroup, Problem, ResizeHandle, cn, formatNumber, rowHeight,
} from "@/ui";
import { yearOf } from "../time";
import { useShell } from "../context";
import type { EntityTab } from "../state";
import { VIEW_LABEL } from "../tabs/subtabs";
import { buildColumns, optionalColumns, propColumn, useColumnPrefs } from "./columns";
import { DetailPane } from "./DetailPane";
import { DlqSummary, reasonFilter } from "./DlqSummary";
import { FilterBar } from "./FilterBar";
import { useAuthKind } from "../access";
import { describePeekError } from "./peekError";
import { rowLimit } from "./rowLimit";
import { CompactHint, CompactLabel, Toolbar } from "./Toolbar";
import { defaultViewState, useViewState, viewKey } from "./viewStore";

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


export const filterCount = (f: FilterSpec) =>
  [f.property, f.body_contains || f.body_regex || f.body_json_path, f.session_id].filter((v) => typeof v === "string" && v.trim() !== "").length;

const plural = (n: number, one: string, many = `${one}s`) => `${formatNumber(n)} ${n === 1 ? one : many}`;

export function MessagesView({
  tab,
  view,
  counts,
  requiresSession,
}: {
  tab: EntityTab;
  view: PeekView;
  counts: EntityCounts | null;
  requiresSession: boolean;
}) {
  const { conn, prefs, shell, dispatch, openSheet } = useShell();
  const profileId = conn.profileId;
  const defaults = useMemo(() => defaultViewState(prefs.peekSize), [prefs.peekSize]);
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
  const shownProps = useMemo(() => columnIds.filter((c) => c.startsWith("prop:")).map((c) => c.slice(5)), [columnIds]);
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
  const auth = useAuthKind();
  const failure = stream.error && stream.canRetry ? describePeekError(stream.error.kind, stream.error.message, label, auth) : null;
  const limit = rowLimit();
  const atLimit = n + vs.topN > limit;
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
    if (stream.canPeekMore && atLimit) return `${rows} · the view holds up to ${formatNumber(limit)} messages`;
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

  // Export: the whole view (with its filter and session), the selected rows, or one message from the detail pane.
  // The sheet gets what the view holds now; the engine reads the messages again when the export runs.
  const [summaryOpen, setSummaryOpen] = useState(false);
  const exportSheet = (selected: number[]) =>
    openSheet({ kind: "export", entity: tab.id, view, filter: vs.filter, selected, total: counts && listedView ? listedView.count(counts) : null });
  const selectedSequences = () => [...sel.ids].map(Number).sort((a, b) => a - b);
  const exportMenu = (compact: boolean) => (
    <DropdownMenu>
      <CompactHint compact={compact} label="Export">
        <DropdownMenuTrigger asChild>
          <Button variant="quiet" disabled={!profileId} aria-label={compact ? "Export" : undefined}>
            <Download aria-hidden="true" className="text-icon" />
            <CompactLabel compact={compact}>Export</CompactLabel>
          </Button>
        </DropdownMenuTrigger>
      </CompactHint>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => exportSheet([])}>Export all…</DropdownMenuItem>
        {sel.ids.size > 0 && <DropdownMenuItem onSelect={() => exportSheet(selectedSequences())}>Export {formatNumber(sel.ids.size)} selected…</DropdownMenuItem>}
      </DropdownMenuContent>
    </DropdownMenu>
  );

  // Operations on this view's messages: the selected rows, or (none selected) every message matching the view's
  // filter. Sending puts new messages on the entity (a subscription's topic).
  const dlqView = view === "dead-letter" || view === "transfer-dead-letter";
  const subQueue = view === "dead-letter" ? "dead-letter" : view === "transfer-dead-letter" ? "transfer-dead-letter" : "active";
  const total = counts && listedView ? listedView.count(counts) : null;
  const pick = (op: "delete" | "move" | "resubmit") => openSheet({ kind: "operation", op, entity: tab.id, view, sequences: selectedSequences(), filter: vs.filter, total });
  const settle = (how: "complete" | "abandon" | "dead-letter") =>
    openSheet({ kind: "operation", op: "settle-deferred", entity: tab.id, subQueue: "active", sequences: selectedSequences(), session: vs.filter.session_id ?? null, how });
  const opButton = (compact: boolean, label: string, icon: ReactNode, run: () => void, pressed?: boolean) => (
    <CompactHint key={label} compact={compact} label={label}>
      <Button variant="quiet" disabled={!profileId} aria-label={compact ? label : undefined} aria-pressed={pressed} onClick={run} className={cn("shrink-0", pressed && "bg-selection")}>
        {icon}
        <CompactLabel compact={compact}>{label}</CompactLabel>
      </Button>
    </CompactHint>
  );
  const operations = (compact: boolean) => {
    const send = opButton(compact, "Send", <SendHorizontal aria-hidden="true" className="text-icon" />, () => openSheet({ kind: "operation", op: "send", entity: tab.id }));
    const receive = opButton(compact, "Receive", <Inbox aria-hidden="true" className="text-icon" />, () =>
      openSheet({ kind: "operation", op: "receive", entity: tab.id, subQueue, session: vs.filter.session_id ?? null }),
    );
    const resubmit = opButton(compact, "Resubmit", <Repeat2 aria-hidden="true" className="text-icon" />, () => pick("resubmit"));
    const move = opButton(compact, "Move", <ArrowRightToLine aria-hidden="true" className="text-icon" />, () => pick("move"));
    const del = opButton(compact, "Delete", <Trash2 aria-hidden="true" className="text-icon" />, () => pick("delete"));
    if (view === "scheduled")
      return (
        <>
          {send}
          {opButton(compact, "Cancel scheduled", <CalendarX aria-hidden="true" className="text-icon" />, () =>
            openSheet({ kind: "operation", op: "cancel-scheduled", entity: tab.id, sequences: selectedSequences() }),
          )}
        </>
      );
    if (view === "deferred")
      return (
        <>
          <DropdownMenu>
            <CompactHint compact={compact} label="Settle">
              <DropdownMenuTrigger asChild>
                <Button variant="quiet" disabled={!profileId} aria-label={compact ? "Settle" : undefined}>
                  <CheckCheck aria-hidden="true" className="text-icon" />
                  <CompactLabel compact={compact}>Settle</CompactLabel>
                  <ChevronDown aria-hidden="true" className="!size-3 text-icon" />
                </Button>
              </DropdownMenuTrigger>
            </CompactHint>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onSelect={() => settle("complete")}>Complete…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => settle("abandon")}>Abandon…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => settle("dead-letter")}>Dead-letter…</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => pick("delete")}>Delete…</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {resubmit}
          {move}
        </>
      );
    return (
      <>
        {view === "active" && send}
        {receive}
        {resubmit}
        {move}
        {del}
        {dlqView && opButton(compact, "Summary", <ListTree aria-hidden="true" className="text-icon" />, () => setSummaryOpen(!summaryOpen), summaryOpen)}
      </>
    );
  };

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
        operations={operations}
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
      {dlqView && summaryOpen && (
        <DlqSummary profileId={profileId} entity={tab.id} subQueue={subQueue} onPick={(reason) => applyFilter({ ...vs.filter, property: reasonFilter(reason) })} />
      )}
      <div
        ref={gridBox}
        className="min-h-0 flex-1"
        onKeyDown={(e) => {
          // The Delete key opens the delete sheet for the selected rows; it never deletes by itself.
          if ((e.key === "Delete" || e.key === "Backspace") && sel.ids.size > 0 && (e.target as HTMLElement).closest('[role="grid"]')) {
            e.preventDefault();
            pick("delete");
          }
        }}
      >
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
                  sessionId={vs.filter.session_id ?? null}
                  onClose={closeDetail}
                  onPeekAgain={restart}
                  onExport={(sequence) => exportSheet([sequence])}
                  onClone={(d) => openSheet({ kind: "operation", op: "send", entity: tab.id, clone: d })}
                  shownProps={shownProps}
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
