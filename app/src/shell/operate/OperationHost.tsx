import { ComposerSheet } from "./ComposerSheet";
import { DeleteSheet } from "./DeleteSheet";
import { ImportSheet } from "./ImportSheet";
import { MoveSheet } from "./MoveSheet";
import { PurgeSheet } from "./PurgeSheet";
import { SessionStateSheet } from "../sessions/SessionStateSheet";
import { ReceiveSheet } from "./ReceiveSheet";
import { ResubmitSheet } from "./ResubmitSheet";
import { CancelScheduledSheet, SettleDeferredSheet } from "./ScheduledSheets";
import type { OperationSeed } from "./seed";
import { EntityCreateSheet, EntityEditSheet } from "../manage/EntitySheet";
import { PolicyAddSheet, PolicyChangeSheet } from "../manage/PolicySheet";
import { RuleRemoveSheet, RuleSaveSheet } from "../manage/RuleSheet";

/** Renders the sheet of one operation. */
export function OperationHost({ seed }: { seed: OperationSeed }) {
  switch (seed.op) {
    case "send":
      return <ComposerSheet entity={seed.entity} clone={seed.clone} />;
    case "import":
      return <ImportSheet entity={seed.entity} />;
    case "receive":
      return <ReceiveSheet entity={seed.entity} subQueue={seed.subQueue} session={seed.session} />;
    case "delete":
      return <DeleteSheet {...seed} />;
    case "move":
      return <MoveSheet {...seed} />;
    case "resubmit":
      return <ResubmitSheet {...seed} />;
    case "cancel-scheduled":
      return <CancelScheduledSheet entity={seed.entity} sequences={seed.sequences} />;
    case "settle-deferred":
      return <SettleDeferredSheet {...seed} />;
    case "purge":
      return <PurgeSheet entities={seed.entities} />;
    case "session-state":
      return <SessionStateSheet entity={seed.entity} session={seed.session} />;
    case "entity-create":
      return <EntityCreateSheet kind={seed.entityKind} topic={seed.topic} />;
    case "entity-edit":
      return <EntityEditSheet kind={seed.entityKind} entity={seed.entity} />;
    case "rule-save":
      return <RuleSaveSheet entity={seed.entity} rule={seed.rule} />;
    case "rule-remove":
      return <RuleRemoveSheet entity={seed.entity} name={seed.name} last={seed.last} />;
    case "policy-add":
      return <PolicyAddSheet kind={seed.entityKind} entity={seed.entity} />;
    case "policy-change":
      return <PolicyChangeSheet kind={seed.entityKind} entity={seed.entity} name={seed.name} change={seed.change} />;
  }
}

/** A key per opened operation: another seed starts a fresh sheet. A cloned message counts by its sequence number (its
 *  body can be megabytes). */
export const operationKey = (seed: OperationSeed): string =>
  JSON.stringify(seed, (k, v) => ((k === "clone" || k === "detail") && v ? `seq:${(v as { sequence_number: number }).sequence_number}` : v));
