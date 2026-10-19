import { useQuery } from "@tanstack/react-query";
import { createColumnHelper } from "@tanstack/react-table";
import { CircleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { commands, type ItemOutcome, type ItemView } from "@/engine";
import { Button, DataGrid, FieldMessage, Segmented, formatNumber, rowHeight } from "@/ui";
import { useShell } from "../context";
import type { Job } from "../drawer/activity";

const OUTCOME: Record<ItemOutcome, string> = {
  succeeded: "Done", failed: "Failed", skipped: "Skipped", missing: "Missing", "not-reached": "Not reached",
};
/** A page holds at most this many items. */
const PAGE = 1000;
type Filter = "all" | "failed" | "problems";

type Row = ItemView & { key: number };
const col = createColumnHelper<Row>();

/** A finished job's report: the core's summary line, its notes, and every item with its outcome, filtered to the
 *  failures or to everything not done. "Save report…" writes it to a file through the save dialog. */
export function ReportView({ job }: { job: Job }) {
  const { prefs } = useShell();
  const r = job.report;
  const [filter, setFilter] = useState<Filter>("all");
  const [selected, setSelected] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  const [saved, setSaved] = useState<{ ok: true; name: string } | { ok: false; message: string } | null>(null);
  const page = useQuery({
    queryKey: ["job-report", job.id, filter !== "all"],
    queryFn: () => commands.jobReport(job.id, 0, PAGE, filter !== "all"),
    enabled: r !== null,
    staleTime: Infinity,
  });
  const rows = useMemo<Row[]>(
    () => (page.data?.items ?? []).filter((i) => filter !== "failed" || i.outcome === "failed").map((i, key) => ({ ...i, key })),
    [page.data, filter],
  );
  const moves = r !== null && ["send", "import", "move", "resubmit"].includes(job.kind);
  const columns = useMemo(
    () => [
      col.accessor("outcome", {
        header: "Outcome",
        meta: { width: "88px" },
        cell: (c) => (
          <span className="inline-flex items-center gap-1">
            {c.getValue() === "failed" && <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />}
            {OUTCOME[c.getValue()]}
          </span>
        ),
      }),
      col.accessor((i) => i.sequence_number ?? i.message_id ?? "", { id: "seq", header: "Seq", meta: { width: "minmax(72px, 112px)", mono: true } }),
      col.accessor((i) => i.reason ?? "", { id: "reason", header: "Reason", meta: { width: "minmax(120px, 1fr)" } }),
      ...(moves ? [col.accessor((i) => i.new_sequence_number ?? "", { id: "new", header: "New seq", meta: { width: "minmax(72px, 112px)", mono: true } })] : []),
    ],
    [moves],
  );

  if (!r) return job.summary ? <p className="m-0 text-12 text-fg">{job.summary}</p> : null;
  const save = async () => {
    setSaved(null);
    try {
      const pick = await commands.pickReportPath(job.id, "json");
      if (!pick) return;
      await commands.jobReportSave(job.id, pick.pick, "json");
      setSaved({ ok: true, name: pick.name });
    } catch (e) {
      setSaved({ ok: false, message: e instanceof Error ? e.message : String(e) });
    }
  };
  const total = page.data?.total ?? 0;
  return (
    <div className="flex flex-col gap-3">
      <p className="m-0 select-text break-words text-12 text-fg" data-selectable>
        {r.summary_line}
      </p>
      {r.notes.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0 text-11 text-muted">
          {r.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      )}
      {r.items > 0 && (
        <>
          <Segmented
            label="Show items"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: `All ${formatNumber(r.items)}` },
              { value: "failed", label: `Failed ${formatNumber(r.failed)}` },
              { value: "problems", label: `Not done ${formatNumber(r.incomplete)}` },
            ]}
          />
          <div className="h-[280px] min-h-0 border border-line">
            <DataGrid
              label="Items"
              columns={columns}
              rows={rows}
              getRowId={(i) => String(i.key)}
              selectedIds={selected.ids}
              focusId={selected.focus}
              onSelectionChange={(ids, focus) => setSelected({ ids, focus })}
              rowHeight={rowHeight[prefs.density]}
              loading={page.isPending}
              initialRect={{ width: 388, height: 280 }}
              empty={<p className="m-0 px-3 py-2 text-12 text-muted">{filter === "failed" ? "Nothing failed." : "Everything was done."}</p>}
            />
          </div>
          {total > PAGE && <p className="m-0 text-11 text-muted">Showing the first {formatNumber(PAGE)} of {formatNumber(total)}. Save the report for all of them.</p>}
        </>
      )}
      <div className="flex flex-col items-start gap-1.5">
        <Button size="sm" onClick={() => void save()}>
          Save report…
        </Button>
        {saved?.ok && <p className="m-0 text-11 text-muted">Saved <span className="font-mono">{saved.name}</span></p>}
        {saved && !saved.ok && <FieldMessage error="Couldn't save the report." detail={saved.message} />}
      </div>
    </div>
  );
}
