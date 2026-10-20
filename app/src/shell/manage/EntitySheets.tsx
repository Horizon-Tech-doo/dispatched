import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { commands, keys, type EntityRefForm, type EntityStatus } from "@/engine";
import { EntityGlyph, Field, Input, Segmented, SkeletonRows } from "@/ui";
import { useShell } from "../context";
import { displayName } from "../entityId";
import { Group } from "../operate/controls";
import { OperationSheet } from "../operate/OperationSheet";
import { useOperation } from "../operate/useOperation";
import { noteRename } from "../operate/renames";

const STATUS: Record<EntityStatus, { label: string; meaning: string }> = {
  Active: { label: "Active", meaning: "Senders and receivers work." },
  Disabled: { label: "Disabled", meaning: "Nothing works: sends and receives are refused." },
  SendDisabled: { label: "Send disabled", meaning: "Senders are refused; receivers go on taking what is there." },
  ReceiveDisabled: { label: "Receive disabled", meaning: "Receivers are refused; senders go on adding messages." },
};

/** The statuses each kind takes. */
export const STATUSES: Record<EntityRefForm["kind"], EntityStatus[]> = {
  queue: ["Active", "Disabled", "SendDisabled", "ReceiveDisabled"],
  topic: ["Active", "Disabled", "SendDisabled"],
  subscription: ["Active", "Disabled", "ReceiveDisabled"],
};

/** Change an entity's status. */
export function StatusSheet({ target }: { target: EntityRefForm }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const q = useQuery({
    queryKey: keys.settings(conn.profileId, target.entity),
    queryFn: () => commands.entitySettings(conn.profileId!, target.kind, target.entity),
    enabled: conn.profileId !== null,
  });
  const current = (q.data?.settings.status ?? null) as EntityStatus | null;
  const [status, setStatus] = useState<EntityStatus | null>(null);
  const chosen = status ?? current;
  return (
    <OperationSheet
      title={`Status of ${displayName(target.entity)}`}
      op={op}
      right="Manage"
      action="Set status"
      canSubmit={!!conn.profileId && chosen !== null && chosen !== current}
      onSubmit={() => chosen && void op.start(() => commands.opEntityStatus(conn.profileId!, target, chosen))}
      onClose={closeSheet}
    >
      {q.isPending ? (
        <SkeletonRows rows={2} />
      ) : (
        <Group label="Status">
          <div>
          <Segmented<EntityStatus>
            label="Status"
            value={chosen ?? ("" as EntityStatus)}
            onChange={setStatus}
            options={STATUSES[target.kind].map((s) => ({ value: s, label: STATUS[s].label }))}
          />
          </div>
          <p className="m-0 text-12 text-muted">{chosen ? STATUS[chosen].meaning : "The current status couldn't be read."}</p>
        </Group>
      )}
    </OperationSheet>
  );
}

/** Duplicate (settings, not messages) or rename (create a copy, then delete the original) an entity. */
export function CopySheet({ target, rename }: { target: EntityRefForm; rename: boolean }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const old = displayName(target.entity);
  const [name, setName] = useState(`${old}-copy`);
  const newName = name.trim();
  const to = target.kind === "subscription" ? `${target.entity.split("/subscriptions/")[0]}/subscriptions/${newName}` : newName;
  // The tab of the old name follows the new one once the rename went through.
  useEffect(() => {
    if (rename && op.jobId) noteRename(op.jobId, target.entity, to);
    // Noted once per job.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [op.jobId]);
  const badName = /[/\\?#]/.test(name) ? "A name has no '/', '\\', '?' or '#'." : newName === old ? "Choose a name other than the original's." : undefined;
  return (
    <OperationSheet
      title={`${rename ? "Rename" : "Duplicate"} ${old}`}
      description={
        rename
          ? `Creates the new ${target.kind} with the same settings, then deletes ${old}. Refused while ${old} holds messages. Entities that forward to ${old} are not changed.`
          : `Copies the settings${target.kind === "subscription" ? " and every rule" : " and policies (with new keys)"}, not the messages.`
      }
      op={op}
      right="Manage"
      action={`${rename ? "Rename" : "Duplicate"} ${target.kind}`}
      failureTitle={rename ? `Couldn't rename ${old}` : `Couldn't duplicate ${old}`}
      canSubmit={!!conn.profileId && !!newName && !badName}
      onSubmit={() => void op.start(() => commands.opEntityCopy(conn.profileId!, target, newName, rename))}
      onClose={closeSheet}
    >
      <Field label="New name" error={badName}>
        {(ids) => <Input {...ids} mono value={name} onChange={(e) => setName(e.target.value)} />}
      </Field>
    </OperationSheet>
  );
}

/** Delete entities with everything they hold. On production each one is confirmed by its name. */
export function DeleteEntitiesSheet({ entities }: { entities: EntityRefForm[] }) {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const one = entities.length === 1 ? entities[0] : null;
  const title = one ? `Delete ${displayName(one.entity)}` : `Delete ${entities.length} entities`;
  return (
    <OperationSheet
      title={title}
      description="Everything they hold goes with them: messages, and a topic's subscriptions and rules. Nothing can bring them back."
      op={op}
      right="Manage"
      action={one ? `Delete ${one.kind}` : `Delete ${entities.length} entities`}
      canSubmit={!!conn.profileId && entities.length > 0}
      onSubmit={() => void op.start(() => commands.opEntityDelete(conn.profileId!, entities))}
      onClose={closeSheet}
    >
      <ul aria-label="Entities to delete" className="m-0 flex list-none flex-col gap-1 p-0">
        {entities.map((e) => (
          <li key={e.entity} className="flex items-center gap-2 text-12">
            <EntityGlyph kind={e.kind} className="size-3.5 shrink-0 text-icon" />
            <span className="font-mono text-11">{displayName(e.entity)}</span>
            {e.kind === "subscription" && <span className="text-muted">on {e.entity.split("/subscriptions/")[0]}</span>}
            {e.kind === "topic" && <span className="text-muted">and its subscriptions</span>}
          </li>
        ))}
      </ul>
      <p className="m-0 text-11 text-muted">The confirmation counts what each one holds.</p>
    </OperationSheet>
  );
}
