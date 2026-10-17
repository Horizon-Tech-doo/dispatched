import { useQueryClient } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { checkFilterNow, normalizeFilter, nothingToCheck, useFilterCheck, type FilterField, type FilterSpec } from "@/engine";
import { Button, Field, FieldMessage, Hint, IconButton, Input, Segmented } from "@/ui";
import { describeIssue } from "./filterText";

type BodyMode = "contains" | "regex" | "json";
const modeOf = (f: FilterSpec): BodyMode => (f.body_regex ? "regex" : f.body_json_path ? "json" : "contains");
const bodyOf = (f: FilterSpec) => f.body_contains ?? f.body_regex ?? f.body_json_path ?? "";
const hasBody = (f: FilterSpec) => bodyOf(f).trim() !== "";
/** Blank means no condition once trimmed; what is sent is the text as typed (a regex or expected value keeps its spaces). */
const text = (v: string) => (v.trim() === "" ? null : v);
const sameChecked = (a: FilterSpec, b: FilterSpec) =>
  a.property === b.property && a.body_regex === b.body_regex && a.body_json_path === b.body_json_path && a.body_json_equals === b.body_json_equals;
const sameFilter = (a: FilterSpec, b: FilterSpec) => JSON.stringify(normalizeFilter(a)) === JSON.stringify(normalizeFilter(b));

/** Left column of the bar: the labels line up above each other. */
const LABEL_W = 72;

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

const Code = ({ children }: { children: ReactNode }) => <code className="font-mono">{children}</code>;

