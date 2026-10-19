import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { commands } from "@/engine";
import { Button, CodeView, Field, FieldMessage, HexView, Input, Problem, Segmented, SkeletonRows, Textarea, formatBytes } from "@/ui";
import { useShell } from "../context";
import { Check, Group } from "../operate/controls";
import { OperationSheet } from "../operate/OperationSheet";
import { useOperation } from "../operate/useOperation";

/** State above this needs "Allow large state" (Premium namespaces allow more). */
export const STATE_LIMIT = 256 * 1024;

const toBase64 = (bytes: Uint8Array) => {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};
const fromBase64 = (b64: string): Uint8Array | null => {
  try {
    return Uint8Array.from(atob(b64.trim()), (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
};

/** A session's state: view it as text or hex, copy it, replace it (text or base64) or clear it. Before a change the
 *  current state can be copied, since a replaced or cleared state can't be brought back. */
export function SessionStateSheet({ entity, session: first }: { entity: string; session: string }) {
  const { conn, closeSheet, copyText } = useShell();
  const op = useOperation();
  const qc = useQueryClient();
  const [session, setSession] = useState(first);
  const [asked, setAsked] = useState(first);
  const key = ["session-state", conn.profileId, entity, asked] as const;
  const current = useQuery({ queryKey: key, queryFn: () => commands.sessionStateGet(conn.profileId!, entity, asked), enabled: !!conn.profileId && asked !== "" });
  const [view, setView] = useState<"text" | "hex">("text");
  const [change, setChange] = useState<"replace" | "clear">("replace");
  const [format, setFormat] = useState<"text" | "base64">("text");
  const [value, setValue] = useState("");
  const [large, setLarge] = useState(false);
  const [copied, setCopied] = useState<boolean | null>(null);
  const [saved, setSaved] = useState<{ name?: string; error?: string } | null>(null);
  // Read again once a change has run.
  useEffect(() => {
    if (op.stage === "done") void qc.invalidateQueries({ queryKey: ["session-state", conn.profileId, entity] });
  }, [op.stage, qc, conn.profileId, entity]);

  const bytes = useMemo(() => (format === "text" ? new TextEncoder().encode(value) : fromBase64(value)), [format, value]);
  const valueError = change === "replace" && format === "base64" && value.trim() && !bytes ? "This is not valid base64." : null;
  const tooLarge = change === "replace" && bytes !== null && bytes.length > STATE_LIMIT && !large;
  const s = current.data;
  const shownBytes = useMemo(() => (s ? fromBase64(s.base64) : null), [s]);
  const valid = !!conn.profileId && asked !== "" && !valueError && !tooLarge && (change === "clear" || bytes !== null);

  const copyCurrent = async () => {
    if (!s) return;
    setCopied(await copyText(s.text ?? s.base64, s.text !== null ? "Copied the session state" : "Copied the session state as base64", { inline: true }));
  };
  const saveCurrent = async () => {
    try {
      const name = await commands.saveSessionState(conn.profileId!, entity, asked);
      setSaved(name ? { name } : null);
    } catch (e) {
      setSaved({ error: e instanceof Error ? e.message : String(e) });
    }
  };
  const submit = () => {
    if (!valid) return;
    const v = change === "clear" ? null : toBase64(bytes!);
    void op.start(() => commands.opSessionStateSet(conn.profileId!, entity, asked, v, large));
  };

  return (
    <OperationSheet
      title="Session state"
      description={`Session ${asked || "…"} of ${entity}.`}
      op={op}
      action={change === "clear" ? "Clear state" : "Replace state"}
      canSubmit={valid}
      onSubmit={submit}
      onClose={closeSheet}
    >
      {first === "" && (
        <Field label="Session" inline={72}>
          {(ids) => (
            <div className="flex gap-1.5">
              <Input {...ids} mono value={session} onChange={(e) => setSession(e.target.value)} onKeyDown={(e) => e.key === "Enter" && setAsked(session.trim())} />
              <Button onClick={() => setAsked(session.trim())} disabled={!session.trim()}>
                Read
              </Button>
            </div>
          )}
        </Field>
      )}
      {asked !== "" && (
        <Group label="Current state">
          {current.isPending ? (
            <SkeletonRows rows={3} />
          ) : current.isError ? (
            <Problem title="Couldn't read the state" body="Try again." detail={current.error.message} action={<Button onClick={() => void current.refetch()}>Try again</Button>} />
          ) : !s ? (
            <p className="m-0 text-12 text-muted">This session has no state.</p>
          ) : (
            <>
              <div className="flex items-center gap-2">
                {s.text !== null && (
                  <Segmented
                    label="Show state as"
                    value={view}
                    onChange={setView}
                    options={[
                      { value: "text", label: "Text" },
                      { value: "hex", label: "Hex" },
                    ]}
                  />
                )}
                <span className="text-11 tabular-nums text-muted">{formatBytes(s.len)}</span>
                <Button size="sm" className="ml-auto" onClick={() => void copyCurrent()}>
                  Copy current state
                </Button>
                <Button size="sm" onClick={() => void saveCurrent()}>
                  Save current state…
                </Button>
              </div>
              <div className="h-40 overflow-hidden rounded-ctl border border-line">
                {s.text !== null && view === "text" ? <CodeView text={s.text} label="Session state" /> : shownBytes && <HexView bytes={shownBytes} label="Session state" />}
              </div>
              {copied === false && <FieldMessage error="Couldn't copy to the clipboard." />}
              {saved?.name && <p className="m-0 text-11 text-muted">Saved to {saved.name}</p>}
              {saved?.error && <FieldMessage error={`Couldn't save the state: ${saved.error}`} />}
            </>
          )}
        </Group>
      )}
      <Group label="Change">
        <Segmented
          label="Change"
          value={change}
          onChange={setChange}
          options={[
            { value: "replace", label: "Replace" },
            { value: "clear", label: "Clear" },
          ]}
        />
        {change === "clear" ? (
          <p className="m-0 text-12 text-muted">
            {s ? "Copy or save the current state first: a cleared state can't be brought back." : "The session's state is removed."}
          </p>
        ) : (
          <>
            <Segmented
              label="New state as"
              value={format}
              onChange={setFormat}
              options={[
                { value: "text", label: "Text" },
                { value: "base64", label: "Base64" },
              ]}
            />
            <Textarea mono aria-label="New state" aria-invalid={valueError ? true : undefined} rows={5} value={value} onChange={(e) => setValue(e.target.value)} />
            {valueError && <FieldMessage error={valueError} />}
            {bytes && <p className="m-0 text-11 tabular-nums text-muted">{formatBytes(bytes.length)}{format === "text" ? " as UTF-8" : ""}</p>}
            {s && <p className="m-0 text-11 text-muted">The current state is replaced; copy or save it first to keep it.</p>}
            {bytes !== null && bytes.length > STATE_LIMIT && (
              <Check
                label="Allow large state"
                hint="Over 256 KiB, which only Premium namespaces accept."
                checked={large}
                onChange={setLarge}
              />
            )}
          </>
        )}
      </Group>
    </OperationSheet>
  );
}
