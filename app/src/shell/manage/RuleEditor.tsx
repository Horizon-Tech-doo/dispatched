import { Plus, X } from "lucide-react";
import { useRef } from "react";
import type { FieldIssue, RuleFilter, RuleFilterForm, RuleForm, RuleInfo } from "@/engine";
import { Button, Field, FieldMessage, IconButton, Input, Segmented, Textarea } from "@/ui";
import { issueOf } from "./useIssues";

/** The correlation filter's system fields, in the order the service lists them. */
export const CORRELATION_FIELDS = [
  ["correlation_id", "Correlation ID"],
  ["message_id", "Message ID"],
  ["to", "To"],
  ["reply_to", "Reply to"],
  ["subject", "Subject"],
  ["session_id", "Session ID"],
  ["reply_to_session_id", "Reply to session ID"],
  ["content_type", "Content type"],
] as const;
type CorrelationKey = (typeof CORRELATION_FIELDS)[number][0];
type Correlation = Extract<RuleFilterForm, { kind: "correlation" }>;
type Kind = RuleFilterForm["kind"];

/** The engine's sentence for a field, without its error-kind prefix. */
export const sentence = (m: string) => {
  const t = m.replace(/^invalid input: /, "");
  return t ? `${t[0].toUpperCase()}${t.slice(1)}` : t;
};

const emptyCorrelation = (): Correlation => ({ kind: "correlation", properties: [] });

/** The form of an existing rule. A filter this reader can't show is edited as an empty SQL filter. */
export function formOf(r: RuleInfo): RuleForm {
  const f: RuleFilter = r.filter;
  const filter: RuleFilterForm =
    f.kind === "unknown"
      ? { kind: "sql", expression: "" }
      : f.kind === "correlation"
        ? { ...f, properties: f.properties.map((p) => ({ name: p.name, value: p.value })) }
        : f;
  return { name: r.name, filter, action: r.action };
}

export const newRule = (): RuleForm => ({ name: "", filter: { kind: "sql", expression: "" }, action: null });

/**
 * A rule's name, filter and optional action. The filter is the default (every message), a SQL expression or a
 * correlation filter; switching kinds keeps what was typed in each, so switching back loses nothing.
 */
export function RuleEditor({
  value,
  onChange,
  issues,
  nameReadOnly,
  showName = true,
}: {
  value: RuleForm;
  onChange(v: RuleForm): void;
  issues: FieldIssue[];
  nameReadOnly?: boolean;
  showName?: boolean;
}) {
  // What each kind held when the user switched away from it.
  const drafts = useRef<Partial<Record<Kind, RuleFilterForm>>>({ [value.filter.kind]: value.filter });
  const setFilter = (filter: RuleFilterForm) => {
    drafts.current[filter.kind] = filter;
    onChange({ ...value, filter });
  };
  const switchTo = (kind: Kind) => {
    drafts.current[value.filter.kind] = value.filter;
    setFilter(drafts.current[kind] ?? (kind === "true" ? { kind } : kind === "sql" ? { kind, expression: "" } : emptyCorrelation()));
  };
  const f = value.filter;
  const filterIssue = issueOf(issues, "filter");
  return (
    <div className="flex flex-col gap-3">
      {showName && (
        <Field label="Name" error={issueOf(issues, "name") && sentence(issueOf(issues, "name")!)} hint={nameReadOnly ? "A rule's name can't change. Add a new rule instead." : undefined}>
          {(ids) => <Input {...ids} mono readOnly={nameReadOnly} value={value.name} onChange={(e) => onChange({ ...value, name: e.target.value })} />}
        </Field>
      )}
      <div className="flex flex-col gap-1.5">
        <span className="text-12 font-semibold text-muted" id="rule-filter-label">
          Filter
        </span>
        <Segmented<Kind>
          label="Filter"
          value={f.kind}
          onChange={switchTo}
          options={[
            { value: "true", label: "Default" },
            { value: "sql", label: "SQL" },
            { value: "correlation", label: "Correlation" },
          ]}
        />
        {f.kind === "true" && <p className="m-0 text-12 text-muted">Every message reaches the subscription, as with the default rule.</p>}
        {f.kind === "sql" && (
          <Field label="SQL filter" hideLabel hint="For example: region = 'eu' AND sys.Label LIKE 'order%'">
            {(ids) => <Textarea {...ids} mono rows={3} value={f.expression} onChange={(e) => setFilter({ kind: "sql", expression: e.target.value })} />}
          </Field>
        )}
        {f.kind === "correlation" && <CorrelationFields f={f} onChange={setFilter} />}
        {filterIssue && <FieldMessage error={sentence(filterIssue)} />}
      </div>
      <Field label="SQL action" hint="Optional. Runs on each message the filter lets through, for example: SET priority = 'high'">
        {(ids) => <Textarea {...ids} mono rows={2} value={value.action ?? ""} onChange={(e) => onChange({ ...value, action: e.target.value || null })} />}
      </Field>
    </div>
  );
}

function CorrelationFields({ f, onChange }: { f: Correlation; onChange(f: Correlation): void }) {
  const set = (k: CorrelationKey, v: string) => onChange({ ...f, [k]: v || null });
  const props = f.properties;
  const setProp = (i: number, patch: Partial<{ name: string; value: string }>) => onChange({ ...f, properties: props.map((p, j) => (j === i ? { ...p, ...patch } : p)) });
  return (
    <div className="flex flex-col gap-1.5">
      <p className="m-0 text-11 text-muted">A message matches when every field set here equals the message's.</p>
      {CORRELATION_FIELDS.map(([k, label]) => (
        <Field key={k} label={label} inline={140}>
          {(ids) => <Input {...ids} mono value={f[k] ?? ""} onChange={(e) => set(k, e.target.value)} />}
        </Field>
      ))}
      <div role="group" aria-label="Properties" className="mt-1 flex flex-col gap-1">
        <span className="text-12 font-semibold text-muted">Properties</span>
        {props.length === 0 && <p className="m-0 text-11 text-muted">No user properties. Values are matched as text.</p>}
        {props.map((p, i) => (
          <div key={i} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_24px] items-center gap-1.5">
            <Input mono aria-label={`Name of property ${i + 1}`} placeholder="name" value={p.name} onChange={(e) => setProp(i, { name: e.target.value })} />
            <Input mono aria-label={`Value of property ${i + 1}`} placeholder="value" value={p.value} onChange={(e) => setProp(i, { value: e.target.value })} />
            <IconButton label={`Remove property ${i + 1}`} onClick={() => onChange({ ...f, properties: props.filter((_, j) => j !== i) })}>
              <X aria-hidden="true" />
            </IconButton>
          </div>
        ))}
        <Button className="self-start" onClick={() => onChange({ ...f, properties: [...props, { name: "", value: "" }] })}>
          <Plus aria-hidden="true" className="size-3.5" />
          Add property
        </Button>
      </div>
    </div>
  );
}
