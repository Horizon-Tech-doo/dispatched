import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { commands, keys, type AccessRight, type EntityKindName, type PolicyChange } from "@/engine";
import { Field, Input } from "@/ui";
import { useShell } from "../context";
import { Check, Group } from "../operate/controls";
import { OperationSheet } from "../operate/OperationSheet";
import { useOperation } from "../operate/useOperation";

/** An entity holds at most this many policies. */
export const POLICY_LIMIT = 12;

/** Add a shared access policy with its rights. The service makes two keys; they are never shown. */
export function PolicyAddSheet({ kind, entity }: { kind: EntityKindName; entity: string }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [name, setName] = useState("");
  const [rights, setRights] = useState<Set<AccessRight>>(new Set(["listen"]));
  const existing = useQuery({
    queryKey: keys.policies(conn.profileId, entity),
    queryFn: () => commands.entityPolicies(conn.profileId!, kind, entity),
    enabled: conn.profileId !== null,
  });
  const manage = rights.has("manage");
  const full = (existing.data?.length ?? 0) >= POLICY_LIMIT;
  const taken = existing.data?.some((p) => p.key_name === name.trim()) ?? false;
  const toggle = (r: AccessRight, on: boolean) => {
    const next = new Set(rights);
    if (on) next.add(r);
    else next.delete(r);
    // Manage includes Listen and Send.
    if (r === "manage" && on) next.add("listen").add("send");
    setRights(next);
  };
  const ordered = (["listen", "send", "manage"] as const).filter((r) => rights.has(r));
  const nameError = taken ? `${entity} already has a policy named ${name.trim()}.` : /[\s/]/.test(name) ? "A policy name has no spaces or '/'." : undefined;
  return (
    <OperationSheet
      title="Add policy"
      description="The new policy gets two keys. Copy them from the policy list; they are never shown."
      op={op}
      right="Manage"
      action="Add policy"
      failureTitle="Couldn't add the policy"
      canSubmit={!!conn.profileId && !!name.trim() && !nameError && ordered.length > 0 && !full}
      onSubmit={() => void op.start(() => commands.opPolicyChange(conn.profileId!, { kind, entity }, name.trim(), { type: "add", rights: ordered }))}
      onClose={closeSheet}
    >
      {full && <p className="m-0 text-12 text-fg">{entity} has {POLICY_LIMIT} policies, the most an entity can hold. Remove one first.</p>}
      <Field label="Name" error={nameError}>
        {(ids) => <Input {...ids} mono value={name} onChange={(e) => setName(e.target.value)} />}
      </Field>
      <Group label="Rights">
        <Check label="Listen" hint="Receive and peek messages." checked={rights.has("listen")} disabled={manage} onChange={(v) => toggle("listen", v)} />
        <Check label="Send" hint="Send messages." checked={rights.has("send")} disabled={manage} onChange={(v) => toggle("send", v)} />
        <Check label="Manage" hint="Change the entity and its policies. Includes Listen and Send." checked={manage} onChange={(v) => toggle("manage", v)} />
      </Group>
    </OperationSheet>
  );
}

/** Remove a policy, or replace one of its keys. Either stops clients that use the old key. */
export function PolicyChangeSheet({ kind, entity, name, change }: { kind: EntityKindName; entity: string; name: string; change: Exclude<PolicyChange, { type: "add" }> }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const renew = change.type === "renew" ? change.slot : null;
  return (
    <OperationSheet
      title={renew ? `Regenerate ${renew} key of ${name}` : `Remove policy ${name}`}
      description={
        renew
          ? `Clients using it stop working. The ${renew === "primary" ? "secondary" : "primary"} key keeps working, so clients can move to it first.`
          : `Clients using its keys stop working.`
      }
      op={op}
      right="Manage"
      action={renew ? `Regenerate ${renew} key` : "Remove policy"}
      canSubmit={!!conn.profileId}
      onSubmit={() => void op.start(() => commands.opPolicyChange(conn.profileId!, { kind, entity }, name, change))}
      onClose={closeSheet}
    />
  );
}
