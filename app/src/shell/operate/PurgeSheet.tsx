import { useEffect, useState } from "react";
import { EngineError, commands, type PurgePlans, type PurgeScope, type SubQueue } from "@/engine";
import { FieldMessage, Problem, Segmented } from "@/ui";
import { useShell } from "../context";
import { useAuthKind } from "../access";
import { adviceFor, rightFor } from "./advice";
import { Group } from "./controls";
import { OperationSheet } from "./OperationSheet";
import { useOperation } from "./useOperation";

const SUB: Record<SubQueue, string> = { active: "Active", "dead-letter": "Dead-letter", "transfer-dead-letter": "Transfer DLQ" };
const SCOPE_NOUN: Record<SubQueue, string> = { active: "active messages", "dead-letter": "dead-letter messages", "transfer-dead-letter": "transfer dead-letter messages" };

/** One line of a plan: what, and how many in the core's words (already grouped, "about 1,284 messages"). */
function PlanLine({ label, value, strong }: { label: string; value: string; strong?: boolean }) {
  return (
    <p className="m-0 grid grid-cols-[96px_minmax(0,1fr)] text-12">
      <span className="text-muted">{label}</span>
      <span className={strong ? "tabular-nums font-semibold" : "tabular-nums"}>{value}</span>
    </p>
  );
}

/**
 * Purge one or more entities. Never one click: the counts come first (a plan per entity, in the core's words), then
 * Purge, then one confirmation per entity. A plan whose count is unknown is refused here, before anything is asked.
 * A plan runs once; changing the scope drops it.
 */
export function PurgeSheet({ entities }: { entities: string[] }) {
  const { conn, closeSheet } = useShell();
  const auth = useAuthKind();
  const op = useOperation();
  const [scope, setScope] = useState<PurgeScope>("active");
  const [plan, setPlan] = useState<PurgePlans | null>(null);
  const [counting, setCounting] = useState(false);
  const [countFailed, setCountFailed] = useState<{ kind: string; message: string } | null>(null);
  // A refused purge used up its plan (or the plan had expired): count again.
  useEffect(() => {
    if (op.failure) setPlan(null);
  }, [op.failure]);
  const unknown = plan?.plans.filter((p) => p.unknown) ?? [];
  const what = entities.length === 1 ? entities[0] : `${entities.length} entities`;

  const count = async () => {
    if (!conn.profileId) return;
    setCounting(true);
    setCountFailed(null);
    try {
      setPlan(await commands.purgePlan(conn.profileId, entities, scope));
    } catch (e) {
      setCountFailed({ kind: e instanceof EngineError ? e.kind : "unknown", message: e instanceof Error ? e.message : String(e) });
    } finally {
      setCounting(false);
    }
  };
  const submit = () => {
    if (!plan) return void count();
    const id = plan.plan_id;
    setPlan(null);
    void op.start(() => commands.opPurge(conn.profileId!, id));
  };

  return (
    <OperationSheet
      title="Purge messages"
      description="Every message in the chosen queues of each entity is removed. Nothing can bring them back."
      op={op}
      action={plan ? `Purge ${what}` : "Count messages"}
      failureTitle="Couldn't purge"
      right="Listen"
      canSubmit={!!conn.profileId && !counting && unknown.length === 0}
      onSubmit={submit}
      onClose={closeSheet}
    >
      <Group label={entities.length === 1 ? "Entity" : "Entities"}>
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0 text-12">
          {entities.map((e) => (
            <li key={e} className="break-all">
              {e}
            </li>
          ))}
        </ul>
      </Group>
      <Group label="Scope">
        <Segmented
          label="Scope"
          value={scope}
          onChange={(s) => {
            setScope(s);
            setPlan(null);
          }}
          options={[
            { value: "active", label: "Active" },
            { value: "dead-letter", label: "Dead-letter" },
            { value: "transfer-dead-letter", label: "Transfer DLQ" },
            { value: "all", label: "All" },
          ]}
        />
        <p className="m-0 text-12 text-muted">
          {scope === "all" ? "Active, deferred and both dead-letter queues." : `Only the ${SCOPE_NOUN[scope as SubQueue]}.`} Scheduled messages are not purged; cancel them from Scheduled.
        </p>
      </Group>
      {countFailed && (
        <Problem role="alert" title="Couldn't count the messages" body={`Nothing was changed. ${adviceFor(countFailed.kind, countFailed.message, rightFor("count"), auth)}`} detail={countFailed.message} />
      )}
      {plan && (
        <div className="flex flex-col gap-3">
          {plan.plans.map((p) => (
            <div key={p.entity} role="group" aria-label={`Plan for ${p.entity}`} className="flex flex-col gap-0.5 border-t border-line pt-2">
              <h4 className="m-0 break-all text-12 font-semibold">{p.entity}</h4>
              {p.lines.map((l) => (
                <PlanLine key={l.sub_queue} label={SUB[l.sub_queue]} value={l.count} />
              ))}
              {p.deferred && <PlanLine label="Deferred" value={p.deferred} />}
              <PlanLine label="Total" value={p.total} strong />
              {p.requires_session && <p className="m-0 text-11 text-muted">A session entity: each session is purged in turn.</p>}
              {p.unknown && (
                <FieldMessage
                  error={`The count of ${p.entity} is unknown, so it can't be purged. Refresh the tree and try again; if it stays unknown, check that the connection can read counts.`}
                />
              )}
            </div>
          ))}
          <p className="m-0 text-11 text-muted">Counts expire after 10 minutes or one purge. If the sheet stays open, count again.</p>
        </div>
      )}
    </OperationSheet>
  );
}
