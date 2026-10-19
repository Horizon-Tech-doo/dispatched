import { useState } from "react";
import type { FilterSpec, PeekView, Selection } from "@/engine";
import { Field, Input, formatNumber } from "@/ui";
import { filterSummary } from "../messages/filterText";
import { VIEW_LABEL } from "../tabs/subtabs";

/** How many messages a "matching" selection may take when the view's count is unknown. */
export const DEFAULT_LIMIT = 1000;

const plural = (n: number, one: string) => `${formatNumber(n)} ${n === 1 ? one : `${one}s`}`;

/** The messages an operation acts on: the grid's selected rows, or (none selected) every message matching the view's
 *  filter, up to a limit. Sequence numbers become strings here, at the engine call (M1c keeps them as numbers). */
export function useSelection(p: { sequences: number[]; filter: FilterSpec; total?: number | null }) {
  const [limit, setLimit] = useState(String(p.total && p.total > 0 ? p.total : DEFAULT_LIMIT));
  const n = /^\d+$/.test(limit.trim()) ? Number(limit.trim()) : NaN;
  const limitError = p.sequences.length || (Number.isSafeInteger(n) && n >= 1) ? null : "Take at least one message.";
  const selection: Selection = p.sequences.length
    ? { type: "sequences", sequences: p.sequences.map(String) }
    : { type: "matching", filter: p.filter, limit: Number.isSafeInteger(n) ? n : 0 };
  return { selection, limit, setLimit, limitError, count: p.sequences.length || null };
}

/** What a selection is, in words, with the limit field for a "matching" one. */
export function SelectionSummary({
  entity, view, sequences, filter, limit, setLimit, limitError,
}: {
  entity: string;
  view: PeekView;
  sequences: number[];
  filter: FilterSpec;
  limit: string;
  setLimit(v: string): void;
  limitError: string | null;
}) {
  const text = filterSummary(filter);
  if (sequences.length)
    return (
      <p className="m-0 text-12 text-muted">
        {plural(sequences.length, "selected message")} from the {VIEW_LABEL[view]} of <span className="text-fg">{entity}</span>, found again by sequence
        number when the job runs.
      </p>
    );
  return (
    <div className="flex flex-col gap-1.5">
      <p className="m-0 text-12 text-muted">
        Every message in the {VIEW_LABEL[view]} of <span className="text-fg">{entity}</span>
        {text ? " that matches the current filter" : ""}, up to the limit. The view is searched when the job runs.
      </p>
      {text && (
        <p className="m-0 break-words font-mono text-11 text-muted" data-selectable>
          {text}
        </p>
      )}
      <Field label="At most" inline={72} error={limitError ?? undefined}>
        {(ids) => <Input {...ids} type="number" min={1} mono className="w-28" value={limit} onChange={(e) => setLimit(e.target.value)} />}
      </Field>
    </div>
  );
}

/** "Delete 3 messages", or "Delete matching messages" for a selection by filter. */
export const verbLabel = (verb: string, count: number | null) => (count ? `${verb} ${plural(count, "message")}` : `${verb} matching messages`);
