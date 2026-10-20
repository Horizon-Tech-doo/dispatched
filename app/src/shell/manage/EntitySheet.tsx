import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { commands, keys, type EntityKindName, type RuleForm, type SettingName } from "@/engine";
import { Field, Input, SkeletonRows } from "@/ui";
import { useShell } from "../context";
import { displayName } from "../entityId";
import { Group } from "../operate/controls";
import { OperationSheet } from "../operate/OperationSheet";
import { useOperation } from "../operate/useOperation";
import { changesOf, createdOf, type Values } from "./fields";
import { RuleEditor } from "./RuleEditor";
import { SettingsForm, localIssues } from "./SettingsForm";
import { useIssues } from "./useIssues";

const KIND_WORD: Record<EntityKindName, string> = { queue: "queue", topic: "topic", subscription: "subscription" };
const nameIssue = (name: string) => (/[/\\?#]/.test(name) ? "A name has no '/', '\\', '?' or '#'." : undefined);

/** A new queue, topic or subscription (a subscription with its first rule). Only the settings given are sent; the
 *  service's defaults apply to the rest. */
export function EntityCreateSheet({ kind, topic }: { kind: EntityKindName; topic: string | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [name, setName] = useState("");
  const [values, setValues] = useState<Values>({});
  const [rule, setRule] = useState<RuleForm>({ name: "", filter: { kind: "true" }, action: null });
  const settings = useMemo(() => createdOf(values), [values]);
  const local = localIssues(kind, values);
  const checked = useIssues(local.length ? {} : settings, (c) => commands.checkSettings(kind, c, true));
  const withRule = kind === "subscription" && (rule.filter.kind !== "true" || !!rule.action);
  const ruleCheck = useIssues(rule, commands.checkRule, withRule);
  const issues = [...local, ...checked.issues];
  const entity = kind === "subscription" ? `${topic}/subscriptions/${name.trim()}` : name.trim();
  const badName = nameIssue(name);
  const title = kind === "subscription" ? `New subscription on ${topic}` : `New ${KIND_WORD[kind]}`;
  return (
    <OperationSheet
      title={title}
      description="Settings left empty take the service's defaults."
      op={op}
      right="Manage"
      action={`Create ${KIND_WORD[kind]}`}
      failureTitle={`Couldn't create the ${KIND_WORD[kind]}`}
      canSubmit={!!conn.profileId && !!name.trim() && !badName && !checked.pending && !issues.length && !(withRule && (ruleCheck.pending || ruleCheck.issues.length))}
      onSubmit={() =>
        void op.start(() =>
          commands.opEntityCreate(conn.profileId!, kind, entity, settings, withRule ? { ...rule, name: rule.name.trim() || "$Default" } : null),
        )
      }
      onClose={closeSheet}
    >
      <Field label="Name" error={badName}>
        {(ids) => <Input {...ids} mono value={name} onChange={(e) => setName(e.target.value)} />}
      </Field>
      <SettingsForm kind={kind} entity={null} values={values} onChange={(k, v) => setValues((p) => ({ ...p, [k]: v }))} issues={issues} fixed={[]} />
      {kind === "subscription" && (
        <Group label="Initial rule">
          <p className="m-0 text-11 text-muted">The default rule lets every message through. Choose a filter to start with another rule instead.</p>
          <RuleEditor value={rule} onChange={setRule} issues={withRule ? ruleCheck.issues : []} nameLabel="Rule name" showName={rule.filter.kind !== "true"} />
        </Group>
      )}
    </OperationSheet>
  );
}

/** An entity's settings, as they are now; saving sends only what was changed. */
export function EntityEditSheet({ kind, entity }: { kind: EntityKindName; entity: string }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const q = useQuery({
    queryKey: keys.settings(conn.profileId, entity),
    queryFn: () => commands.entitySettings(conn.profileId!, kind, entity),
    enabled: conn.profileId !== null,
    // The form edits what was read when it opened.
    staleTime: Infinity,
  });
  const initial = useMemo(() => (q.data?.settings ?? {}) as Values, [q.data]);
  const [values, setValues] = useState<Values | null>(null);
  useEffect(() => {
    if (q.data && values === null) setValues(q.data.settings as Values);
  }, [q.data, values]);
  const current = values ?? initial;
  const changes = useMemo(() => changesOf(initial, current), [initial, current]);
  const n = Object.keys(changes).length;
  const local = localIssues(kind, current);
  const checked = useIssues(local.length ? {} : changes, (c) => commands.checkSettings(kind, c, false), n > 0);
  const issues = [...local, ...checked.issues];
  return (
    <OperationSheet
      title={`Settings of ${displayName(entity)}`}
      description="Only the settings you change are saved; the rest stay as they are on the service, even if someone changes them meanwhile."
      op={op}
      right="Manage"
      action={`Save ${n} ${n === 1 ? "change" : "changes"}`}
      failureTitle="Couldn't save the settings"
      canSubmit={!!conn.profileId && n > 0 && !checked.pending && !issues.length}
      onSubmit={() => void op.start(() => commands.opEntityUpdate(conn.profileId!, kind, entity, changes))}
      onClose={closeSheet}
    >
      {q.isPending ? (
        <SkeletonRows rows={8} />
      ) : q.isError ? (
        <p className="m-0 text-12 text-fg">Couldn't read the settings: {q.error.message}</p>
      ) : (
        <SettingsForm
          kind={kind}
          entity={entity}
          values={current}
          onChange={(k: SettingName, v) => setValues((p) => ({ ...(p ?? initial), [k]: v }))}
          issues={issues}
          fixed={q.data.fixed}
        />
      )}
    </OperationSheet>
  );
}
