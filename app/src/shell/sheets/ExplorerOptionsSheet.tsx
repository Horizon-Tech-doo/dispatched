import { X } from "lucide-react";
import { useState } from "react";
import { useProfiles } from "@/engine";
import {
  Button, Field, IconButton, Input, LabelSwatch, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, ToggleGroup, labelColors, type LabelColor,
} from "@/ui";
import { useShell } from "../context";
import { COUNT_KINDS, MAX_PATTERN_LENGTH, MAX_RULES, type CountKind, type ExplorerOptions, type HighlightRule } from "../state";
import { ruleProblem } from "../tree/highlight";

type MatchKey = "name" | CountKind;
const MATCH: { value: MatchKey; label: string }[] = [
  { value: "name", label: "Name matches" },
  { value: "active", label: "Active at least" },
  { value: "scheduled", label: "Scheduled at least" },
  { value: "transfer-dlq", label: "Transfer DLQ at least" },
  { value: "dead-letter", label: "Dead-letter at least" },
];
const COUNT_OPTIONS: { value: CountKind; label: string }[] = [
  { value: "active", label: "Active" },
  { value: "scheduled", label: "Scheduled" },
  { value: "transfer-dlq", label: "Transfer DLQ" },
  { value: "dead-letter", label: "Dead-letter" },
];
const COLORS = Object.keys(labelColors) as LabelColor[];
const capital = (c: string) => c[0].toUpperCase() + c.slice(1);

const keyOf = (r: HighlightRule): MatchKey => (r.match.kind === "name" ? "name" : r.match.count);
const newId = () => Math.random().toString(36).slice(2, 10);
/** What a count box holds becomes a number only when it is plain digits: `1.5` and `1e3` are errors, not roundings. */
const parseCount = (text: string) => (/^\s*\d+\s*$/.test(text) ? Number(text) : Number.NaN);

