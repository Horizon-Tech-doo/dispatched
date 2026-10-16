import { useQueryClient } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { commands, keys, useFilterCheck, type FilterField, type FilterSpec } from "@/engine";
import { Button, Field, IconButton, Input, Segmented } from "@/ui";

type BodyMode = "contains" | "regex" | "json";
const modeOf = (f: FilterSpec): BodyMode => (f.body_regex ? "regex" : f.body_json_path ? "json" : "contains");
const bodyOf = (f: FilterSpec) => f.body_contains ?? f.body_regex ?? f.body_json_path ?? "";
/** Blank means no condition once trimmed; what is sent is the text as typed (a regex or expected value keeps its spaces). */
const text = (v: string) => (v.trim() === "" ? null : v);
/** The fields the engine can find a problem in. */
const needsCheck = (f: FilterSpec) => [f.property, f.body_regex, f.body_json_path, f.body_json_equals].some((v) => v?.trim());
const sameChecked = (a: FilterSpec, b: FilterSpec) =>
  a.property === b.property && a.body_regex === b.body_regex && a.body_json_path === b.body_json_path && a.body_json_equals === b.body_json_equals;

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

export function FilterBar({
  applied,
  onApply,
  onClose,
  autoFocus,
}: {
  applied: FilterSpec;
  onApply(f: FilterSpec): void;
  onClose(): void;
  /** Focus the first field on opening (set when the user just opened the bar, so returning to a view doesn't take focus). */
  autoFocus?: boolean;
}) {
  const qc = useQueryClient();
  const [property, setProperty] = useState(applied.property ?? "");
  const [mode, setMode] = useState<BodyMode>(modeOf(applied));
  const [body, setBody] = useState(bodyOf(applied));
  const [equals, setEquals] = useState(applied.body_json_equals ?? "");
  // The fields follow what is applied from elsewhere (another way of changing the filter), but not the session
  // chip: removing that must keep what has been typed since.
  const fields = JSON.stringify([applied.property ?? "", modeOf(applied), bodyOf(applied), applied.body_json_equals ?? ""]);
  const synced = useRef(fields);
  useEffect(() => {
    if (synced.current === fields) return;
    synced.current = fields;
    setProperty(applied.property ?? "");
    setMode(modeOf(applied));
    setBody(bodyOf(applied));
    setEquals(applied.body_json_equals ?? "");
    // `applied` is read for the values above: `fields` is what says they changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fields]);
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
  const settled = useDebounced(draft, 250);
  const checked = useFilterCheck(settled);
  const issue = (f: FilterField) => checked.data?.find((i) => i.field === f)?.message;
  // Problems belong to what was checked: while typing has moved on from it, the old problem is not shown against the new text.
  const current = sameChecked(settled, draft);
  const blocked = current && (checked.data?.length ?? 0) > 0;
  const apply = async () => {
    if (blocked) return;
    // Enter a moment after typing: the check has not run on this text yet, so run it now rather than send a bad filter.
    if (!current && needsCheck(draft)) {
      const found = await qc.fetchQuery({ queryKey: keys.filterCheck(draft), queryFn: () => commands.checkFilter(draft), staleTime: Infinity }).catch(() => []);
      if (found.length > 0) return;
    }
    onApply(draft);
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
  const bodyLabel = mode === "contains" ? "Body contains" : mode === "regex" ? "Body regex" : "JSONPath";
  return (
    <div role="search" aria-label="Message filter" className="flex shrink-0 flex-col gap-2 border-b border-line bg-side px-3 py-2.5" onKeyDown={onKeyDown}>
      <Field
        label="Properties"
        hint="sys.MessageId, sys.Subject, sys.SessionId, sys.DeliveryCount, sys.EnqueuedTime, sys.DeadLetterReason, or an application property by name. = <> < > <= >= LIKE IN IS NULL, AND OR NOT."
        error={current ? issue("property") : undefined}
      >
        {(ids) => (
          <Input {...ids} mono autoFocus={autoFocus} placeholder="sys.DeliveryCount > 3 AND region = 'eu'" value={property} onChange={(e) => setProperty(e.target.value)} />
        )}
      </Field>
      <div className="flex flex-wrap items-end gap-x-2 gap-y-2">
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
        <div className="min-w-[200px] flex-1">
          <Field label={bodyLabel} error={current ? issue(mode === "regex" ? "body_regex" : "body_json_path") : undefined}>
            {(ids) => (
              <Input
                {...ids}
                mono
                placeholder={mode === "json" ? "$.order.status" : mode === "regex" ? '^\\{"type":"refund"' : "text in the body"}
                value={body}
                onChange={(e) => setBody(e.target.value)}
              />
            )}
          </Field>
        </div>
        {mode === "json" && (
          <div className="w-[180px]">
            <Field label="Equals" error={current ? issue("body_json_equals") : undefined}>
              {(ids) => <Input {...ids} mono placeholder="optional" value={equals} onChange={(e) => setEquals(e.target.value)} />}
            </Field>
          </div>
        )}
        {session && (
          <span className="inline-flex h-6 items-center gap-1 rounded-ctl border border-line pl-2 text-12">
            Session <span className="font-mono text-11">{session}</span>
            <IconButton label={`Remove session ${session}`} className="size-5" onClick={() => onApply({ ...applied, session_id: null })}>
              <X />
            </IconButton>
          </span>
        )}
        <div className="ml-auto flex gap-2">
          <Button onClick={clear}>Clear</Button>
          <Button variant="primary" disabled={blocked} onClick={() => void apply()}>
            Apply
          </Button>
        </div>
      </div>
    </div>
  );
}
