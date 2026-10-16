import { ChevronDown, ChevronUp, X } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { useEntities, useProfiles, useSubscriptionsFor, type EntityCounts } from "@/engine";
import {
  Button, Field, IconButton, Input, LabelSwatch, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, ToggleGroup, formatNumber, labelColors, type LabelColor,
} from "@/ui";
import { useShell } from "../context";
import { COUNT_KINDS, MAX_PATTERN_LENGTH, MAX_RULES, type CountKind, type ExplorerOptions, type HighlightRule } from "../state";
import { compileRules, ruleProblem, type Compiled } from "../tree/highlight";

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
  // An empty pattern is a problem only once the person has left its box or tried to save; a bad one at once.
  const [touched, setTouched] = useState<ReadonlySet<string>>(new Set());
  const touch = (id: string) => setTouched((t) => (t.has(id) ? t : new Set(t).add(id)));
  const problemOf = (r: HighlightRule) => (r.match.kind === "name" && !r.match.pattern.trim() && !touched.has(r.id) ? undefined : ruleProblem(r));
  const blocked = o.rules.some((r) => problemOf(r));
  const move = (i: number, by: -1 | 1) => {
    const rules = [...o.rules];
    [rules[i], rules[i + by]] = [rules[i + by], rules[i]];
    setO({ ...o, rules });
  };

  // What each rule would mark among the entities already loaded (the tree's own queries, so nothing more is fetched).
  const profileId = conn.profileId;
  const listing = useEntities(profileId).data;
  const topics = useMemo(
    () => (listing ? shell.expanded.filter((e) => e.startsWith("t:") && listing.topics.some((t) => t.name === e.slice(2))).map((e) => e.slice(2)) : []),
    [listing, shell.expanded],
  );
  const subQueries = useSubscriptionsFor(profileId, topics);
  const loaded = useMemo(() => {
    const all: { name: string; counts: EntityCounts | null }[] = [];
    if (listing) {
      for (const q of listing.queues) all.push({ name: q.name, counts: q.counts });
      for (const t of listing.topics) all.push({ name: t.name, counts: null });
      for (const k of listing.known) all.push({ name: k.subscription ?? k.entity, counts: null });
    }
    for (const q of subQueries) for (const sub of q.data ?? []) all.push({ name: sub.name, counts: sub.counts });
    return all;
  }, [listing, subQueries]);
  // A rule is compiled once per pattern: typing in one rule does not recompile the others (or its own earlier forms).
  const compiled = useRef(new Map<string, Compiled | null>());
  const matchesOf = (r: HighlightRule) => {
    const key = `${r.id}\0${JSON.stringify(r.match)}`;
    let rule = compiled.current.get(key);
    if (rule === undefined) {
      rule = compileRules([r])[0] ?? null;
      compiled.current.set(key, rule);
    }
    return rule ? loaded.filter((e) => rule.test(e.name, e.counts)).length : 0;
  };
  // Forms that are no longer on screen are forgotten.
  const live = new Set(o.rules.map((r) => `${r.id}\0${JSON.stringify(r.match)}`));
  for (const k of compiled.current.keys()) if (!live.has(k)) compiled.current.delete(k);
  const save = () => {
    if (o.rules.some((r) => ruleProblem(r))) return setTouched(new Set(o.rules.map((r) => r.id))); // show what is wrong
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
            <div key={r.id} role="group" aria-label={`Rule ${i + 1}`} className="flex flex-col gap-1.5 border-t border-line pt-2">
              <div className="flex items-center gap-2">
                <span aria-hidden="true" className="w-3 shrink-0 text-11 tabular-nums text-muted">{i + 1}</span>
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
                <IconButton label={`Move rule ${i + 1} up`} disabled={i === 0} onClick={() => move(i, -1)}>
                  <ChevronUp />
                </IconButton>
                <IconButton label={`Move rule ${i + 1} down`} disabled={i === o.rules.length - 1} onClick={() => move(i, 1)}>
                  <ChevronDown />
                </IconButton>
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
              <Field label={`Rule ${i + 1} value`} hideLabel error={problemOf(r)}>
                {(ids) => (
                  <Input
                    {...ids}
                    mono={r.match.kind === "name"}
                    inputMode={r.match.kind === "count" ? "numeric" : undefined}
                    maxLength={r.match.kind === "name" ? MAX_PATTERN_LENGTH : 9}
                    placeholder={r.match.kind === "name" ? "^orders- or -dlq$" : "100"}
                    value={r.match.kind === "name" ? r.match.pattern : (typed[r.id] ?? String(r.match.atLeast))}
                    onBlur={() => touch(r.id)}
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
              {!ruleProblem(r) && (() => {
                const n = matchesOf(r);
                return <p className="m-0 text-11 tabular-nums text-muted">{n === 0 ? "Matches no loaded entities" : n === 1 ? "Matches 1 loaded entity" : `Matches ${formatNumber(n)} loaded entities`}</p>;
              })()}
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
