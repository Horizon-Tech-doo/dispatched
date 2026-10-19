import { useMemo, useState, type KeyboardEvent } from "react";
import { useEntities } from "@/engine";
import { EntityGlyph, Input, cn } from "@/ui";
import { useShell } from "../context";

/** Where messages can go: the namespace's queues and topics (never a subscription), without `exclude`. A listbox
 *  with a filter field; arrow keys move the choice. */
export function EntityPicker({ label, value, onChange, exclude }: { label: string; value: string | null; onChange(v: string): void; exclude?: string }) {
  const { conn } = useShell();
  const entities = useEntities(conn.profileId);
  const [q, setQ] = useState("");
  const all = useMemo(() => {
    const l = entities.data;
    if (!l) return [];
    const known = l.known.filter((k) => !k.subscription).map((k) => ({ name: k.entity, kind: "unknown" as const }));
    const list = [...l.queues.map((x) => ({ name: x.name, kind: "queue" as const })), ...l.topics.map((x) => ({ name: x.name, kind: "topic" as const })), ...known];
    return list.filter((e, i) => e.name !== exclude && list.findIndex((o) => o.name === e.name) === i);
  }, [entities.data, exclude]);
  const shown = all.filter((e) => e.name.toLowerCase().includes(q.trim().toLowerCase()));
  const move = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const i = shown.findIndex((x) => x.name === value);
    const next = shown[Math.max(0, Math.min(shown.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
    if (next) onChange(next.name);
  };
  return (
    <div className="flex flex-col gap-1">
      <Input aria-label={`Filter ${label.toLowerCase()} entities`} placeholder="Filter" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={move} />
      <div
        role="listbox"
        aria-label={label}
        tabIndex={0}
        aria-activedescendant={value ? `pick-${value}` : undefined}
        onKeyDown={move}
        className="max-h-44 overflow-y-auto rounded-ctl border border-line bg-field py-0.5 focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg"
      >
        {entities.isPending && <p className="m-0 px-2 py-1 text-12 text-muted">Loading…</p>}
        {!entities.isPending && shown.length === 0 && <p className="m-0 px-2 py-1 text-12 text-muted">No match</p>}
        {shown.map((e) => (
          <div
            key={e.name}
            id={`pick-${e.name}`}
            role="option"
            aria-selected={e.name === value}
            onClick={() => onChange(e.name)}
            className={cn("flex h-row cursor-default items-center gap-1.5 px-2 text-12", e.name === value ? "bg-selection" : "hover:bg-hover")}
          >
            <EntityGlyph kind={e.kind} className="shrink-0 text-icon" />
            <span className="truncate">{e.name}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