export function FilterBar({
  applied,
  onApply,
  onClose,
  autoFocus,
  id,
}: {
  applied: FilterSpec;
  /** Always a fresh read: the caller restarts the peek, also when nothing changed. */
  onApply(f: FilterSpec): void;
  onClose(): void;
  /** Focus the first field on opening (set when the user just opened the bar, so returning to a view doesn't take focus). */
  autoFocus?: boolean;
  /** For the Filter toggle's `aria-controls`. */
  id?: string;
}) {
  const qc = useQueryClient();
  const uid = useId();
  const [property, setProperty] = useState(applied.property ?? "");
  const [mode, setMode] = useState<BodyMode>(modeOf(applied));
  const [body, setBody] = useState(bodyOf(applied));
  const [equals, setEquals] = useState(applied.body_json_equals ?? "");
  const [propertyFocused, setPropertyFocused] = useState(false);
  const [equalsFocused, setEqualsFocused] = useState(false);
  const propertyRef = useRef<HTMLInputElement>(null);

  // The fields follow what is applied from elsewhere, but not the session chip (removing it keeps what has been
  // typed since), and the body segment moves only to the kind of body condition that was applied: applying with
  // an empty JSONPath does not snap the segment back to Contains.
  const appliedMode = hasBody(applied) ? modeOf(applied) : null;
  const appliedFields = JSON.stringify([applied.property ?? "", bodyOf(applied), applied.body_json_equals ?? "", appliedMode]);
  const [syncedFields, setSyncedFields] = useState(appliedFields);
  if (syncedFields !== appliedFields) {
    setSyncedFields(appliedFields);
    setProperty(applied.property ?? "");
    setBody(bodyOf(applied));
    setEquals(applied.body_json_equals ?? "");
    if (appliedMode) setMode(appliedMode);
  }

  // Memoised over its primitive fields: a new object on every render would restart the debounce timer each time
  // and make it set state every 250 ms for as long as the bar is open.
  const session = applied.session_id ?? null;
  const draft = useMemo<FilterSpec>(
    () => ({
      property: text(property),
      body_contains: mode === "contains" ? text(body) : null,
      body_regex: mode === "regex" ? text(body) : null,
      body_json_path: mode === "json" ? text(body) : null,
      body_json_equals: mode === "json" ? text(equals) : null,
      session_id: session,
    }),
    [property, mode, body, equals, session],
  );
  const latest = useRef(draft);
  useEffect(() => {
    latest.current = draft;
  }, [draft]);

  const settled = useDebounced(draft, 250);
  const checked = useFilterCheck(settled);
  // Problems belong to what was checked: while typing has moved on from it, the old problem is not shown against the new text.
  const current = sameChecked(settled, draft);
  const issue = (f: FilterField) => (current ? checked.data?.find((i) => i.field === f) : undefined);
  const blocked = current && (checked.data?.length ?? 0) > 0;
  const dirty = !sameFilter(draft, applied);

  // An Apply that is waiting on a check: another Enter meanwhile is the same request.
  const waiting = useRef(false);
  const apply = async () => {
    if (blocked || waiting.current) return;
    const submitted = draft;
    // Not checked yet (typed a moment ago, or the check is on its way): wait for the answer rather than send a bad filter.
    if ((!current || checked.isFetching) && !nothingToCheck(submitted)) {
      waiting.current = true;
      const found = await checkFilterNow(qc, submitted).finally(() => {
        waiting.current = false;
      });
      // Typed on while waiting: what is in the fields is no longer what was checked.
      if (found.length > 0 || latest.current !== submitted) return;
    }
    onApply(submitted);
  };
  const clear = () => {
    setProperty("");
    setBody("");
    setEquals("");
    setMode("contains");
    onApply({});
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") onClose();
    // Enter applies from a field; on a button it is that button's own press.
    if (e.key === "Enter" && e.target instanceof HTMLInputElement && !e.nativeEvent.isComposing) void apply();
  };

  const propertyIssue = issue("property");
  const propertyProblem = propertyIssue ? describeIssue("property", propertyIssue.message, property) : null;
  const bodyField: FilterField = mode === "regex" ? "body_regex" : "body_json_path";
  const bodyIssue = issue(bodyField);
  const bodyProblem = bodyIssue ? describeIssue(bodyField, bodyIssue.message, body) : null;
  const equalsIssue = mode === "json" ? issue("body_json_equals") : undefined;
  const equalsProblem = equalsIssue ? describeIssue("body_json_equals", equalsIssue.message, equals) : null;
  const bodyLabel = mode === "contains" ? "Body contains" : mode === "regex" ? "Body regex" : "JSONPath";
  const announced = [propertyProblem, bodyProblem, equalsProblem].filter(Boolean).map((p) => p!.sentence).join(" ");
  const equalsId = `${uid}-equals`;
  const syntaxId = `${uid}-syntax`;

  return (
    <div id={id} role="search" aria-label="Message filter" className="relative flex shrink-0 flex-col gap-1.5 border-b border-line bg-side px-3 py-2" onKeyDown={onKeyDown}>
      <Field inline={LABEL_W} label="Properties" error={propertyProblem?.sentence} detail={propertyProblem?.detail}>
        {(ids) => (
          <Input
            {...ids}
            aria-describedby={[ids["aria-describedby"], propertyFocused ? syntaxId : undefined].filter(Boolean).join(" ") || undefined}
            ref={propertyRef}
            mono
            autoFocus={autoFocus}
            placeholder="e.g. sys.DeliveryCount > 3"
            value={property}
            onChange={(e) => setProperty(e.target.value)}
            onFocus={() => setPropertyFocused(true)}
            onBlur={() => setPropertyFocused(false)}
          />
        )}
      </Field>
      <div className="grid items-start gap-x-2 gap-y-1" style={{ gridTemplateColumns: `${LABEL_W}px minmax(0, 1fr)` }}>
        <span className="flex h-6 items-center text-12 font-semibold text-muted">Body</span>
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
            <Segmented
              label="Body condition"
              value={mode}
              onChange={setMode}
              options={[
                { value: "contains", label: "Contains" },
                { value: "regex", label: "Regex" },
                { value: "json", label: "JSONPath" },
              ]}
            />
            <Input
              mono
              aria-label={bodyLabel}
              aria-invalid={bodyProblem ? true : undefined}
              className="min-w-[160px] flex-1"
              placeholder={mode === "json" ? "e.g. $.order.status" : mode === "regex" ? 'e.g. ^\\{"type":"refund"' : "e.g. SHP-1000"}
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
            {mode === "json" && (
              <div className="flex items-center gap-1.5">
                <label htmlFor={equalsId} className="text-12 text-muted">
                  Equals
                </label>
                <Input
                  id={equalsId}
                  mono
                  aria-invalid={equalsProblem ? true : undefined}
                  className="w-[140px]"
                  placeholder="e.g. held"
                  value={equals}
                  onChange={(e) => setEquals(e.target.value)}
                  onFocus={() => setEqualsFocused(true)}
                  onBlur={() => setEqualsFocused(false)}
                />
              </div>
            )}
            {session && (
              <span className="inline-flex h-6 items-center gap-1 rounded-ctl border border-line pl-2 text-12">
                Session <span className="font-mono text-11">{session}</span>
                <IconButton
                  label={`Remove session ${session}`}
                  className="size-5"
                  onClick={() => {
                    onApply({ ...applied, session_id: null });
                    propertyRef.current?.focus();
                  }}
                >
                  <X />
                </IconButton>
              </span>
            )}
            <div className="ml-auto flex items-center gap-2">
              {dirty && <span className="text-11 text-muted">Not applied</span>}
              <Hint label="Clears every condition, including the session">
                <Button onClick={clear}>Clear filter</Button>
              </Hint>
              <Button variant="primary" disabled={blocked || !dirty} onClick={() => void apply()}>
                Apply
              </Button>
            </div>
          </div>
          {bodyProblem && <FieldMessage error={bodyProblem.sentence} detail={bodyProblem.detail} />}
          {equalsProblem && <FieldMessage error={equalsProblem.sentence} detail={equalsProblem.detail} />}
          {mode === "json" && equalsFocused && (
            <p className="m-0 text-11 text-muted">
              Compared as JSON when it reads as JSON (<Code>3</Code>, <Code>true</Code>, <Code>"3"</Code>), otherwise as plain text. Quote a number to compare it as text.
            </p>
          )}
        </div>
      </div>
      {/* The syntax help hangs under the bar and takes no room, and lets clicks through: hiding it when focus moves
          (for instance by pressing Apply) must never move the controls between pointer-down and pointer-up. */}
      {propertyFocused && (
        <p
          id={syntaxId}
          className="pointer-events-none absolute inset-x-3 top-full z-20 m-0 rounded-ctl border border-line bg-side px-2 py-1 text-11 text-muted shadow-float"
        >
          <Code>sys.MessageId</Code> <Code>sys.Subject</Code> <Code>sys.SessionId</Code> <Code>sys.DeliveryCount</Code> <Code>sys.EnqueuedTime</Code>{" "}
          <Code>sys.DeadLetterReason</Code>, or an application property by name. Operators: <Code>= {"<>"} {"<"} {">"} {"<="} {">="} LIKE IN IS NULL AND OR NOT</Code>
        </p>
      )}
      <span role="status" className="sr-only">
        {announced}
      </span>
    </div>
  );
}
