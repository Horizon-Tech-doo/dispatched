import { commands, type FilterSpec, type PeekView } from "@/engine";
import { useShell } from "../context";
import { OperationSheet } from "./OperationSheet";
import { SelectionSummary, useSelection, verbLabel } from "./selection";
import { useOperation } from "./useOperation";

/** Delete the selected messages of a view, or every message matching its filter (up to a limit). */
export function DeleteSheet(p: { entity: string; view: PeekView; sequences: number[]; filter: FilterSpec; total?: number | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const sel = useSelection(p);
  return (
    <OperationSheet
      title="Delete messages"
      description="Each message is received and removed. Nothing can bring it back."
      op={op}
      action={verbLabel("Delete", sel.count)}
      canSubmit={!sel.limitError && !!conn.profileId}
      onSubmit={() => void op.start(() => commands.opDelete(conn.profileId!, { entity: p.entity, view: p.view, selection: sel.selection }))}
      onClose={closeSheet}
    >
      <SelectionSummary entity={p.entity} view={p.view} sequences={p.sequences} filter={p.filter} limit={sel.limit} setLimit={sel.setLimit} limitError={sel.limitError} />
    </OperationSheet>
  );
}
