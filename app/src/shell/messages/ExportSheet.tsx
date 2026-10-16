import { useState } from "react";
import { EngineError, commands, useStartExport, type ExportFormat, type FilterSpec, type PeekView, type PickedPath } from "@/engine";
import { Button, Problem, Segmented, Sheet, formatNumber } from "@/ui";
import { useShell } from "../context";
import { VIEW_LABEL } from "../tabs/subtabs";
import { filterSummary } from "./filterText";

/** What `export_start` says when the engine no longer holds the file the dialog returned. */
const PICK_SPENT = "Choose where to save the export.";

/** What the user can do about a refused export, by the kind of refusal. The engine's own text goes on the detail line. */
const advice = (kind: string): string =>
  kind === "invalid"
    ? "Check the filter or the selection, then export again. The file you chose is kept."
    : kind === "not_connected"
      ? "The connection has closed. Connect again, then export."
      : "Try again, or choose another file.";

const plural = (n: number, one: string) => `${formatNumber(n)} ${n === 1 ? one : `${one}s`}`;

/** Export the messages of one view to a file: all of them (with the view's filter and session) or the ones selected.
 *  The native save dialog picks the file (and asks about replacing it); the engine keeps the path and the job reports
 *  in the Jobs tab. */
export function ExportSheet(p: { entity: string; view: PeekView; filter: FilterSpec; selected: number[]; total: number | null }) {
  const { conn, closeSheet, dispatch } = useShell();
  const [scope, setScope] = useState<"all" | "selected">(p.selected.length > 0 ? "selected" : "all");
  const [format, setFormat] = useState<ExportFormat>("json");
  // The file the dialog returned, kept when the engine refuses the request so a retry does not ask again. It was
  // named for one format, so another format asks again.
  const [picked, setPicked] = useState<(PickedPath & { format: ExportFormat }) | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ kind: string; message: string } | null>(null);
  const start = useStartExport();
  const held = picked && picked.format === format ? picked : null;
  const filtered = filterSummary(p.filter) !== "";
  const total = !filtered && scope === "all" ? p.total : null;

  const what =
    scope === "selected"
      ? `${plural(p.selected.length, "selected message")} of ${p.entity}, read again when the export runs.`
      : `Every message in the ${VIEW_LABEL[p.view]} of ${p.entity}${filtered ? " that matches the current filter" : ""}${total != null ? ` (about ${formatNumber(total)})` : ""}.`;

  const go = async () => {
    if (!conn.profileId || busy) return;
    setBusy(true);
    setFailure(null);
    try {
      let file = held;
      // A second round only when the engine had dropped the file: the dialog opens again, once.
      for (let round = 0; round < 2; round++) {
        if (!file) {
          const r = await commands.pickExportPath(p.entity, p.view, format);
          if (!r) return;
          file = { ...r, format };
          setPicked(file);
        }
        try {
          await start.mutateAsync({
            profileId: conn.profileId,
            request: {
              entity: p.entity, view: p.view, format, pick: file.pick, total_hint: total,
              scope: scope === "selected" ? { kind: "selected", sequences: p.selected } : { kind: "all", filter: p.filter },
            },
          });
        } catch (e) {
          if (round === 0 && e instanceof EngineError && e.kind === "invalid" && e.message === PICK_SPENT) {
            file = null;
            setPicked(null);
            continue;
          }
          throw e;
        }
        closeSheet();
        dispatch({ type: "layout", patch: { drawerOpen: true, drawerTab: "jobs" } });
        return;
      }
    } catch (e) {
      setFailure({ kind: e instanceof EngineError ? e.kind : "unknown", message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet
      open
      onOpenChange={(o) => !o && closeSheet()}
      title="Export messages"
      description="Peeked again and written to a file on this computer. Nothing is removed from Service Bus."
      footer={
        <>
          <Button onClick={closeSheet}>Cancel</Button>
          <Button variant="primary" disabled={busy || !conn.profileId} onClick={() => void go()}>
            {held ? "Export" : "Choose file and export"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="flex flex-col items-start gap-1.5">
          <span className="text-12 font-semibold text-muted">Messages</span>
          {p.selected.length > 0 && (
            <Segmented
              label="Messages to export"
              value={scope}
              onChange={setScope}
              options={[
                { value: "all", label: "All in this view" },
                { value: "selected", label: `Selected (${formatNumber(p.selected.length)})` },
              ]}
            />
          )}
          <p className="m-0 text-12 text-muted">{what}</p>
          {scope === "all" && filtered && (
            <p className="m-0 break-words font-mono text-11 text-muted" data-selectable>
              {filterSummary(p.filter)}
            </p>
          )}
        </div>
        <div className="flex flex-col items-start gap-1.5">
          <span className="text-12 font-semibold text-muted">Format</span>
          <Segmented
            label="Format"
            value={format}
            onChange={setFormat}
            options={[
              { value: "json", label: "JSON" },
              { value: "ndjson", label: "NDJSON" },
            ]}
          />
          <p className="m-0 text-12 text-muted">
            {format === "json" ? "One JSON array, pretty-printed. Each message has its body and every property." : "One JSON object per line, for streaming tools such as jq."}
          </p>
        </div>
        {held && (
          <div className="flex flex-col gap-1.5">
            <span className="text-12 font-semibold text-muted">File</span>
            <p className="m-0 min-w-0 text-12" data-selectable>
              <span className="break-all font-mono">{held.name}</span>
              <span className="block break-all text-11 text-muted">{held.dir}</span>
            </p>
            <div>
              <Button size="sm" disabled={busy} onClick={() => setPicked(null)}>
                Choose another file
              </Button>
            </div>
          </div>
        )}
        {failure && <Problem role="alert" title="Couldn't start the export" body={advice(failure.kind)} detail={failure.message} />}
      </div>
    </Sheet>
  );
}
