import { useState } from "react";
import { commands, type FilterSpec, type PeekView } from "@/engine";
import { useShell } from "../context";
import { Check, Group } from "./controls";
import { EntityPicker } from "./EntityPicker";
import { OperationSheet } from "./OperationSheet";
import { SelectionSummary, useSelection, verbLabel } from "./selection";
import { useOperation } from "./useOperation";

/** Move messages to another queue or topic: each is sent there, then removed here. */
export function MoveSheet(p: { entity: string; view: PeekView; sequences: number[]; filter: FilterSpec; total?: number | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const sel = useSelection(p);
  const [target, setTarget] = useState<string | null>(null);
  const [drop, setDrop] = useState(false);
  return (
    <OperationSheet
      title="Move messages"
      description="Each message is sent to the target, then removed from here."
      op={op}
      action={verbLabel("Move", sel.count)}
      canSubmit={!sel.limitError && target !== null && !!conn.profileId}
      onSubmit={() => target && void op.start(() => commands.opMove(conn.profileId!, { entity: p.entity, view: p.view, selection: sel.selection }, target, drop))}
      onClose={closeSheet}
    >
      <SelectionSummary entity={p.entity} view={p.view} sequences={p.sequences} filter={p.filter} limit={sel.limit} setLimit={sel.setLimit} limitError={sel.limitError} />
      <Group label="Move to">
        <EntityPicker label="Move to" value={target} onChange={setTarget} exclude={p.entity} />
      </Group>
      <Check
        label="Drop properties that can't be copied"
        hint="Otherwise a message with such a property is not moved and the report says why."
        checked={drop}
        onChange={setDrop}
      />
    </OperationSheet>
  );
}
