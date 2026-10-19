import { useState } from "react";
import { commands, type IdsForm, type MessageDetail, type OpenedFiles, type OutgoingForm } from "@/engine";
import { Button, Field, FieldMessage, Input, Segmented, Textarea, formatNumber } from "@/ui";
import { useShell } from "../context";
import { decodeBody } from "../messages/exportJson";
import { Check, Group, sendTarget } from "./controls";
import { OperationSheet } from "./OperationSheet";
import { PropertyEditor, newRowKey, propertyErrors, propertyForms, type PropRow } from "./PropertyEditor";
import { checkTtl, copyOf, incrementId } from "./props";
import { useOperation } from "./useOperation";

type BodyKind = "text" | "files" | "template";
type IdMode = "fixed" | "increment" | "random" | "keep";
type SystemKey = Exclude<keyof OutgoingForm, "properties" | "message_id" | "time_to_live">;

const CONTENT_TYPES = ["application/json", "application/xml", "text/plain", "application/octet-stream"];
/** The system properties a message can be sent with (§4.7), in form order. */
const SYSTEM: { key: SystemKey; label: string }[] = [
  { key: "session_id", label: "Session ID" },
  { key: "correlation_id", label: "Correlation ID" },
  { key: "subject", label: "Subject" },
  { key: "to", label: "To" },
  { key: "reply_to", label: "Reply to" },
  { key: "reply_to_session_id", label: "Reply to session ID" },
  { key: "partition_key", label: "Partition key" },
];
/** Properties the broker sets when it dead-letters a message: not part of a resend. */
const BROKER_SET = new Set(["DeadLetterReason", "DeadLetterErrorDescription"]);

/** Seconds as the shortest duration text the engine reads back exactly. */
const ttlText = (ms: number) => `${ms / 1000}s`;

/** A `datetime-local` value as RFC 3339 in UTC, whole seconds. */
export const localToUtc = (local: string): string | null => {
  const t = new Date(local);
  return Number.isNaN(t.getTime()) ? null : t.toISOString().replace(/\.\d{3}Z$/, "Z");
};

interface Draft {
  target: string;
  bodyKind: BodyKind;
  text: string;
  files: OpenedFiles | null;
  template: OpenedFiles | null;
  idMode: IdMode;
  fixedId: string;
  base: string;
  system: Partial<Record<SystemKey, string>>;
  contentType: string;
  ttl: string;
  props: PropRow[];
  count: string;
  schedule: string;
  batch: boolean;
  /** Why the cloned body could not be copied, if it could not. */
  bodyNote: string | null;
}

/** The form, empty or seeded from a message (Clone & resend: every property a sender can set, typed properties kept,
 *  the ones a form cannot send shown as not copyable). */
function draftOf(entity: string | null, d: MessageDetail | null | undefined): Draft {
  const empty: Draft = {
    target: entity ? sendTarget(entity) : "", bodyKind: "text", text: "", files: null, template: null, idMode: "random", fixedId: "", base: "",
    system: {}, contentType: "", ttl: "", props: [], count: "1", schedule: "", batch: false, bodyNote: null,
  };
  if (!d) return empty;
  const { text } = decodeBody(d.body, d.body_truncated);
  const bodyNote = d.body_truncated
    ? "The body is larger than 4 MiB, so it was not copied. Export the message and send the file instead."
    : text === null
      ? "The body is not text, so it was not copied. Export the message and send the file instead."
      : null;
  const system: Draft["system"] = {};
  for (const { key } of SYSTEM) {
    const v = d[key as keyof MessageDetail];
    if (typeof v === "string" && v) system[key] = v;
  }
  const props: PropRow[] = Object.entries(d.application_properties)
    .filter(([name]) => !BROKER_SET.has(name))
    .map(([name, v]) => {
      const c = copyOf(v);
      return "locked" in c ? { key: newRowKey(), name, type: "string", value: "", locked: c.locked } : { key: newRowKey(), name, type: c.type, value: c.value, was: c.was };
    });
  return {
    ...empty,
    text: bodyNote ? "" : (text ?? ""),
    bodyNote,
    idMode: d.message_id ? "fixed" : "random",
    fixedId: d.message_id ?? "",
    system,
    contentType: d.content_type ?? "",
    ttl: d.time_to_live_ms != null ? ttlText(d.time_to_live_ms) : "",
    props,
  };
}

