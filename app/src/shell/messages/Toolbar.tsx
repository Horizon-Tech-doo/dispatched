import { Check, Columns3, Filter, RefreshCw } from "lucide-react";
import { DropdownMenu as M } from "radix-ui";
import { useState, type ComponentProps, type ReactNode } from "react";
import { TAIL_MAX, type PeekFrom } from "@/engine";
import {
  Button, DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger, Hint, IconButton, Input,
  Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, cn, formatNumber, menuItem,
} from "@/ui";
import { COLUMN_LABEL, OPTIONAL, propColumn, type ColumnId } from "./columns";

export const TOP_N = [50, 100, 500, 1000, 5000, 10000, 50000] as const;

/** A menu row with a check mark. Belongs in `ui/components/dropdown-menu.tsx` as `DropdownMenuCheckboxItem`;
 *  kept here while `ui/` is being edited by another task. */
function CheckboxItem({ className, children, ...props }: ComponentProps<typeof M.CheckboxItem>) {
  return (
    <M.CheckboxItem className={cn(menuItem, "pl-7", className)} {...props}>
      <span className="absolute left-2.5 inline-flex size-3 items-center justify-center">
        <M.ItemIndicator>
          <Check aria-hidden="true" className="!size-3" />
        </M.ItemIndicator>
      </span>
      {children}
    </M.CheckboxItem>
  );
}

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
  onColumns(ids: ColumnId[]): void;
  propertyNames: string[];
  onRefresh(): void;
  exportMenu?: ReactNode;
}) {
  const [seq, setSeq] = useState(p.from.kind === "sequence" ? String(p.from.sequence) : "");
  // Choosing Sequence shows the field; the peek starts there once a number is entered.
  const [picking, setPicking] = useState(false);
  const mode = picking ? "sequence" : p.from.kind;
  const applySeq = () => {
    const n = Number(seq.trim());
    if (seq.trim() !== "" && Number.isSafeInteger(n) && n >= 0) {
      setPicking(false);
      if (p.from.kind !== "sequence" || p.from.sequence !== n) p.onFrom({ kind: "sequence", sequence: n });
    }
  };
  const toggle = (id: ColumnId) => p.onColumns(p.columns.includes(id) ? p.columns.filter((c) => c !== id) : [...p.columns, id]);
  return (
    <div role="toolbar" aria-label="Peek options" className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-3">
      <span className="text-12 text-muted">From</span>
      <Segmented
        label="Peek from"
        value={mode}
        onChange={(v) => {
          if (v === "sequence") return seq ? applySeq() : setPicking(true);
          setPicking(false);
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
          inputMode="numeric"
          placeholder="Sequence number"
          className="w-[132px]"
          value={seq}
          onChange={(e) => setSeq(e.target.value.replace(/[^\d]/g, ""))}
          onKeyDown={(e) => {
            if (e.key === "Enter") applySeq();
            if (e.key === "Escape" && picking) setPicking(false);
          }}
          autoFocus={picking}
          onBlur={() => (seq ? applySeq() : setPicking(false))}
        />
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
          <DropdownMenuContent align="end" className="max-h-[min(420px,var(--radix-dropdown-menu-content-available-height))] overflow-y-auto">
            <DropdownMenuLabel>Columns</DropdownMenuLabel>
            {OPTIONAL.map((id) => (
              <CheckboxItem key={id} checked={p.columns.includes(id)} onCheckedChange={() => toggle(id)} onSelect={(e) => e.preventDefault()}>
                {COLUMN_LABEL[id]}
              </CheckboxItem>
            ))}
            {p.propertyNames.length > 0 && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuLabel>Application properties</DropdownMenuLabel>
                {p.propertyNames.slice(0, 30).map((name) => (
                  <CheckboxItem
                    key={name}
                    checked={p.columns.includes(propColumn(name))}
                    onCheckedChange={() => toggle(propColumn(name))}
                    onSelect={(e) => e.preventDefault()}
                  >
                    <span className="truncate font-mono text-11">{name}</span>
                  </CheckboxItem>
                ))}
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
        {p.exportMenu}
        <Hint label="Peek again" shortcut="f5">
          <IconButton label="Peek again" onClick={p.onRefresh}>
            <RefreshCw />
          </IconButton>
        </Hint>
      </div>
    </div>
  );
}
