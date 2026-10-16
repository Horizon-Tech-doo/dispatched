import { CircleAlert, Columns3, Filter, RefreshCw, Square, X } from "lucide-react";
import { useEffect, useId, useRef, useState, type ReactNode, type Ref } from "react";
import { TAIL_MAX, type FilterSpec, type PeekFrom } from "@/engine";
import {
  Button, DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
  Hint, IconButton, Input, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, cn, formatNumber,
} from "@/ui";
import { COLUMN_LABEL, propColumn, type ColumnId } from "./columns";
import { filterSummary } from "./filterText";

/** Below this toolbar width the labels of Filter, Columns, Export and Peek again give way to icons (each keeps its
 *  name and gets a tooltip). A width of 0 (not laid out) counts as wide. */
export const COMPACT_BELOW = 760;

function useCompact(ref: { current: HTMLElement | null }): boolean {
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const measure = () => setCompact(el.clientWidth > 0 && el.clientWidth < COMPACT_BELOW);
    measure();
    const sizes = new ResizeObserver(measure);
    sizes.observe(el);
    return () => sizes.disconnect();
  }, [ref]);
  return compact;
}

/** A button's label, or, when compact, nothing beside its icon (the button carries `aria-label` and a tooltip then). */
export function CompactLabel({ compact, children }: { compact: boolean; children: ReactNode }) {
  return compact ? null : <>{children}</>;
}

/** Wraps a compact button in a tooltip carrying its name. */
export function CompactHint({ compact, label, children }: { compact: boolean; label: string; children: ReactNode }) {
  return compact ? <Hint label={label}>{children}</Hint> : <>{children}</>;
}

export const TOP_N = [50, 100, 500, 1000, 5000, 10000, 50000] as const;

