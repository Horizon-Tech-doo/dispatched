import type { ColumnDef } from "@tanstack/react-table";
import { Check, ClipboardCopy } from "lucide-react";
import { createContext, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useDashboard, type DashboardRow } from "@/engine";
import { Button, Checkbox, CountBadge, DataGrid, EmptyState, EntityGlyph, Hint, Input, Problem, Segmented, cn, formatClock, formatNumber, rowHeight, type GridSort } from "@/ui";
import { accessAdvice, rightsOf, useAuthKind } from "../access";
import { useShell } from "../context";
import { DASHBOARD_REFRESH, type DashboardRefresh } from "../state";
import { compareValues, pathOf, sortRows, sortValue } from "./sort";
import { toTsv } from "./tsv";

const REFRESH_LABEL: Record<DashboardRefresh, string> = { 0: "Off", 30: "30 s", 60: "1 min", 300: "5 min" };
const FLASH_MS = 1800;
const MAX_TOPICS_SHOWN = 5;
const MAX_REASONS_SHOWN = 3;
const LIMITED_NOTE = "Counts are unavailable while the namespace can't be listed.";
const TSV_HINT = "Copies the rows as shown. ~ marks an approximate count, ~N+ one that stopped at its cap.";

const plural = (n: number, one: string, many: string) => `${formatNumber(n)} ${n === 1 ? one : many}`;
const hasDeadLetters = (r: DashboardRow) => (r.counts?.dead_letter ?? 0) > 0 || (r.counts?.transfer_dead_letter ?? 0) > 0;

/** A count as data: zero is muted, a number above zero is the primary colour, and only dead letters turn amber.
 *  An unknown count says why on hover. */
function countColumn(id: string, header: string, pick: (r: DashboardRow) => number | undefined, unknownReason: string, attention = false): ColumnDef<DashboardRow, unknown> {
  return {
    id,
    header,
    accessorFn: sortValue[id],
    sortingFn: (a, b, col) => compareValues(a.getValue(col), b.getValue(col)),
    meta: { align: "end", width: "minmax(96px, 120px)" },
    cell: (c) => {
      const v = pick(c.row.original);
      const badge = <CountBadge value={v} quality={c.row.original.counts ?? undefined} attention={attention} className={cn("text-12", !attention && v != null && v > 0 && "text-fg")} />;
      return v === undefined ? <span title={unknownReason}>{badge}</span> : badge;
    },
  };
}

const Picks = createContext<{ ids: ReadonlySet<string>; toggle(id: string): void }>({ ids: new Set(), toggle: () => {} });

/** A row's checkbox: picked for a purge. */
function PickBox({ row }: { row: DashboardRow }) {
  const p = useContext(Picks);
  return (
    // Not a tab stop: the grid is one, with its own keyboard selection (arrows, Shift, ⌘A) that these boxes show.
    <Checkbox
      tabIndex={-1}
      aria-label={`Select ${pathOf(row).replace(/^\//, "")}`}
      className="align-middle"
      checked={p.ids.has(row.id)}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onChange={() => p.toggle(row.id)}
    />
  );
}

