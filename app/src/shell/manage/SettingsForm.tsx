import type { EntityKindName, FieldIssue, SettingName } from "@/engine";
import { useState } from "react";
import { Field, Input, Textarea } from "@/ui";
import { Check, Group } from "../operate/controls";
import { EntityPicker } from "../operate/EntityPicker";
import { GROUPS, type FieldDef, type Values } from "./fields";
import { sentence } from "./RuleEditor";
import { issueOf } from "./useIssues";

const FIXED_NOTE = "Set when the entity was created.";

/** A whole number typed in a number field; anything else stays text, so the field can say what is wrong. */
const numberOf = (t: string): number | string | null => (t.trim() === "" ? null : /^\d+$/.test(t.trim()) ? Number(t.trim()) : t);

/** Local problems the engine is not asked about: text in a number field. */
export function localIssues(kind: EntityKindName, values: Values): FieldIssue[] {
  return GROUPS.flatMap((g) => g.fields)
    .filter((f) => f.kinds.includes(kind) && f.input === "number" && typeof values[f.name] === "string")
    .map((f) => ({ field: f.name, message: "Enter a whole number." }));
}

/**
 * Every setting of an entity kind, in groups. On edit, settings fixed at creation are shown read-only. Durations are
 * typed as text (45s, 5m, 1h30m, 14d); those that can be "never" have a checkbox for it.
 */
export function SettingsForm({
  kind,
  entity,
  values,
  onChange,
  issues,
  fixed,
}: {
  kind: EntityKindName;
  /** The entity itself (edit), so it is not offered as its own forwarding target. */
  entity: string | null;
  values: Values;
  onChange(name: SettingName, v: Values[SettingName]): void;
  issues: FieldIssue[];
  /** Settings shown read-only (edit). */
  fixed: SettingName[];
}) {
  return (
    <>
      {GROUPS.map((g) => {
        const fields = g.fields.filter((f) => f.kinds.includes(kind));
        if (!fields.length) return null;
        return (
          <Group key={g.label} label={g.label}>
            {fields.map((f) => (
              <SettingField key={f.name} f={f} entity={entity} value={values[f.name] ?? null} onChange={(v) => onChange(f.name, v)} issue={issueOf(issues, f.name)} fixed={fixed.includes(f.name)} />
            ))}
          </Group>
        );
      })}
    </>
  );
}

function SettingField({
  f,
  entity,
  value,
  onChange,
  issue,
  fixed,
}: {
  f: FieldDef;
  entity: string | null;
  value: Values[SettingName];
  onChange(v: Values[SettingName]): void;
  issue?: string;
  fixed: boolean;
}) {
  const error = issue ? sentence(issue) : undefined;
  if (f.input === "bool")
    return (
      <div className="flex flex-col gap-0.5">
        <Check label={f.label} hint={fixed ? <>{f.help} <span>{FIXED_NOTE}</span></> : f.help} checked={value === true} disabled={fixed} onChange={(v) => onChange(v)} />
        {error && <p className="m-0 pl-5 text-11 text-fg">{error}</p>}
      </div>
    );
  if (f.input === "entity") return <ForwardField f={f} entity={entity} value={typeof value === "string" ? value : null} onChange={onChange} error={error} />;
  if (f.input === "text")
    return (
      <Field label={f.label} hint={f.help} error={error}>
        {(ids) => <Textarea {...ids} rows={2} value={typeof value === "string" ? value : ""} onChange={(e) => onChange(e.target.value)} />}
      </Field>
    );
  if (f.input === "number")
    return (
      <Field label={f.label} hint={f.help} error={error}>
        {(ids) => <Input {...ids} mono inputMode="numeric" className="w-32" value={value === null || value === undefined ? "" : String(value)} onChange={(e) => onChange(numberOf(e.target.value))} />}
      </Field>
    );
  // A duration.
  const never = !!f.never && typeof value === "string" && /^(max|never)$/i.test(value);
  return (
    <Field label={f.label} hint={f.help} error={error}>
      {(ids) => (
        <div className="flex items-center gap-3">
          <Input
            {...ids}
            mono
            className="w-40"
            placeholder="45s, 5m, 1h30m, 14d"
            disabled={never}
            value={never || typeof value !== "string" ? "" : value}
            onChange={(e) => onChange(e.target.value || null)}
          />
          {f.never && <Check label={f.never} checked={never} onChange={(on) => onChange(on ? "max" : null)} />}
        </div>
      )}
    </Field>
  );
}

/** A forwarding target: off, or a queue or topic picked from the namespace. */
function ForwardField({ f, entity, value, onChange, error }: { f: FieldDef; entity: string | null; value: string | null; onChange(v: string | null): void; error?: string }) {
  const [on, setOn] = useState(value !== null);
  return (
    <div className="flex flex-col gap-1">
      <Check
        label={f.label}
        hint={f.help}
        checked={on}
        onChange={(v) => {
          setOn(v);
          if (!v) onChange(null);
        }}
      />
      {on && (
        <div className="pl-5">
          <EntityPicker label={f.label} value={value} onChange={(v) => onChange(v)} exclude={entity ?? undefined} />
          {!value && <p className="m-0 mt-1 text-11 text-muted">Choose the queue or topic.</p>}
          {error && <p className="m-0 mt-1 text-11 text-fg">{error}</p>}
        </div>
      )}
    </div>
  );
}
