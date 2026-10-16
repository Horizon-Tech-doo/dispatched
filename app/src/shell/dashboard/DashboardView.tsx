import type { ColumnDef } from "@tanstack/react-table";
import { Check, ClipboardCopy, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useDashboard, type DashboardRow } from "@/engine";
import { Button, CountBadge, DataGrid, EmptyState, EntityGlyph, IconButton, Input, Problem, Segmented, cn, formatClock, formatNumber, rowHeight, type GridSort } from "@/ui";
import { useShell } from "../context";
import { DASHBOARD_REFRESH, type DashboardRefresh } from "../state";
import { toTsv } from "./tsv";

const REFRESH_LABEL: Record<DashboardRefresh, string> = { 0: "Off", 30: "30 s", 60: "1 min", 300: "5 min" };
const FLASH_MS = 1800;

/** A count as data: zero is muted, a number above zero is the primary colour, and only dead letters turn amber. */
function countColumn(id: string, header: string, pick: (r: DashboardRow) => number | undefined, attention = false): ColumnDef<DashboardRow, unknown> {
  return {
    id,
    header,
    // Unknown counts sort below zero.
    accessorFn: (r) => pick(r) ?? -1,
    meta: { align: "end", width: "minmax(96px, 120px)" },
    cell: (c) => {
      const v = pick(c.row.original);
      return <CountBadge value={v} quality={c.row.original.counts ?? undefined} attention={attention} className={cn("text-12", !attention && v != null && v > 0 && "text-fg")} />;
    },
  };
}

const plural = (n: number, one: string, many: string) => `${formatNumber(n)} ${n === 1 ? one : many}`;
const pathOf = (r: DashboardRow) => `${r.topic ?? ""}/${r.name}`;

