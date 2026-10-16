import { flexRender, getCoreRowModel, getSortedRowModel, useReactTable, type ColumnDef, type RowData, type SortingState } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useEffect, useId, useMemo, useRef, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { Skeleton } from "../components/skeleton";
import { cn } from "../lib/cn";
import { observeRectWithFloor } from "../lib/virtual";

export interface GridColumnMeta {
  align?: "start" | "end";
  mono?: boolean;
  /** A CSS grid track, e.g. `"64px"` or `"minmax(190px, 1fr)"`. */
  width?: string;
}

declare module "@tanstack/react-table" {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars, @typescript-eslint/no-empty-object-type
  interface ColumnMeta<TData extends RowData, TValue> extends GridColumnMeta {}
}

export interface GridSort {
  id: string;
  desc: boolean;
}

export interface DataGridProps<R> {
  label: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  columns: ColumnDef<R, any>[];
  rows: R[];
  getRowId(row: R): string;
  selectedIds: ReadonlySet<string>;
  focusId: string | null;
  onSelectionChange(ids: Set<string>, focusId: string | null): void;
  onActivate?(id: string): void;
  rowHeight: number;
  loading?: boolean;
  empty?: ReactNode;
  error?: ReactNode;
  initialRect?: { width: number; height: number };
  /** The sorted column, or null for the incoming order. Headers sort only when `onSortingChange` is given. */
  sorting?: GridSort | null;
  /** Header click cycles ascending, descending, none. */
  onSortingChange?(sort: GridSort | null): void;
  /** A leading `#` column numbering the rows in display order. */
  rowNumbers?: boolean;
}

const HEADER_HEIGHT = 24;
const ROW_NUMBER_WIDTH = "44px";
const SKELETON_WIDTHS = ["w-[62%]", "w-[48%]", "w-[71%]", "w-[39%]", "w-[56%]", "w-[66%]", "w-[44%]", "w-[58%]"];

