import { useState } from "react";
import { commands, type Settlement, type SubQueue } from "@/engine";
import { FieldMessage, Field, Input, Segmented, formatNumber } from "@/ui";
import { useShell } from "../context";
import { Group } from "./controls";
import { OperationSheet } from "./OperationSheet";
import { useOperation } from "./useOperation";

const plural = (n: number, one: string) => `${formatNumber(n)} ${n === 1 ? one : `${one}s`}`;

/** Cancel the selected scheduled messages: they are never enqueued. */
export function CancelScheduledSheet({ entity, sequences }: { entity: string; sequences: number[] }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  return (
    <OperationSheet
      title="Cancel scheduled messages"
      description="Cancelled messages are never enqueued."
      op={op}
      action={sequences.length ? `Cancel ${plural(sequences.length, "scheduled message")}` : "Cancel scheduled messages"}
      canSubmit={sequences.length > 0 && !!conn.profileId}
      onSubmit={() => void op.start(() => commands.opCancelScheduled(conn.profileId!, entity, sequences.map(String)))}
      onClose={closeSheet}
    >
      {sequences.length ? (
        <p className="m-0 text-12 text-muted">
          {plural(sequences.length, "selected message")} scheduled in <span className="text-fg">{entity}</span>.
        </p>
      ) : (
        <FieldMessage error="Select the scheduled messages to cancel first." />
      )}
    </OperationSheet>
  );
}

type How = Settlement["type"];
const HOW_LABEL: Record<How, string> = { complete: "Complete", abandon: "Abandon", defer: "Defer", "dead-letter": "Dead-letter" };

/** Settle the selected deferred messages: complete, abandon (back to active) or dead-letter them. */
export function SettleDeferredSheet({ entity, subQueue, sequences, session, how: first }: { entity: string; subQueue: SubQueue; sequences: number[]; session: string | null; how?: How }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [how, setHow] = useState<How>(first ?? "complete");
  const [reason, setReason] = useState("");
  return (
    <OperationSheet
      title="Settle deferred messages"
      op={op}
      action={sequences.length ? `${HOW_LABEL[how]} ${plural(sequences.length, "message")}` : "Settle messages"}
      canSubmit={sequences.length > 0 && !!conn.profileId}
      onSubmit={() =>
        void op.start(() =>
          commands.opSettleDeferred(conn.profileId!, entity, subQueue, sequences.map(String), session, how === "dead-letter" ? { type: "dead-letter", ...(reason.trim() ? { reason: reason.trim() } : {}) } : { type: how }),
        )
      }
      onClose={closeSheet}
    >
      {sequences.length ? (
        <p className="m-0 text-12 text-muted">
          {plural(sequences.length, "selected deferred message")} in <span className="text-fg">{entity}</span>
          {session ? ` (session ${session})` : ""}.
        </p>
      ) : (
        <FieldMessage error="Select the deferred messages to settle first." />
      )}
      <Group label="Settle as">
        <Segmented
          label="Settlement"
          value={how}
          onChange={setHow}
          options={(["complete", "abandon", "dead-letter"] as How[]).map((h) => ({ value: h, label: HOW_LABEL[h] }))}
        />
        <p className="m-0 text-12 text-muted">
          {how === "complete" ? "Each message is removed." : how === "abandon" ? "Each message stays deferred, and its delivery count goes up." : "Each message moves to the dead-letter queue."}
        </p>
        {how === "dead-letter" && (
          <Field label="Reason" inline={96}>
            {(ids) => <Input {...ids} value={reason} onChange={(e) => setReason(e.target.value)} />}
          </Field>
        )}
      </Group>
    </OperationSheet>
  );
}