/** Per-connection tree options: which counts rows show, which rows get a colour mark, and what an entity opens on. */
export function ExplorerOptionsSheet() {
  const { shell, dispatch, closeSheet, conn } = useShell();
  const name = useProfiles().data?.find((p) => p.id === conn.profileId)?.name ?? "this connection";
  const [o, setO] = useState<ExplorerOptions>(shell.explorer);
  // What is typed in a count rule's box, kept as text so a half-typed number is not rewritten under the cursor.
  const [typed, setTyped] = useState<Record<string, string>>({});
  const forget = (id: string) => setTyped(({ [id]: _text, ...rest }) => rest);
  const setRule = (i: number, r: HighlightRule) => setO({ ...o, rules: o.rules.map((x, j) => (j === i ? r : x)) });
  const blocked = o.rules.some((r) => ruleProblem(r));
  const save = () => {
    // The tree draws counts in its own order whatever order they were switched on in.
    dispatch({ type: "explorer", options: { ...o, counts: COUNT_KINDS.filter((k) => o.counts.includes(k)) } });
    closeSheet();
  };
  return (
    <Sheet
      open
      onOpenChange={(v) => !v && closeSheet()}
      title={`Explorer options for ${name}`}
      description="Saved for this connection only."
      initialFocus="field"
      footer={
        <>
          <Button onClick={closeSheet}>Cancel</Button>
          <Button variant="primary" disabled={blocked} onClick={save}>
            Save
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-1.5">
          <span className="text-12 font-semibold text-muted">Counts in the tree</span>
          <div>
            <ToggleGroup
              label="Counts in the tree"
              values={o.counts}
              onChange={(counts) => setO({ ...o, counts: counts.length ? counts : ["active"] })}
              options={COUNT_OPTIONS}
            />
          </div>
          <p className="m-0 text-11 text-muted">One count stays on. Scheduled, Transfer DLQ and Dead-letter show only above zero; dead letters come last.</p>
        </div>
        <div className="flex flex-col gap-1.5">
          <span className="text-12 font-semibold text-muted">Open entities on</span>
          <div>
            <Segmented
              label="Open entities on"
              value={o.openOn}
              onChange={(openOn) => setO({ ...o, openOn })}
              options={[
                { value: "overview", label: "Overview" },
                { value: "messages", label: "Messages" },
                { value: "dead-letter", label: "Dead-letter" },
              ]}
            />
          </div>
          <p className="m-0 text-11 text-muted">Topics and the dashboard always open on Overview.</p>
        </div>
        <div className="flex flex-col gap-2">
          <span className="text-12 font-semibold text-muted">Highlight rules</span>
          <p className="m-0 text-11 text-muted">Mark a row with a coloured square. The first rule that matches a row wins. Name patterns ignore case.</p>
          {o.rules.map((r, i) => (
            <div key={r.id} className="flex flex-col gap-1.5 border-t border-line pt-2">
              <div className="flex items-center gap-2">
                <Select value={r.color} onValueChange={(color) => setRule(i, { ...r, color: color as LabelColor })}>
                  <SelectTrigger aria-label={`Rule ${i + 1} colour`} className="w-[60px] shrink-0">
                    <SelectValue>
                      <LabelSwatch color={r.color} />
                      <span className="sr-only">{capital(r.color)}</span>
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {COLORS.map((c) => (
                      <SelectItem key={c} value={c}>
                        <span className="flex items-center gap-2">
                          <LabelSwatch color={c} />
                          {capital(c)}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select
                  value={keyOf(r)}
                  onValueChange={(k) => {
                    // Moving between a name and a count starts the box afresh; between two counts the number stays.
                    if (k === "name" || r.match.kind === "name") forget(r.id);
                    setRule(i, {
                      ...r,
                      match: k === "name" ? { kind: "name", pattern: "" } : { kind: "count", count: k as CountKind, atLeast: r.match.kind === "count" ? r.match.atLeast : 1 },
                    });
                  }}
                >
                  <SelectTrigger aria-label={`Rule ${i + 1} match`} className="min-w-0 flex-1">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {MATCH.map((m) => (
                      <SelectItem key={m.value} value={m.value}>
                        {m.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <IconButton
                  label={`Remove rule ${i + 1}`}
                  onClick={() => {
                    setO({ ...o, rules: o.rules.filter((_, j) => j !== i) });
                    forget(r.id);
                  }}
                >
                  <X />
                </IconButton>
              </div>
              <Field label={`Rule ${i + 1} value`} hideLabel error={ruleProblem(r)}>
                {(ids) => (
                  <Input
                    {...ids}
                    mono={r.match.kind === "name"}
                    inputMode={r.match.kind === "count" ? "numeric" : undefined}
                    maxLength={r.match.kind === "name" ? MAX_PATTERN_LENGTH : 9}
                    placeholder={r.match.kind === "name" ? "^orders- or -dlq$" : "100"}
                    value={r.match.kind === "name" ? r.match.pattern : (typed[r.id] ?? String(r.match.atLeast))}
                    onChange={(e) => {
                      const text = e.target.value;
                      if (r.match.kind === "name") setRule(i, { ...r, match: { kind: "name", pattern: text } });
                      else {
                        setTyped({ ...typed, [r.id]: text });
                        setRule(i, { ...r, match: { ...r.match, atLeast: parseCount(text) } });
                      }
                    }}
                  />
                )}
              </Field>
            </div>
          ))}
          <Button
            className="self-start"
            disabled={o.rules.length >= MAX_RULES}
            onClick={() => setO({ ...o, rules: [...o.rules, { id: newId(), color: COLORS[o.rules.length % COLORS.length], match: { kind: "name", pattern: "" } }] })}
          >
            Add rule
          </Button>
          {o.rules.length >= MAX_RULES && <p className="m-0 text-11 text-muted">{MAX_RULES} rules is the most a connection keeps.</p>}
        </div>
      </div>
    </Sheet>
  );
}
