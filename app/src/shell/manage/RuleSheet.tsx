import { useState } from "react";
import { commands, type RuleInfo } from "@/engine";
import { useShell } from "../context";
import { displayName } from "../entityId";
import { OperationSheet } from "../operate/OperationSheet";
import { useOperation } from "../operate/useOperation";
import { RuleEditor, formOf, newRule } from "./RuleEditor";
import { useIssues } from "./useIssues";

/** Add a rule to a subscription, or edit one: its filter and action (a rule's name is fixed). */
export function RuleSaveSheet({ entity, rule }: { entity: string; rule: RuleInfo | null }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [form, setForm] = useState(() => (rule ? formOf(rule) : newRule()));
  const { issues, pending } = useIssues(form, commands.checkRule);
  const editing = rule !== null;
  return (
    <OperationSheet
      title={editing ? `Edit rule ${rule.name}` : "Add rule"}
      description={`Messages that match the filter reach ${displayName(entity)}.`}
      op={op}
      right="Manage"
      action={editing ? "Save rule" : "Add rule"}
      failureTitle={editing ? "Couldn't save the rule" : "Couldn't add the rule"}
      canSubmit={!!conn.profileId && !pending && issues.length === 0}
      onSubmit={() => void op.start(() => commands.opRuleSave(conn.profileId!, entity, form, editing))}
      onClose={closeSheet}
    >
      <RuleEditor value={form} onChange={setForm} issues={issues} nameReadOnly={editing} />
    </OperationSheet>
  );
}

/** Remove a rule. Removing a subscription's last rule means it receives nothing, and the sheet says so. */
export function RuleRemoveSheet({ entity, name, last }: { entity: string; name: string; last: boolean }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  return (
    <OperationSheet
      title={`Remove rule ${name}`}
      description={`${displayName(entity)} stops receiving the messages only this rule lets through.`}
      op={op}
      right="Manage"
      action="Remove rule"
      canSubmit={!!conn.profileId}
      onSubmit={() => void op.start(() => commands.opRuleRemove(conn.profileId!, entity, name))}
      onClose={closeSheet}
    >
      {last && <p className="m-0 text-12 text-fg">Without a rule this subscription receives no messages.</p>}
    </OperationSheet>
  );
}