/** Compose messages and send them, or schedule them, after the core's confirmation. */
export function ComposerSheet({ entity, clone }: { entity: string | null; clone?: MessageDetail | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [f, setF] = useState<Draft>(() => draftOf(entity, clone));
  const [pickFailed, setPickFailed] = useState<string | null>(null);
  const set = (patch: Partial<Draft>) => setF((d) => ({ ...d, ...patch }));

  const count = /^\d+$/.test(f.count.trim()) ? Number(f.count.trim()) : NaN;
  const countError = Number.isSafeInteger(count) && count >= 1 ? null : "Send at least one message.";
  const ttlError = checkTtl(f.ttl);
  const scheduleUtc = f.schedule ? localToUtc(f.schedule) : null;
  const scheduleError = f.schedule && !scheduleUtc ? "Choose a date and time." : null;
  const propErrors = propertyErrors(f.props);
  const template = f.bodyKind === "template";
  const pickMissing = f.bodyKind === "files" && !f.files ? "Choose at least one file." : template && !f.template ? "Choose a template file." : null;
  const idError =
    f.idMode === "fixed" && !f.fixedId.trim() ? "Type the message ID, or choose Random." : f.idMode === "increment" && !f.base.trim() ? "Type the first message ID." : null;
  const valid = f.target.trim() !== "" && !countError && !ttlError && !scheduleError && propErrors.size === 0 && !pickMissing && !idError;
  const n = Number.isSafeInteger(count) && count >= 1 ? count : 1;
  const action = `${f.schedule ? "Schedule" : "Send"} ${formatNumber(n)} ${n === 1 ? "message" : "messages"}`;

  const pick = async (purpose: "body" | "template") => {
    setPickFailed(null);
    try {
      const r = await commands.pickOpenPaths(purpose);
      if (r) set(purpose === "body" ? { files: r } : { template: r });
    } catch (e) {
      setPickFailed(e instanceof Error ? e.message : String(e));
    }
  };

  const submit = () => {
    if (!conn.profileId || !valid) return;
    const ids: IdsForm =
      f.idMode === "fixed" ? { type: "fixed", id: f.fixedId } : f.idMode === "increment" ? { type: "increment", base: f.base } : f.idMode === "keep" ? { type: "keep" } : { type: "random" };
    const templateForm: OutgoingForm = template
      ? { properties: [] }
      : {
          ...Object.fromEntries(Object.entries(f.system).filter(([, v]) => v && v.trim() !== "")),
          ...(f.contentType.trim() ? { content_type: f.contentType.trim() } : {}),
          ...(f.ttl.trim() ? { time_to_live: f.ttl.trim() } : {}),
          properties: propertyForms(f.props),
        };
    const body = f.bodyKind === "text" ? { type: "text" as const, text: f.text } : f.bodyKind === "files" ? { type: "files" as const, pick: f.files!.pick } : { type: "template" as const, pick: f.template!.pick };
    const profileId = conn.profileId;
    void op.start(() =>
      commands.opSend(profileId, { target: f.target.trim(), template: templateForm, body, ids, count: n, ...(scheduleUtc ? { schedule: scheduleUtc } : {}), batch: f.batch }),
    );
  };

  const preview = f.idMode === "increment" && f.base.trim() ? `${incrementId(f.base, 0)} → ${incrementId(f.base, 1)}${n > 2 ? ` … ${incrementId(f.base, n - 1)}` : ""}` : null;

  return (
    <OperationSheet
      title="Send messages"
      description={clone ? `A copy of message ${clone.sequence_number}, sent as a new message.` : undefined}
      op={op}
      action={action}
      canSubmit={valid && !!conn.profileId}
      onSubmit={submit}
      onClose={closeSheet}
    >
      <Field label="Entity" hint="A queue or a topic." error={f.target.trim() ? undefined : "Choose where to send the messages."}>
        {(ids) => <Input {...ids} mono value={f.target} onChange={(e) => set({ target: e.target.value })} />}
      </Field>

      <Group label="Body">
        <Segmented
          label="Body source"
          value={f.bodyKind}
          onChange={(v) => set({ bodyKind: v, idMode: v === "template" ? "keep" : f.idMode === "keep" ? "random" : f.idMode })}
          options={[
            { value: "text", label: "Text" },
            { value: "files", label: "Files" },
            { value: "template", label: "Template" },
          ]}
        />
        {f.bodyKind === "text" && (
          <>
            <Textarea mono aria-label="Body" rows={6} value={f.text} onChange={(e) => set({ text: e.target.value })} />
            {f.bodyNote && <p className="m-0 text-11 text-muted">{f.bodyNote}</p>}
          </>
        )}
        {f.bodyKind !== "text" && (
          <div className="flex flex-col items-start gap-1.5">
            {(f.bodyKind === "files" ? f.files : f.template)?.names.map((name) => (
              <span key={name} className="break-all font-mono text-11 text-fg">
                {name}
              </span>
            ))}
            <p className="m-0 text-11 text-muted">
              {f.bodyKind === "files"
                ? "Each file is one message's body, used in turn when sending more messages than files."
                : "A message template (JSON or XML) sets the body and every property."}
            </p>
            <Button size="sm" onClick={() => void pick(f.bodyKind === "files" ? "body" : "template")}>
              {f.bodyKind === "files" ? "Choose files…" : "Choose template…"}
            </Button>
            {pickMissing && <FieldMessage error={pickMissing} />}
            {pickFailed && <FieldMessage error="The file dialog failed." detail={pickFailed} />}
          </div>
        )}
      </Group>

      <Group label="Message ID">
        <Segmented
          label="Message ID source"
          value={f.idMode}
          onChange={(v) => set({ idMode: v })}
          options={[
            ...(template ? [{ value: "keep" as const, label: "From template" }] : []),
            { value: "random", label: "Random" },
            { value: "fixed", label: "Fixed" },
            { value: "increment", label: "Increment" },
          ]}
        />
        {f.idMode === "random" && <p className="m-0 text-11 text-muted">A new GUID for every message.</p>}
        {f.idMode === "fixed" && (
          <Field label="Message ID" hideLabel hint={n > 1 ? "Every message gets this ID; duplicate detection may drop all but one." : undefined} error={idError ?? undefined}>
            {(ids) => <Input {...ids} mono value={f.fixedId} onChange={(e) => set({ fixedId: e.target.value })} />}
          </Field>
        )}
        {f.idMode === "increment" && (
          <Field label="First message ID" hideLabel hint={preview ?? "Trailing digits count up and keep their zero padding."} error={idError ?? undefined}>
            {(ids) => <Input {...ids} mono placeholder="order-0001" value={f.base} onChange={(e) => set({ base: e.target.value })} />}
          </Field>
        )}
      </Group>

      {!template && (
        <Group label="Properties">
          <Field label="Content type" inline={128}>
            {(ids) => (
              <>
                <Input {...ids} mono list="composer-content-types" value={f.contentType} onChange={(e) => set({ contentType: e.target.value })} />
                <datalist id="composer-content-types">
                  {CONTENT_TYPES.map((t) => (
                    <option key={t} value={t} />
                  ))}
                </datalist>
              </>
            )}
          </Field>
          {SYSTEM.map(({ key, label }) => (
            <Field key={key} label={label} inline={128}>
              {(ids) => <Input {...ids} mono value={f.system[key] ?? ""} onChange={(e) => set({ system: { ...f.system, [key]: e.target.value } })} />}
            </Field>
          ))}
          <Field label="Time to live" inline={128} error={ttlError ?? undefined}>
            {(ids) => <Input {...ids} mono placeholder="5m, 2h, 7d" value={f.ttl} onChange={(e) => set({ ttl: e.target.value })} />}
          </Field>
        </Group>
      )}

      {!template && (
        <Group label="Custom properties">
          <PropertyEditor rows={f.props} onChange={(props) => set({ props })} />
        </Group>
      )}

      <Group label="Sending">
        <Field label="Count" inline={128} error={countError ?? undefined}>
          {(ids) => <Input {...ids} type="number" min={1} mono className="w-24" value={f.count} onChange={(e) => set({ count: e.target.value })} />}
        </Field>
        <Field label="Schedule for" inline={128} hint="Empty sends now. Your local time; sent in UTC." error={scheduleError ?? undefined}>
          {(ids) => <Input {...ids} type="datetime-local" mono value={f.schedule} onChange={(e) => set({ schedule: e.target.value })} />}
        </Field>
        <Check label="Send in batches" hint="Fewer round trips. A batch is not atomic: some messages may be sent when others fail." checked={f.batch} onChange={(batch) => set({ batch })} />
      </Group>
    </OperationSheet>
  );
}
