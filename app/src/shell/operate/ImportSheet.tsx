import { useState } from "react";
import { commands, type OpenedFiles } from "@/engine";
import { Button, Field, FieldMessage, Input } from "@/ui";
import { useShell } from "../context";
import { Check, Group, sendTarget } from "./controls";
import { OperationSheet } from "./OperationSheet";
import { useOperation } from "./useOperation";

/** Send the messages of an export file (JSON or NDJSON) to an entity, after the core's confirmation. */
export function ImportSheet({ entity }: { entity: string | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [target, setTarget] = useState(entity ? sendTarget(entity) : "");
  const [file, setFile] = useState<OpenedFiles | null>(null);
  const [pickFailed, setPickFailed] = useState<string | null>(null);
  const [newIds, setNewIds] = useState(false);
  const [keepSessions, setKeepSessions] = useState(true);
  const [keepDlq, setKeepDlq] = useState(false);
  const choose = async () => {
    setPickFailed(null);
    try {
      const r = await commands.pickOpenPaths("import");
      if (r) setFile(r);
    } catch (e) {
      setPickFailed(e instanceof Error ? e.message : String(e));
    }
  };
  const valid = target.trim() !== "" && file !== null && !!conn.profileId;
  return (
    <OperationSheet
      title="Import messages"
      description="Messages from an export file are sent as new messages."
      op={op}
      action="Import messages"
      canSubmit={valid}
      onSubmit={() =>
        valid &&
        void op.start(() =>
          commands.opImport(conn.profileId!, { target: target.trim(), pick: file!.pick, new_message_id: newIds, keep_sessions: keepSessions, keep_dead_letter_properties: keepDlq }),
        )
      }
      onClose={closeSheet}
    >
      <Field label="Entity" hint="A queue or a topic." error={target.trim() ? undefined : "Choose where to send the messages."}>
        {(ids) => <Input {...ids} mono value={target} onChange={(e) => setTarget(e.target.value)} />}
      </Field>
      <Group label="File">
        {file ? (
          file.names.map((n) => (
            <span key={n} className="break-all font-mono text-11 text-fg">
              {n}
            </span>
          ))
        ) : (
          <p className="m-0 text-12 text-muted">A JSON or NDJSON file written by Export.</p>
        )}
        <Button size="sm" className="self-start" onClick={() => void choose()}>
          {file ? "Choose another file…" : "Choose file…"}
        </Button>
        {pickFailed && <FieldMessage error="The file dialog failed." detail={pickFailed} />}
      </Group>
      <Group label="Options">
        <Check label="New message IDs" hint="Each message gets a new GUID; otherwise its ID is kept." checked={newIds} onChange={setNewIds} />
        <Check label="Keep session IDs" hint="Turn off to send to an entity without sessions." checked={keepSessions} onChange={setKeepSessions} />
        <Check label="Keep dead-letter properties" hint="The reason and description the broker added." checked={keepDlq} onChange={setKeepDlq} />
      </Group>
    </OperationSheet>
  );
}
