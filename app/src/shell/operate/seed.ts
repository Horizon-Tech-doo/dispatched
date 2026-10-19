import type { FilterSpec, MessageDetail, PeekView, SubQueue } from "@/engine";

/** What an operation sheet opens on. Sequence numbers are the grid's numbers; they become strings at the engine call. */
export type OperationSeed =
  /** The composer; `clone` seeds it from a message (Clone & resend). */
  | { op: "send"; entity: string | null; clone?: MessageDetail | null }
  | { op: "import"; entity: string | null }
  | { op: "receive"; entity: string; subQueue: SubQueue; session?: string | null }
  /** Selected rows, or (none selected) every message matching the view's filter. */
  | { op: "delete" | "move" | "resubmit"; entity: string; view: PeekView; sequences: number[]; filter: FilterSpec; total?: number | null; detail?: MessageDetail | null }
  | { op: "cancel-scheduled"; entity: string; sequences: number[] }
  | { op: "settle-deferred"; entity: string; subQueue: SubQueue; sequences: number[]; session: string | null }
  | { op: "purge"; entities: string[] }
  | { op: "session-state"; entity: string; session: string };

export type OperationKind = OperationSeed["op"];