export function DataGrid<R>(p: DataGridProps<R>) {
  const domId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  // The range anchor is a row id, so it survives re-sorting.
  const anchor = useRef<string | null>(null);

  // Stable identity per value: the sorted row model is memoised on this array, so a fresh one per render re-sorts every render.
  const sortId = p.sorting?.id;
  const sortDesc = p.sorting?.desc;
  const sortingState = useMemo<SortingState>(() => (sortId === undefined ? [] : [{ id: sortId, desc: !!sortDesc }]), [sortId, sortDesc]);
  const table = useReactTable({
    data: p.rows,
    columns: p.columns,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getRowId: p.getRowId,
    manualSorting: false,
    enableSorting: !!p.onSortingChange,
    enableSortingRemoval: true,
    enableMultiSort: false,
    sortDescFirst: false,
    state: { sorting: sortingState },
    onSortingChange: (u) => {
      const next = typeof u === "function" ? u(sortingState) : u;
      p.onSortingChange?.(next[0] ? { id: next[0].id, desc: next[0].desc } : null);
    },
  });
  const rows = table.getRowModel().rows;
  const leafColumns = table.getVisibleLeafColumns();
  const columnTemplate = leafColumns.map((c) => c.columnDef.meta?.width ?? "minmax(80px, 1fr)").join(" ");
  const template = p.rowNumbers ? `${ROW_NUMBER_WIDTH} ${columnTemplate}` : columnTemplate;
  const colCount = leafColumns.length + (p.rowNumbers ? 1 : 0);

  const { initialRect } = p;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => p.rowHeight,
    overscan: 16,
    initialRect,
    // The sticky header sits above the rows inside the scroller.
    scrollMargin: HEADER_HEIGHT,
    scrollPaddingStart: HEADER_HEIGHT,
    observeElementRect: observeRectWithFloor(initialRect),
  });

  useEffect(() => virtualizer.measure(), [p.rowHeight, virtualizer]);

  const indexOf = useMemo(() => new Map(rows.map((r, i) => [r.id, i])), [rows]);
  const focusIndex = p.focusId != null ? (indexOf.get(p.focusId) ?? -1) : -1;
  const idAt = (i: number) => rows[i]?.id;

  function selectOnly(i: number) {
    const id = idAt(i);
    if (id === undefined) return;
    anchor.current = id;
    p.onSelectionChange(new Set([id]), id);
    virtualizer.scrollToIndex(i, { align: "auto" });
  }

  function selectRange(to: number) {
    const from = (anchor.current != null ? indexOf.get(anchor.current) : undefined) ?? (focusIndex >= 0 ? focusIndex : to);
    const [a, b] = from <= to ? [from, to] : [to, from];
    const ids = new Set(rows.slice(a, b + 1).map((r) => r.id));
    p.onSelectionChange(ids, idAt(to) ?? null);
    virtualizer.scrollToIndex(to, { align: "auto" });
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (rows.length === 0) return;
    const mod = e.metaKey || e.ctrlKey;
    const last = rows.length - 1;
    const step = (d: number) => Math.max(0, Math.min(last, (focusIndex < 0 ? (d > 0 ? -1 : 1) : focusIndex) + d));
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (mod && key === "a") {
      e.preventDefault();
      p.onSelectionChange(new Set(rows.map((r) => r.id)), p.focusId);
      return;
    }
    if (mod || e.altKey) return;
    let to: number | null = null;
    const page = Math.max(1, Math.floor((scrollRef.current?.clientHeight || p.rowHeight * 10) / p.rowHeight) - 1);
    if (key === "ArrowDown" || key === "j") to = step(1);
    else if (key === "ArrowUp" || key === "k") to = step(-1);
    else if (key === "PageDown") to = step(page);
    else if (key === "PageUp") to = step(-page);
    else if (key === "Home") to = 0;
    else if (key === "End") to = last;
    else if (key === "Enter" && p.focusId && e.target === e.currentTarget) {
      e.preventDefault();
      p.onActivate?.(p.focusId);
      return;
    }
    if (to === null) return;
    e.preventDefault();
    if (e.shiftKey) selectRange(to);
    else selectOnly(to);
  }

  function onRowClick(e: MouseEvent, i: number) {
    const id = idAt(i);
    if (id === undefined) return;
    if (e.shiftKey) return selectRange(i);
    if (e.metaKey || e.ctrlKey) {
      const next = new Set(p.selectedIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      anchor.current = id;
      return p.onSelectionChange(next, id);
    }
    selectOnly(i);
  }

  const header = table.getHeaderGroups()[0];
  const virtualItems = virtualizer.getVirtualItems();
  const focusRendered = focusIndex >= 0 && virtualItems.some((v) => v.index === focusIndex);
  const loadingRows = (
    <div role="status" aria-label="Loading">
      {Array.from({ length: 10 }, (_, r) => (
        <div key={r} data-skeleton-row className="grid items-center px-2.5" style={{ height: p.rowHeight, gridTemplateColumns: template }}>
          {p.rowNumbers && <div className="px-1"><Skeleton className="ml-auto w-[60%]" /></div>}
          {leafColumns.map((c, ci) => (
            <div key={c.id} className="px-1">
              <Skeleton className={cn(SKELETON_WIDTHS[(r + ci * 3) % SKELETON_WIDTHS.length], c.columnDef.meta?.align === "end" && "ml-auto")} />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
  const state = p.error ?? (p.loading ? loadingRows : rows.length === 0 ? p.empty : null);
  const body = state != null && state !== false ? (
    <div role="row">
      <div role="gridcell" aria-colspan={colCount}>{state}</div>
    </div>
  ) : null;

  return (
    <div
      ref={scrollRef}
      role="grid"
      aria-label={p.label}
      aria-multiselectable="true"
      aria-rowcount={rows.length + 1}
      tabIndex={0}
      aria-busy={p.loading || undefined}
      aria-activedescendant={body == null && focusRendered ? `${domId}-${p.focusId}` : undefined}
      onKeyDown={onKeyDown}
      className="relative h-full overflow-auto focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg"
    >
      {/* Sizes to the header's max-content so absolutely positioned rows span the full scroll width. */}
      <div className="w-max min-w-full">
      <div
        role="row"
        aria-rowindex={1}
        className="sticky top-0 z-10 grid items-center border-b border-line bg-content px-2.5 text-12 text-muted"
        style={{ height: HEADER_HEIGHT, gridTemplateColumns: template }}
      >
        {p.rowNumbers && (
          <div role="columnheader" className="truncate px-1 text-right">
            #
          </div>
        )}
        {header.headers.map((h) => {
          const meta = h.column.columnDef.meta;
          const dir = h.column.getIsSorted();
          const label = flexRender(h.column.columnDef.header, h.getContext());
          return (
            <div
              key={h.id}
              role="columnheader"
              aria-sort={dir === "asc" ? "ascending" : dir === "desc" ? "descending" : undefined}
              className={cn("truncate px-1", meta?.align === "end" && "text-right")}
            >
              {h.column.getCanSort() ? (
                <button
                  type="button"
                  onClick={h.column.getToggleSortingHandler()}
                  className={cn(
                    "inline-flex max-w-full items-center gap-1 rounded-ctl text-muted hover:text-fg focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg",
                    meta?.align === "end" && "flex-row-reverse",
                  )}
                >
                  <span className="truncate">{label}</span>
                  {dir === "asc" && <ArrowUp aria-hidden="true" className="size-3 shrink-0 text-icon" />}
                  {dir === "desc" && <ArrowDown aria-hidden="true" className="size-3 shrink-0 text-icon" />}
                </button>
              ) : (
                label
              )}
            </div>
          );
        })}
      </div>
      {body ?? (
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualItems.map((v) => {
            const row = rows[v.index];
            const selected = p.selectedIds.has(row.id);
            const focused = row.id === p.focusId;
            return (
              <div
                key={row.id}
                id={`${domId}-${row.id}`}
                role="row"
                data-row-id={row.id}
                aria-rowindex={v.index + 2}
                aria-selected={selected}
                onClick={(e) => onRowClick(e, v.index)}
                onDoubleClick={() => p.onActivate?.(row.id)}
                className={cn(
                  "absolute inset-x-0 grid items-center px-2.5 text-12",
                  selected ? "bg-selection" : "hover:bg-hover",
                  focused && !(selected && p.selectedIds.size === 1) && "outline outline-1 -outline-offset-1 outline-muted",
                )}
                style={{ height: p.rowHeight, transform: `translateY(${v.start - HEADER_HEIGHT}px)`, gridTemplateColumns: template }}
              >
                {p.rowNumbers && (
                  <div role="gridcell" className="truncate px-1 text-right font-mono text-11 tabular-nums text-muted">
                    {v.index + 1}
                  </div>
                )}
                {row.getVisibleCells().map((c) => {
                  const meta = c.column.columnDef.meta;
                  return (
                    <div
                      key={c.id}
                      role="gridcell"
                      className={cn("truncate px-1", meta?.align === "end" && "text-right tabular-nums", meta?.mono && "font-mono text-11 tabular-nums")}
                    >
                      {flexRender(c.column.columnDef.cell, c.getContext())}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      )}
      </div>
    </div>
  );
}
