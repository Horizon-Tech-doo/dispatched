import { Plus, X } from "lucide-react";
import type { PropType, PropertyForm } from "@/engine";
import { Button, FieldMessage, IconButton, Input, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/ui";
import { PROP_TYPES, checkValue } from "./props";

/** One custom property as edited. `locked`: it came from a message and cannot be sent from here (the reason why);
 *  `was`: the type it had, when it is sent as another one. */
export interface PropRow {
  key: number;
  name: string;
  type: PropType;
  value: string;
  locked?: string;
  was?: string;
}

/** Problems by row key: a value its type refuses, a missing name, a name set twice. Locked rows are not sent. */
export function propertyErrors(rows: PropRow[]): Map<number, string> {
  const out = new Map<number, string>();
  const seen = new Set<string>();
  for (const r of rows) {
    if (r.locked) continue;
    if (!r.name.trim()) {
      if (r.value.trim()) out.set(r.key, "Give the property a name.");
      continue;
    }
    if (seen.has(r.name)) {
      out.set(r.key, `${r.name} is set twice.`);
      continue;
    }
    seen.add(r.name);
    const bad = checkValue(r.type, r.value);
    if (bad) out.set(r.key, bad);
  }
  return out;
}

/** The rows a form sends: named, not locked. Values go as typed, so a long stays exact. */
export const propertyForms = (rows: PropRow[]): PropertyForm[] =>
  rows.filter((r) => !r.locked && r.name.trim()).map((r) => ({ name: r.name, type: r.type, value: r.value }));

let nextKey = 1;
export const newRowKey = () => nextKey++;

/** Typed custom properties: name, type and value per row, each checked as it is typed by the CLI parser's rules. */
export function PropertyEditor({ rows, onChange, label = "Property" }: { rows: PropRow[]; onChange(rows: PropRow[]): void; label?: string }) {
  const errors = propertyErrors(rows);
  const set = (key: number, patch: Partial<PropRow>) => onChange(rows.map((r) => (r.key === key ? { ...r, ...patch } : r)));
  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((r, i) => {
        const n = i + 1;
        const error = errors.get(r.key);
        const errorId = `prop-${r.key}-error`;
        if (r.locked)
          return (
            <div key={r.key} role="group" aria-label={`${label} ${n}`} className="flex items-start gap-1.5">
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate font-mono text-11 text-fg">{r.name}</span>
                <span className="text-11 text-muted">Left out, not copyable. {r.locked}</span>
              </span>
              <IconButton label={`Remove ${r.name}`} className="size-5" onClick={() => onChange(rows.filter((x) => x.key !== r.key))}>
                <X />
              </IconButton>
            </div>
          );
        return (
          <div key={r.key} role="group" aria-label={`${label} ${n}`} className="flex flex-col gap-1">
            <div className="grid grid-cols-[minmax(0,1fr)_92px_minmax(0,1.2fr)_20px] items-center gap-1.5">
              <Input mono aria-label={`${label} ${n} name`} placeholder="name" value={r.name} onChange={(e) => set(r.key, { name: e.target.value })} />
              <Select value={r.type} onValueChange={(v) => set(r.key, { type: v as PropType, was: undefined })}>
                <SelectTrigger aria-label={`${label} ${n} type`} className="font-mono text-11">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PROP_TYPES.map((t) => (
                    <SelectItem key={t} value={t} className="font-mono text-11">
                      {t}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Input
                mono
                aria-label={`${label} ${n} value`}
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? errorId : undefined}
                placeholder={r.type === "bool" ? "true or false" : r.type === "datetime" ? "2026-10-17T12:00:00Z" : r.type === "timespan" ? "1h30m" : "value"}
                value={r.value}
                onChange={(e) => set(r.key, { value: e.target.value })}
              />
              <IconButton label={`Remove ${r.name || `${label.toLowerCase()} ${n}`}`} className="size-5" onClick={() => onChange(rows.filter((x) => x.key !== r.key))}>
                <X />
              </IconButton>
            </div>
            {r.was && !error && <p className="m-0 text-11 text-muted">Sent as {r.type}; it was {r.was}.</p>}
            {error && <FieldMessage id={errorId} error={error} />}
          </div>
        );
      })}
      <Button size="sm" variant="quiet" className="self-start" onClick={() => onChange([...rows, { key: newRowKey(), name: "", type: "string", value: "" }])}>
        <Plus aria-hidden="true" className="text-icon" />
        Add property
      </Button>
    </div>
  );
}
