import type { EntityKindName, FilterSpec, MessageDetail, PeekView, PolicyChange, RuleInfo, Settlement, SubQueue } from "@/engine";

/** What an operation sheet opens on. Sequence numbers are the grid's numbers; they become strings at the engine call. */
export type OperationSeed =
  /** The composer; `clone` seeds it from a message (Clone & resend). */
  | { op: "send"; entity: string | null; clone?: MessageDetail | null }
  | { op: "import"; entity: string | null }
  | { op: "receive"; entity: string; subQueue: SubQueue; session?: string | null }
  /** Selected rows, or (none selected) every message matching the view's filter. */
  | { op: "delete" | "move" | "resubmit"; entity: string; view: PeekView; sequences: number[]; filter: FilterSpec; total?: number | null; detail?: MessageDetail | null }
  | { op: "cancel-scheduled"; entity: string; sequences: number[] }
  | { op: "settle-deferred"; entity: string; subQueue: SubQueue; sequences: number[]; session: string | null; how?: Settlement["type"] }
  | { op: "purge"; entities: string[] }
  | { op: "session-state"; entity: string; session: string }
  /** A new queue or topic, or (with `topic`) a new subscription on that topic. */
  | { op: "entity-create"; entityKind: EntityKindName; topic: string | null }
  | { op: "entity-edit"; entityKind: EntityKindName; entity: string }
  /** Add a rule to a subscription, or (with `rule`) edit that rule. */
  | { op: "rule-save"; entity: string; rule: RuleInfo | null }
  /** Remove a rule; `last` when it is the subscription's only rule. */
  | { op: "rule-remove"; entity: string; name: string; last: boolean }
  | { op: "policy-add"; entityKind: EntityKindName; entity: string }
  /** Remove a policy, or regenerate one of its keys. */
  | { op: "policy-change"; entityKind: EntityKindName; entity: string; name: string; change: Exclude<PolicyChange, { type: "add" }> };

export type OperationKind = OperationSeed["op"];
