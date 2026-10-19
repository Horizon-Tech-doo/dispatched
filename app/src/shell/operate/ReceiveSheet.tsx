import { useState } from "react";
import { commands, type Settlement, type SubQueue } from "@/engine";
import { Field, Input, Segmented, formatNumber } from "@/ui";
import { useShell } from "../context";
import { Group } from "./controls";
import { OperationSheet } from "./OperationSheet";
import { useOperation } from "./useOperation";

type Mode = "lock" | "delete";
type How = Settlement["type"];
const SUB_LABEL: Record<SubQueue, string> = { active: "active messages", "dead-letter": "dead-letter queue", "transfer-dead-letter": "transfer dead-letter queue" };
const AFTER: Record<How, string> = {
  complete: "Each message is removed once received.",
  abandon: "Each message is released at once, and its delivery count goes up.",
  defer: "Each message is set aside; it can be settled later from Deferred.",
  "dead-letter": "Each message moves to the dead-letter queue.",
};

/** Receive messages and settle each one, or receive and delete them. Receive and delete is never the starting choice:
 *  the sheet opens on peek-lock every time. */
export function ReceiveSheet({ entity, subQueue, session }: { entity: string; subQueue: SubQueue; session?: string | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [max, setMax] = useState("10");
  const [wait, setWait] = useState("5");
  const [mode, setMode] = useState<Mode>("lock");
  const [how, setHow] = useState<How>("complete");
  const [reason, setReason] = useState("");
  const [description, setDescription] = useState("");
  const [sessionId, setSessionId] = useState(session ?? "");
  const n = /^\d+$/.test(max.trim()) ? Number(max.trim()) : NaN;
  const maxError = n >= 1 && n <= 5000 ? null : "Receive between 1 and 5,000 messages.";
  const w = /^\d+$/.test(wait.trim()) ? Number(wait.trim()) : NaN;
  const waitError = w >= 0 && w <= 600 ? null : "Wait between 0 and 600 seconds.";
  const shown = maxError ? 1 : n;
  const action = `${mode === "delete" ? "Receive and delete" : "Receive"} up to ${formatNumber(shown)} ${shown === 1 ? "message" : "messages"}`;
  const valid = !maxError && !waitError && !!conn.profileId;
  const submit = () => {
    if (!valid) return;
    const settlement: Settlement =
      how === "dead-letter" ? { type: "dead-letter", ...(reason.trim() ? { reason: reason.trim() } : {}), ...(description.trim() ? { description: description.trim() } : {}) } : { type: how };
    void op.start(() =>
      commands.opReceive(conn.profileId!, {
        entity, sub_queue: subQueue, ...(sessionId.trim() ? { session: sessionId.trim() } : {}), max: n, wait_seconds: w,
        action: mode === "delete" ? { type: "delete" } : { type: "settle", how: settlement },
      }),
    );
  };
  return (
    <OperationSheet
      title="Receive messages"
      description={`From the ${SUB_LABEL[subQueue]} of ${entity}. Received messages are locked or removed, unlike a peek.`}
      op={op}
      action={action}
      canSubmit={valid}
      onSubmit={submit}
      onClose={closeSheet}
    >
      <Group label="How many">
        <Field label="Up to" inline={96} error={maxError ?? undefined}>
          {(ids) => <Input {...ids} type="number" min={1} max={5000} mono className="w-28" value={max} onChange={(e) => setMax(e.target.value)} />}
        </Field>
        <Field label="Wait (s)" inline={96} hint="How long to wait for messages to arrive." error={waitError ?? undefined}>
          {(ids) => <Input {...ids} type="number" min={0} mono className="w-28" value={wait} onChange={(e) => setWait(e.target.value)} />}
        </Field>
        <Field label="Session" inline={96} hint="Empty takes the next available session, on a session entity.">
          {(ids) => <Input {...ids} mono value={sessionId} onChange={(e) => setSessionId(e.target.value)} />}
        </Field>
      </Group>
      <Group label="Mode">
        <Segmented
          label="Receive mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: "lock", label: "Peek-lock" },
            { value: "delete", label: "Receive and delete" },
          ]}
        />
        <p className="m-0 text-12 text-muted">
          {mode === "lock" ? "Each message is locked, then settled as chosen below." : "Each message is removed as it is received. Nothing can bring it back."}
        </p>
      </Group>
      {mode === "lock" && (
        <Group label="Then">
          <Segmented
            label="Settlement"
            value={how}
            onChange={setHow}
            options={[
              { value: "complete", label: "Complete" },
              { value: "abandon", label: "Abandon" },
              { value: "defer", label: "Defer" },
              { value: "dead-letter", label: "Dead-letter" },
            ]}
          />
          <p className="m-0 text-12 text-muted">{AFTER[how]}</p>
          {how === "dead-letter" && (
            <>
              <Field label="Reason" inline={96}>
                {(ids) => <Input {...ids} value={reason} onChange={(e) => setReason(e.target.value)} />}
              </Field>
              <Field label="Description" inline={96}>
                {(ids) => <Input {...ids} value={description} onChange={(e) => setDescription(e.target.value)} />}
              </Field>
            </>
          )}
        </Group>
      )}
    </OperationSheet>
  );
}