export function DashboardView() {
  const { conn, prefs, setPrefs, dispatch, openSheet } = useShell();
  const auth = useAuthKind();
  const q = useDashboard(conn.profileId, prefs.dashboardRefresh * 1000);
  const [sorting, setSorting] = useState<GridSort | null>({ id: "dlq", desc: true });
  const [show, setShow] = useState<"all" | "dlq">("all");
  const [filter, setFilter] = useState("");
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");
  const flash = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(flash.current), []);

  const data = q.data;
  const all = data?.rows;
  // In the limited view the namespace could not be listed, so only names are known: a topic named in the connection
  // string's EntityPath arrives as a queue row. Such rows get the "Kind unknown" glyph, and no kind in the TSV.
  const limited = data?.limited != null;
  const unknownReason = limited ? LIMITED_NOTE : "The service didn't report counts for this entity.";
  // Row checkboxes pick the entities to purge, beside the grid's own multi-select (they share one selection). They read
  // it from context, so the columns are not rebuilt (and the cells not remounted) when it changes.
  const picks = useMemo(
    () => ({
      ids: sel.ids,
      toggle: (id: string) =>
        setSel((s) => {
          const ids = new Set(s.ids);
          if (ids.has(id)) ids.delete(id);
          else ids.add(id);
          return { ids, focus: id };
        }),
    }),
    [sel.ids],
  );
  const columns = useMemo<ColumnDef<DashboardRow, unknown>[]>(
    () => [
      {
        id: "pick",
        header: () => <span className="sr-only">Selected</span>,
        enableSorting: false,
        meta: { width: "28px" },
        cell: (c) => <PickBox row={c.row.original} />,
      },
      {
        id: "entity",
        header: "Entity",
        accessorFn: sortValue.entity,
        sortingFn: (a, b, col) => compareValues(a.getValue(col), b.getValue(col)),
        meta: { width: "minmax(240px, 2fr)" },
        cell: (c) => {
          const r = c.row.original;
          return (
            <span className="flex min-w-0 items-center gap-1.5">
              <EntityGlyph kind={limited && r.kind !== "subscription" ? "unknown" : r.kind} className="size-3.5 shrink-0 text-icon" />
              <span className="min-w-0 truncate" title={r.topic ? `${r.topic}/${r.name}` : r.name}>{r.name}</span>
              {r.topic && <span className="min-w-0 truncate text-muted">{r.topic}</span>}
            </span>
          );
        },
      },
      {
        id: "status",
        header: "Status",
        accessorFn: sortValue.status,
        sortingFn: (a, b, col) => compareValues(a.getValue(col), b.getValue(col)),
        meta: { width: "minmax(100px, 130px)" },
        cell: (c) => {
          const s = c.row.original.status;
          return s ? <span className={s === "Active" ? "text-muted" : undefined}>{s}</span> : null;
        },
      },
      countColumn("active", "Active", (r) => r.counts?.active, unknownReason),
      countColumn("dlq", "Dead-letter", (r) => r.counts?.dead_letter, unknownReason, true),
      countColumn("scheduled", "Scheduled", (r) => r.counts?.scheduled, unknownReason),
      countColumn("transfer", "Transfer", (r) => r.counts?.transfer, unknownReason),
      countColumn("tdlq", "Transfer DLQ", (r) => r.counts?.transfer_dead_letter, unknownReason, true),
    ],
    [limited, unknownReason],
  );

  const stats = useMemo(() => {
    const rs = all ?? [];
    return {
      withDlq: rs.filter(hasDeadLetters).length,
      approx: rs.some((r) => r.counts?.approximate && !r.counts.truncated),
      capped: rs.some((r) => r.counts?.truncated),
      unknown: rs.filter((r) => r.counts === null).length,
    };
  }, [all]);

  const onlyDlq = show === "dlq" && !limited;
  const needle = filter.trim().toLowerCase();
  // The rows in the order the grid shows them: the grid and "Copy as TSV" both take this list.
  const rows = useMemo(
    () => sortRows((all ?? []).filter((r) => (!onlyDlq || hasDeadLetters(r)) && (!needle || pathOf(r).toLowerCase().includes(needle))), sorting),
    [all, onlyDlq, needle, sorting],
  );
  const open = (id: string) => {
    const r = rows.find((x) => x.id === id);
    if (r) dispatch({ type: "open", tab: { id: r.id, kind: r.kind } });
  };
  const doCopy = () => {
    clearTimeout(flash.current);
    const done = (s: "copied" | "failed") => {
      setCopy(s);
      flash.current = setTimeout(() => setCopy("idle"), FLASH_MS);
    };
    if (!navigator.clipboard) return done("failed");
    navigator.clipboard.writeText(toTsv(rows, { kindsKnown: !limited })).then(() => done("copied"), () => done("failed"));
  };

  const failures = data?.failures ?? [];
  const kinds = [...new Set(failures.map((f) => f.kind))];
  const filtered = onlyDlq || needle !== "";
  const total = all?.length ?? 0;
  const refreshing = prefs.dashboardRefresh > 0;
  const status = data
    ? limited
      ? [`${filtered ? `${formatNumber(rows.length)} of ` : ""}${plural(total, "pinned entity", "pinned entities")}`, LIMITED_NOTE].join(" · ")
      : [
          filtered ? `${formatNumber(rows.length)} of ${plural(total, "entity", "entities")}` : plural(total, "entity", "entities"),
          ...(stats.unknown > 0 ? [`${plural(stats.unknown, "entity", "entities")} with unknown counts`] : []),
          ...(stats.approx ? ["~ counts are approximate"] : []),
          ...(stats.capped ? ["~N+ counts are lower bounds"] : []),
          ...(refreshing ? [`Auto-refresh ${REFRESH_LABEL[prefs.dashboardRefresh]}`, `Counts read ${formatClock(new Date(q.dataUpdatedAt).toISOString())}`] : []),
        ].join(" · ")
    : "";

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1 px-3.5 pt-2.5">
        <h1 className="m-0 text-14 font-semibold">Dashboard</h1>
        <span className="text-muted">Every queue and subscription with its counts</span>
      </div>
      <div role="toolbar" aria-label="Dashboard options" className="mt-1.5 flex min-h-8 shrink-0 flex-wrap items-center gap-x-2 gap-y-1 border-y border-line px-3 py-1">
        <Input
          type="search"
          aria-label="Filter dashboard"
          placeholder="Filter"
          className="w-[200px] [&::-webkit-search-cancel-button]:appearance-none"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setFilter("")}
        />
        {!limited && (
          <Segmented
            label="Rows shown"
            value={show}
            onChange={setShow}
            options={[
              { value: "all", label: <>All <span className="tabular-nums text-muted">{formatNumber(total)}</span></> },
              { value: "dlq", label: <>With dead letters <span className="tabular-nums text-muted">{formatNumber(stats.withDlq)}</span></> },
            ]}
          />
        )}
        <span className="ml-2 text-12 text-muted">Auto-refresh</span>
        <Segmented
          label="Auto-refresh"
          value={String(prefs.dashboardRefresh)}
          onChange={(v) => setPrefs({ dashboardRefresh: Number(v) as DashboardRefresh })}
          options={DASHBOARD_REFRESH.map((s) => ({ value: String(s), label: REFRESH_LABEL[s] }))}
        />
        <div className="ml-auto flex items-center gap-1">
          {!limited && (
            <Button
              variant="quiet"
              className="min-w-[148px] tabular-nums"
              disabled={sel.ids.size === 0}
              onClick={() => openSheet({ kind: "operation", op: "purge", entities: rows.filter((r) => sel.ids.has(r.id)).map((r) => r.id) })}
            >
              {sel.ids.size > 0 ? `Purge ${formatNumber(sel.ids.size)} selected…` : "Purge…"}
            </Button>
          )}
          {!limited && (
            <Button
              variant="quiet"
              className="min-w-[148px] tabular-nums"
              disabled={sel.ids.size === 0}
              onClick={() =>
                openSheet({ kind: "operation", op: "entity-delete", entities: rows.filter((r) => sel.ids.has(r.id)).map((r) => ({ kind: r.kind, entity: r.id })) })
              }
            >
              {sel.ids.size > 0 ? `Delete ${formatNumber(sel.ids.size)} selected…` : "Delete…"}
            </Button>
          )}
          <Hint label={TSV_HINT}>
            <Button variant="quiet" className="min-w-[124px]" disabled={rows.length === 0} onClick={doCopy}>
              {copy === "copied" ? <Check aria-hidden="true" className="text-icon" /> : <ClipboardCopy aria-hidden="true" className="text-icon" />}
              <span aria-live="polite">{copy === "copied" ? "Copied" : copy === "failed" ? "Couldn't copy" : "Copy as TSV"}</span>
            </Button>
          </Hint>
        </div>
      </div>
      {failures.length > 0 && (
        <div className="shrink-0 border-b border-line px-3 py-2.5">
          <Problem
            title={`Couldn't list the subscriptions of ${plural(failures.length, "topic", "topics")}`}
            body={
              <>
                {failures.slice(0, MAX_TOPICS_SHOWN).map((f) => f.topic).join(", ")}
                {failures.length > MAX_TOPICS_SHOWN && ` and ${formatNumber(failures.length - MAX_TOPICS_SHOWN)} more`}. {kinds.length === 1 ? "Reason" : "Reasons"}: {kinds.join(", ")}.
                {kinds.includes("forbidden") && ` ${accessAdvice(rightsOf(failures.find((f) => f.kind === "forbidden")!.message, "Manage"), auth)}`}
              </>
            }
            detail={kinds.slice(0, MAX_REASONS_SHOWN).map((k) => failures.find((f) => f.kind === k)!.message).join(" | ")}
            action={
              <>
                {kinds.includes("forbidden") && conn.profileId && <Button onClick={() => openSheet({ kind: "profile", profileId: conn.profileId! })}>Edit connection</Button>}
                {kinds.some((k) => k !== "forbidden") && <Button onClick={() => void q.refetch()}>Try again</Button>}
              </>
            }
          />
        </div>
      )}
      {q.isError && data && (
        <div className="shrink-0 border-b border-line px-3 py-2.5">
          <Problem
            title="Couldn't refresh the dashboard"
            body="Showing the last loaded data."
            detail={q.error.message}
            action={<Button onClick={() => void q.refetch()}>Try again</Button>}
          />
        </div>
      )}
      <div className="min-h-0 flex-1">
        <Picks.Provider value={picks}>
        <DataGrid<DashboardRow>
          label="Dashboard"
          columns={columns}
          rows={rows}
          getRowId={(r) => r.id}
          rowLabel={(r) => pathOf(r).replace(/^\//, "")}
          rowNumbers
          selectedIds={sel.ids}
          focusId={sel.focus}
          onSelectionChange={(ids, focus) => setSel({ ids, focus })}
          onActivate={open}
          rowHeight={rowHeight[prefs.density]}
          sorting={sorting}
          onSortingChange={setSorting}
          loading={q.isPending}
          error={
            q.isError && !data ? (
              <EmptyState
                tone="error"
                title="Couldn't load the dashboard"
                body="Try again. If it keeps failing, the detail below says why."
                detail={q.error.message}
                action={<Button onClick={() => void q.refetch()}>Try again</Button>}
              />
            ) : undefined
          }
          empty={
            needle ? (
              <EmptyState
                title="No match"
                body="Change the filter to see more entities."
                action={<Button onClick={() => { setFilter(""); setShow("all"); }}>Clear filters</Button>}
              />
            ) : onlyDlq ? (
              <EmptyState
                title={stats.unknown > 0 ? "No dead letters found" : "No dead letters anywhere"}
                body={
                  stats.unknown > 0
                    ? `${stats.unknown === 1 ? "1 entity has" : `${formatNumber(stats.unknown)} entities have`} unknown counts, so ${stats.unknown === 1 ? "it" : "they"} can't be checked.`
                    : "Every queue and subscription has an empty dead-letter queue."
                }
                action={<Button onClick={() => setShow("all")}>Show all</Button>}
              />
            ) : (
              <EmptyState title="No queues or subscriptions" body="This namespace has no queue, and no topic with a subscription." />
            )
          }
          initialRect={{ width: 1000, height: 600 }}
        />
        </Picks.Provider>
      </div>
      <div title={data?.limited ?? undefined} className="flex min-h-[22px] shrink-0 items-center border-t border-line px-3 py-0.5 text-11 text-muted">
        <span className="truncate tabular-nums">{status}</span>
      </div>
    </div>
  );
}