export function Toolbar(p: {
  from: PeekFrom;
  onFrom(f: PeekFrom): void;
  topN: number;
  onTopN(n: number): void;
  filter: FilterSpec;
  filterCount: number;
  filterOpen: boolean;
  /** The filter bar's id, for the toggle's `aria-controls`. */
  filterBarId?: string;
  onClearFilter(): void;
  onToggleFilter(): void;
  /** For returning focus to the toggle when the filter bar closes with Escape. */
  filterToggleRef?: Ref<HTMLButtonElement>;
  columns: ColumnId[];
  /** The built-in columns this view can fill, in menu order. */
  offered: Exclude<ColumnId, `prop:${string}`>[];
  onColumns(ids: ColumnId[]): void;
  onResetColumns(): void;
  propertyNames: string[];
  /** Pages are on their way: the Peek again slot is Stop. */
  streaming: boolean;
  onStop(): void;
  onRefresh(): void;
  /** The Export control; told whether the toolbar is compact, to show an icon alone. */
  exportMenu?: (compact: boolean) => ReactNode;
}) {
  const [seq, setSeq] = useState(p.from.kind === "sequence" ? String(p.from.sequence) : "");
  // Choosing Sequence shows the field; the peek starts there once a number is entered.
  const [picking, setPicking] = useState(false);
  const [seqError, setSeqError] = useState<string | null>(null);
  const errorId = useId();
  const mode = picking ? "sequence" : p.from.kind;
  const applySeq = () => {
    const text = seq.trim();
    if (text === "") return;
    const n = Number(text);
    // Digits only reach here; past 2^53 a JavaScript number is no longer the number typed.
    if (!Number.isSafeInteger(n)) {
      setSeqError("Too large to type here. Use End, or a smaller number.");
      return;
    }
    setSeqError(null);
    setPicking(false);
    if (p.from.kind !== "sequence" || p.from.sequence !== n) p.onFrom({ kind: "sequence", sequence: n });
  };
  const bar = useRef<HTMLDivElement>(null);
  const compact = useCompact(bar);
  const summary = filterSummary(p.filter);
  const toggle = (id: ColumnId) => p.onColumns(p.columns.includes(id) ? p.columns.filter((c) => c !== id) : [...p.columns, id]);
  return (
    <div ref={bar} role="toolbar" aria-label="Peek options" className={cn("flex min-h-8 shrink-0 flex-wrap items-center gap-y-0.5 border-b border-line px-3 py-0.5", compact ? "gap-x-1.5" : "gap-x-2")}>
      {!compact && <span className="text-12 text-muted">From</span>}
      <Segmented
        label="Peek from"
        value={mode}
        onChange={(v) => {
          if (v === "sequence") {
            setPicking(true);
            return;
          }
          setPicking(false);
          setSeqError(null);
          p.onFrom({ kind: v as "start" | "end" });
        }}
        options={[
          { value: "start", label: "Start" },
          { value: "sequence", label: "Sequence" },
          { value: "end", label: "End" },
        ]}
      />
      {mode === "sequence" && (
        <Input
          mono
          aria-label="From sequence number"
          aria-invalid={seqError ? true : undefined}
          aria-describedby={seqError ? errorId : undefined}
          inputMode="numeric"
          placeholder="Sequence number"
          className="w-[132px]"
          value={seq}
          onChange={(e) => {
            setSeq(e.target.value.replace(/[^\d]/g, ""));
            setSeqError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") applySeq();
            if (e.key === "Escape" && picking) {
              setPicking(false);
              setSeqError(null);
            }
          }}
          autoFocus={picking}
          onBlur={() => (seq ? applySeq() : setPicking(false))}
        />
      )}
      {seqError && (
        <span id={errorId} role="alert" className="flex min-w-0 items-center gap-1.5 text-12 text-fg">
          <CircleAlert aria-hidden="true" className="size-3.5 shrink-0 text-accent" />
          <span className="truncate">{seqError}</span>
        </span>
      )}
      {!compact && <span className="ml-2 text-12 text-muted">Top</span>}
      <Select value={String(p.topN)} onValueChange={(v) => p.onTopN(Number(v))}>
        <SelectTrigger aria-label="Messages to read" className="w-[84px] font-mono text-11 tabular-nums">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {TOP_N.filter((n) => mode !== "end" || n <= TAIL_MAX).map((n) => (
            <SelectItem key={n} value={String(n)} className="font-mono text-11 tabular-nums">
              {formatNumber(n)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <div className="ml-auto flex min-w-0 items-center gap-1">
        <CompactHint compact={compact} label="Filter">
          <Button
            ref={p.filterToggleRef}
            variant="quiet"
            aria-label={compact ? (p.filterCount > 0 ? `Filter, ${p.filterCount} applied` : "Filter") : undefined}
            aria-expanded={p.filterOpen}
            aria-controls={p.filterOpen ? p.filterBarId : undefined}
            onClick={p.onToggleFilter}
            className={cn("shrink-0", p.filterOpen && "bg-selection")}
          >
            <Filter aria-hidden="true" className="text-icon" />
            <CompactLabel compact={compact}>Filter</CompactLabel>
            {p.filterCount > 0 && <span className="tabular-nums text-muted">{p.filterCount}</span>}
          </Button>
        </CompactHint>
        {/* Closed with a filter applied: what it is, so the rows are never a mystery. Opens the bar. */}
        {!p.filterOpen && summary && (
          <>
            <Hint label={summary}>
              <button
                type="button"
                aria-label={`Edit filter: ${summary}`}
                onClick={p.onToggleFilter}
                className="min-w-0 max-w-[360px] truncate rounded-ctl px-1 font-mono text-11 text-muted hover:text-fg focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg"
              >
                {summary}
              </button>
            </Hint>
            <IconButton label="Clear filter" onClick={p.onClearFilter}>
              <X />
            </IconButton>
          </>
        )}
        <DropdownMenu>
          <CompactHint compact={compact} label="Columns">
            <DropdownMenuTrigger asChild>
              <Button variant="quiet" aria-label={compact ? "Columns" : undefined}>
                <Columns3 aria-hidden="true" className="text-icon" />
                <CompactLabel compact={compact}>Columns</CompactLabel>
              </Button>
            </DropdownMenuTrigger>
          </CompactHint>
          <DropdownMenuContent align="end" className="max-h-[min(560px,var(--radix-dropdown-menu-content-available-height))] overflow-y-auto">
            {p.offered.map((id) => (
              <DropdownMenuCheckboxItem key={id} checked={p.columns.includes(id)} onCheckedChange={() => toggle(id)} onSelect={(e) => e.preventDefault()}>
                {COLUMN_LABEL[id]}
              </DropdownMenuCheckboxItem>
            ))}
            {p.propertyNames.length > 0 && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuLabel>Application properties</DropdownMenuLabel>
                {p.propertyNames.slice(0, 30).map((name) => (
                  <DropdownMenuCheckboxItem
                    key={name}
                    checked={p.columns.includes(propColumn(name))}
                    onCheckedChange={() => toggle(propColumn(name))}
                    onSelect={(e) => e.preventDefault()}
                  >
                    <span className="truncate font-mono text-11">{name}</span>
                  </DropdownMenuCheckboxItem>
                ))}
              </>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={p.onResetColumns}>Reset columns</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        {p.exportMenu?.(compact)}
        {/* One slot, as wide as its wider label ("Peek again", with its icon and padding), so swapping it with Stop
            never moves Columns. */}
        <div data-peek-slot className={cn("flex justify-end", !compact && "min-w-[104px]")}>
        {p.streaming ? (
          <CompactHint compact={compact} label="Stop">
            <Button variant="quiet" aria-label={compact ? "Stop" : undefined} onClick={p.onStop}>
              <Square aria-hidden="true" className="fill-current text-icon" />
              <CompactLabel compact={compact}>Stop</CompactLabel>
            </Button>
          </CompactHint>
        ) : (
          <Hint label="Peek again" shortcut="f5">
            <Button variant="quiet" aria-label={compact ? "Peek again" : undefined} onClick={p.onRefresh}>
              <RefreshCw aria-hidden="true" className="text-icon" />
              <CompactLabel compact={compact}>Peek again</CompactLabel>
            </Button>
          </Hint>
        )}
        </div>
      </div>
    </div>
  );
}
