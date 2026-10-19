import { useState } from "react";
import { commands, type EditsForm, type FilterSpec, type PeekView } from "@/engine";
import { Field, Input, Textarea } from "@/ui";
import { useShell } from "../context";
import { Check, Group, sendTarget } from "./controls";
import { EntityPicker } from "./EntityPicker";
import { OperationSheet } from "./OperationSheet";
import { PropertyEditor, propertyErrors, propertyForms, type PropRow } from "./PropertyEditor";
import { SelectionSummary, useSelection, verbLabel } from "./selection";
import { useOperation } from "./useOperation";

type EditKey = "subject" | "content_type" | "correlation_id" | "session_id" | "to" | "reply_to";
const EDITS: { key: EditKey; label: string }[] = [
  { key: "subject", label: "Subject" },
  { key: "content_type", label: "Content type" },
  { key: "correlation_id", label: "Correlation ID" },
  { key: "session_id", label: "Session ID" },
  { key: "to", label: "To" },
  { key: "reply_to", label: "Reply to" },
];

/** Send copies of messages again (by default back where they came from), with edits; optionally remove the originals. */
export function ResubmitSheet(p: { entity: string; view: PeekView; sequences: number[]; filter: FilterSpec; total?: number | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const sel = useSelection(p);
  const back = sendTarget(p.entity);
  const [elsewhere, setElsewhere] = useState(false);
  const [target, setTarget] = useState<string | null>(null);
  const [edits, setEdits] = useState<Partial<Record<EditKey, string>>>({});
  const [replaceBody, setReplaceBody] = useState(false);
  const [body, setBody] = useState("");
  const [set, setSet] = useState<PropRow[]>([]);
  const [remove, setRemove] = useState("");
  const [newId, setNewId] = useState(false);
  const dlq = p.view === "dead-letter" || p.view === "transfer-dead-letter";
  const [removeOriginal, setRemoveOriginal] = useState(dlq);
  const [keepDlq, setKeepDlq] = useState(false);
  const [drop, setDrop] = useState(false);
  const propErrors = propertyErrors(set);
  const valid = !sel.limitError && propErrors.size === 0 && (!elsewhere || target !== null) && !!conn.profileId;

  const submit = () => {
    if (!valid) return;
    const e: EditsForm = {
      set: propertyForms(set),
      remove: remove.split(",").map((s) => s.trim()).filter(Boolean),
      // A field left empty is left alone.
      ...Object.fromEntries(Object.entries(edits).filter(([, v]) => v && v.trim() !== "")),
      ...(replaceBody ? { body } : {}),
    };
    void op.start(() =>
      commands.opResubmit(conn.profileId!, {
        source: { entity: p.entity, view: p.view, selection: sel.selection },
        ...(elsewhere && target ? { target } : {}),
        edits: e, new_message_id: newId, remove_original: removeOriginal, keep_dead_letter_properties: keepDlq, drop_uncopyable_properties: drop,
      }),
    );
  };

  return (
    <OperationSheet
      title="Resubmit messages"
      description="Copies are sent as new messages; the originals stay unless you remove them."
      op={op}
      action={verbLabel("Resubmit", sel.count)}
      canSubmit={valid}
      onSubmit={submit}
      onClose={closeSheet}
    >
      <SelectionSummary entity={p.entity} view={p.view} sequences={p.sequences} filter={p.filter} limit={sel.limit} setLimit={sel.setLimit} limitError={sel.limitError} />
      <Group label="Send to">
        <Check label={`Back to ${back}`} checked={!elsewhere} onChange={(v) => setElsewhere(!v)} />
        {elsewhere && <EntityPicker label="Resubmit to" value={target} onChange={setTarget} />}
      </Group>
      <Group label="Edits">
        <p className="m-0 text-11 text-muted">Empty fields are left as they are.</p>
        {EDITS.map(({ key, label }) => (
          <Field key={key} label={label} inline={112}>
            {(ids) => <Input {...ids} mono placeholder="unchanged" value={edits[key] ?? ""} onChange={(ev) => setEdits({ ...edits, [key]: ev.target.value })} />}
          </Field>
        ))}
        <Check label="Replace the body" checked={replaceBody} onChange={setReplaceBody} />
        {replaceBody && <Textarea mono aria-label="New body" rows={5} value={body} onChange={(e) => setBody(e.target.value)} />}
        <span className="text-12 text-muted">Set properties</span>
        <PropertyEditor rows={set} onChange={setSet} />
        <Field label="Remove properties" hint="Names, separated by commas.">
          {(ids) => <Input {...ids} mono value={remove} onChange={(e) => setRemove(e.target.value)} />}
        </Field>
      </Group>
      <Group label="Options">
        <Check label="New message IDs" hint="Otherwise each copy keeps its message ID; duplicate detection may drop it." checked={newId} onChange={setNewId} />
        <Check label="Remove the originals" hint="Each original is removed once its copy is sent." checked={removeOriginal} onChange={setRemoveOriginal} />
        {dlq && <Check label="Keep dead-letter properties" hint="The reason and description the broker added." checked={keepDlq} onChange={setKeepDlq} />}
        <Check label="Drop properties that can't be copied" checked={drop} onChange={setDrop} />
      </Group>
    </OperationSheet>
  );
}