export function DashboardView() {
  const { conn, prefs, setPrefs, dispatch, openSheet } = useShell();
  const q = useDashboard(conn.profileId, prefs.dashboardRefresh * 1000);
  const [sorting, setSorting] = useState<GridSort | null>({ id: "dlq", desc: true });
  const [onlyDlq, setOnlyDlq] = useState(false);
  const [filter, setFilter] = useState("");
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");
  const flash = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(flash.current), []);

  const data = q.data;
  // In the limited view the namespace could not be listed, so only names are known: a topic named in the connection
  // string's EntityPath arrives as a queue row. Such rows get no kind glyph, and no kind in the TSV.
  const kindsKnown = data?.limited == null;
  const columns = useMemo<ColumnDef<DashboardRow, unknown>[]>(
    () => [
      {
        id: "entity",
        header: "Entity",
        accessorFn: pathOf,
        meta: { width: "minmax(240px, 2fr)" },
        cell: (c) => {
          const r = c.row.original;
          const glyph = kindsKnown || r.kind === "subscription" ? r.kind : null;
          return (
            <span className="flex min-w-0 items-center gap-1.5">
              {glyph ? <EntityGlyph kind={glyph} className="size-3.5 shrink-0 text-icon" /> : <span aria-hidden="true" className="size-3.5 shrink-0" />}
              <span className="min-w-0 truncate" title={r.topic ? `${r.topic}/${r.name}` : r.name}>{r.name}</span>
              {r.topic && <span className="min-w-0 truncate text-muted">{r.topic}</span>}
            </span>
          );
        },
      },
      {
        id: "status",
        header: "Status",
        accessorFn: (r) => r.status ?? "",
        meta: { width: "minmax(100px, 130px)" },
        cell: (c) => {
          const s = c.row.original.status;
          return s ? <span className={s === "Active" ? "text-muted" : undefined}>{s}</span> : null;
        },
      },
      countColumn("active", "Active", (r) => r.counts?.active),
      countColumn("dlq", "Dead-letter", (r) => r.counts?.dead_letter, true),
      countColumn("scheduled", "Scheduled", (r) => r.counts?.scheduled),
      countColumn("transfer", "Transfer", (r) => r.counts?.transfer),
      countColumn("tdlq", "Transfer DLQ", (r) => r.counts?.transfer_dead_letter, true),
    ],
    [kindsKnown],
  );

  const all = data?.rows;
  const needle = filter.trim().toLowerCase();
  const rows = useMemo(
    () => (all ?? []).filter((r) => (!onlyDlq || (r.counts?.dead_letter ?? 0) > 0) && (!needle || pathOf(r).toLowerCase().includes(needle))),
    [all, onlyDlq, needle],
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
    navigator.clipboard.writeText(toTsv(rows, { kindsKnown })).then(() => done("copied"), () => done("failed"));
  };

  const failures = data?.failures ?? [];
  const withDlq = (all ?? []).filter((r) => (r.counts?.dead_letter ?? 0) > 0).length;
  const approx = (all ?? []).some((r) => r.counts?.approximate && !r.counts.truncated);
  const capped = (all ?? []).some((r) => r.counts?.truncated);
  const filtered = onlyDlq || needle !== "";
  const status = data
    ? [
        filtered ? `${formatNumber(rows.length)} of ${plural(all!.length, "entity", "entities")}` : plural(all!.length, "entity", "entities"),
        ...(withDlq > 0 ? [`${formatNumber(withDlq)} with dead letters`] : []),
        ...(approx ? ["~ counts are approximate"] : []),
        ...(capped ? ["~N+ counts are lower bounds"] : []),
        q.isFetching ? "Refreshing" : `Refreshed ${formatClock(new Date(q.dataUpdatedAt).toISOString())}`,
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
        <Button variant="quiet" aria-pressed={onlyDlq} className={cn(onlyDlq && "bg-selection")} onClick={() => setOnlyDlq(!onlyDlq)}>
          Only with dead letters
        </Button>
        <span className="ml-2 text-12 text-muted">Auto-refresh</span>
        <Segmented
          label="Auto-refresh"
          value={String(prefs.dashboardRefresh)}
          onChange={(v) => setPrefs({ dashboardRefresh: Number(v) as DashboardRefresh })}
          options={DASHBOARD_REFRESH.map((s) => ({ value: String(s), label: REFRESH_LABEL[s] }))}
        />
        <div className="ml-auto flex items-center gap-1">
          <Button variant="quiet" disabled={rows.length === 0} onClick={doCopy}>
            {copy === "copied" ? <Check aria-hidden="true" className="text-icon" /> : <ClipboardCopy aria-hidden="true" className="text-icon" />}
            Copy as TSV
          </Button>
          <span role="status" className="min-w-[4.5rem] text-11 text-muted">
            {copy === "copied" ? "Copied" : copy === "failed" ? "Couldn't copy" : ""}
          </span>
          <IconButton label="Refresh the dashboard" onClick={() => void q.refetch()}>
            <RefreshCw />
          </IconButton>
        </div>
      </div>
      {data?.limited && (
        <div className="shrink-0 border-b border-line px-3 py-2.5">
          <Problem
            title="Showing pinned entities only"
            body={data.limited}
            action={conn.profileId ? <Button onClick={() => openSheet({ kind: "profile", profileId: conn.profileId })}>Edit connection</Button> : undefined}
          />
        </div>
      )}
      {failures.length > 0 && (
        <div className="shrink-0 border-b border-line px-3 py-2.5">
          <Problem
            title={`Couldn't list the subscriptions of ${plural(failures.length, "topic", "topics")}`}
            body={failures.map((f) => f.topic).join(", ")}
            detail={failures[0].message}
            action={<Button onClick={() => void q.refetch()}>Try again</Button>}
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
            filtered ? (
              <EmptyState
                title={onlyDlq && !needle ? "No dead letters anywhere" : "No match"}
                body={onlyDlq && !needle ? "Every queue and subscription has an empty dead-letter queue." : "Change the filter to see more entities."}
                action={
                  <Button
                    onClick={() => {
                      setFilter("");
                      setOnlyDlq(false);
                    }}
                  >
                    Clear filters
                  </Button>
                }
              />
            ) : (
              <EmptyState title="No queues or subscriptions" body="This namespace has no queue, and no topic with a subscription." />
            )
          }
          initialRect={{ width: 1000, height: 600 }}
        />
      </div>
      <div role="status" aria-label="Dashboard status" className="flex min-h-[22px] shrink-0 items-center border-t border-line px-3 py-0.5 text-11 text-muted">
        <span className="truncate tabular-nums">{status}</span>
      </div>
    </div>
  );
}
