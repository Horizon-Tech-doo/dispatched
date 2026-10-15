import { flexRender, getCoreRowModel, useReactTable, type ColumnDef, type RowData } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useId, useRef, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
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
}

const HEADER_HEIGHT = 24;
const SKELETON_WIDTHS = ["w-[62%]", "w-[48%]", "w-[71%]", "w-[39%]", "w-[56%]", "w-[66%]", "w-[44%]", "w-[58%]"];

export function DataGrid<R>(p: DataGridProps<R>) {
  const domId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const anchor = useRef<number | null>(null);

  const table = useReactTable({ data: p.rows, columns: p.columns, getCoreRowModel: getCoreRowModel(), getRowId: p.getRowId });
  const rows = table.getRowModel().rows;
  const leafColumns = table.getVisibleLeafColumns();
  const template = leafColumns.map((c) => c.columnDef.meta?.width ?? "minmax(80px, 1fr)").join(" ");

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

  const focusIndex = p.focusId ? rows.findIndex((r) => r.id === p.focusId) : -1;
  const idAt = (i: number) => rows[i]?.id;

  function selectOnly(i: number) {
    const id = idAt(i);
    if (id === undefined) return;
    anchor.current = i;
    p.onSelectionChange(new Set([id]), id);
    virtualizer.scrollToIndex(i, { align: "auto" });
  }

  function selectRange(to: number) {
    const from = anchor.current ?? (focusIndex >= 0 ? focusIndex : to);
    const [a, b] = from <= to ? [from, to] : [to, from];
    const ids = new Set(rows.slice(a, b + 1).map((r) => r.id));
    anchor.current = from;
    p.onSelectionChange(ids, idAt(to) ?? null);
    virtualizer.scrollToIndex(to, { align: "auto" });
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (rows.length === 0) return;
    const mod = e.metaKey || e.ctrlKey;
    const last = rows.length - 1;
    const step = (d: number) => Math.max(0, Math.min(last, (focusIndex < 0 ? (d > 0 ? -1 : 1) : focusIndex) + d));
    if (mod && e.key.toLowerCase() === "a") {
      e.preventDefault();
      p.onSelectionChange(new Set(rows.map((r) => r.id)), p.focusId);
      return;
    }
    if (mod || e.altKey) return;
    let to: number | null = null;
    if (e.key === "ArrowDown" || e.key === "j") to = step(1);
    else if (e.key === "ArrowUp" || e.key === "k") to = step(-1);
    else if (e.key === "Home") to = 0;
    else if (e.key === "End") to = last;
    else if (e.key === "Enter" && p.focusId) {
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
      anchor.current = i;
      return p.onSelectionChange(next, id);
    }
    selectOnly(i);
  }

  const header = table.getHeaderGroups()[0];
  const loadingRows = (
    <div role="status" aria-label="Loading">
      {Array.from({ length: 10 }, (_, r) => (
        <div key={r} data-skeleton-row className="grid items-center px-2.5" style={{ height: p.rowHeight, gridTemplateColumns: template }}>
          {leafColumns.map((c, ci) => (
            <div key={c.id} className="px-1">
              <Skeleton className={cn(SKELETON_WIDTHS[(r + ci * 3) % SKELETON_WIDTHS.length], c.columnDef.meta?.align === "end" && "ml-auto")} />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
  const body = p.error ?? (p.loading ? loadingRows : rows.length === 0 ? p.empty : null);

  return (
    <div
      ref={scrollRef}
      role="grid"
      aria-label={p.label}
      aria-multiselectable="true"
      aria-rowcount={rows.length + 1}
      tabIndex={0}
      aria-activedescendant={p.focusId && focusIndex >= 0 ? `${domId}-${p.focusId}` : undefined}
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
        {header.headers.map((h) => (
          <div key={h.id} role="columnheader" className={cn("truncate px-1", h.column.columnDef.meta?.align === "end" && "text-right")}>
            {flexRender(h.column.columnDef.header, h.getContext())}
          </div>
        ))}
      </div>
      {body ?? (
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((v) => {
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
