import { CircleAlert, Columns3, Filter, RefreshCw, Square } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { TAIL_MAX, type PeekFrom } from "@/engine";
import {
  Button, DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
  Hint, Input, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, cn, formatNumber,
} from "@/ui";
import { COLUMN_LABEL, propColumn, type ColumnId } from "./columns";

export const TOP_N = [50, 100, 500, 1000, 5000, 10000, 50000] as const;

export function Toolbar(p: {
  from: PeekFrom;
  onFrom(f: PeekFrom): void;
  topN: number;
  onTopN(n: number): void;
  filterCount: number;
  filterOpen: boolean;
  /** Shown once the view has a filter bar to open. */
  onToggleFilter?(): void;
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
  exportMenu?: ReactNode;
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
  const toggle = (id: ColumnId) => p.onColumns(p.columns.includes(id) ? p.columns.filter((c) => c !== id) : [...p.columns, id]);
  return (
    <div role="toolbar" aria-label="Peek options" className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3">
      <span className="text-12 text-muted">From</span>
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
      <span className="ml-2 text-12 text-muted">Top</span>
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
      <div className="ml-auto flex items-center gap-1">
        {p.onToggleFilter && (
          <Button variant="quiet" aria-pressed={p.filterOpen} onClick={p.onToggleFilter} className={cn(p.filterOpen && "bg-selection")}>
            <Filter aria-hidden="true" className="text-icon" />
            Filter
            {p.filterCount > 0 && <span className="tabular-nums text-muted">{p.filterCount}</span>}
          </Button>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="quiet">
              <Columns3 aria-hidden="true" className="text-icon" />
              Columns
            </Button>
          </DropdownMenuTrigger>
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
        {p.exportMenu}
        {/* One slot, as wide as its wider label, so swapping Peek again and Stop never moves Columns. */}
        <div className="flex min-w-[96px] justify-end">
        {p.streaming ? (
          <Button variant="quiet" onClick={p.onStop}>
            <Square aria-hidden="true" className="text-icon" />
            Stop
          </Button>
        ) : (
          <Hint label="Peek again" shortcut="f5">
            <Button variant="quiet" onClick={p.onRefresh}>
              <RefreshCw aria-hidden="true" className="text-icon" />
              Peek again
            </Button>
          </Hint>
        )}
        </div>
      </div>
    </div>
  );
}
