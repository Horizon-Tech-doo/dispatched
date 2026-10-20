import type { EntityKindName, SettingName, SettingsChange } from "@/engine";

export type Input = "duration" | "number" | "bool" | "entity" | "text";

/** One setting in the entity form. */
export interface FieldDef {
  name: SettingName;
  label: string;
  /** What it does, under the label. */
  help: string;
  input: Input;
  kinds: EntityKindName[];
  /** Durations that take "never" (TimeSpan.MaxValue): the label of that choice. */
  never?: string;
  /** Shown in the Size group with "Premium only". */
  premium?: boolean;
}

export interface GroupDef {
  label: string;
  fields: FieldDef[];
}

const Q: EntityKindName = "queue";
const T: EntityKindName = "topic";
const S: EntityKindName = "subscription";

/** The form's groups and fields, in order. Status has its own sheet. */
export const GROUPS: GroupDef[] = [
  {
    label: "Messages",
    fields: [
      { name: "lock_duration", label: "Lock duration", help: "How long a received message stays locked to its receiver. Up to 5 minutes.", input: "duration", kinds: [Q, S] },
      { name: "max_delivery_count", label: "Max delivery count", help: "Deliveries before a message is dead-lettered.", input: "number", kinds: [Q, S] },
      { name: "default_message_time_to_live", label: "Message time to live", help: "How long a message waits before it expires.", input: "duration", kinds: [Q, T, S], never: "Never expire" },
      { name: "dead_lettering_on_message_expiration", label: "Dead-letter expired messages", help: "An expired message moves to the dead-letter queue instead of being dropped.", input: "bool", kinds: [Q, S] },
      { name: "dead_lettering_on_filter_evaluation_exceptions", label: "Dead-letter on filter errors", help: "A message a rule's filter fails on moves to the dead-letter queue.", input: "bool", kinds: [S] },
    ],
  },
  {
    label: "Duplicate detection",
    fields: [
      { name: "requires_duplicate_detection", label: "Duplicate detection", help: "Drop a message whose MessageId was seen within the window.", input: "bool", kinds: [Q, T] },
      { name: "duplicate_detection_history_time_window", label: "Detection window", help: "How far back MessageIds are remembered.", input: "duration", kinds: [Q, T] },
    ],
  },
  {
    label: "Forwarding",
    fields: [
      { name: "forward_to", label: "Forward messages to", help: "Every message moves on to this queue or topic.", input: "entity", kinds: [Q, S] },
      { name: "forward_dead_lettered_messages_to", label: "Forward dead-lettered messages to", help: "Dead-lettered messages move on to this queue or topic.", input: "entity", kinds: [Q, S] },
    ],
  },
  {
    label: "Size",
    fields: [
      { name: "max_size_in_megabytes", label: "Max size (MB)", help: "Sends are refused once the entity holds this much.", input: "number", kinds: [Q, T] },
      { name: "max_message_size_in_kilobytes", label: "Max message size (KB)", help: "Premium only. The largest message the entity accepts.", input: "number", kinds: [Q, T], premium: true },
    ],
  },
  {
    label: "Lifecycle",
    fields: [{ name: "auto_delete_on_idle", label: "Delete when idle for", help: "The entity is deleted after this long without use.", input: "duration", kinds: [Q, T, S], never: "Never delete" }],
  },
  {
    label: "Advanced",
    fields: [
      { name: "requires_session", label: "Sessions", help: "Messages are received in order per session id.", input: "bool", kinds: [Q, S] },
      { name: "enable_partitioning", label: "Partitioning", help: "Spread the entity over several message brokers (Standard).", input: "bool", kinds: [Q, T] },
      { name: "enable_batched_operations", label: "Batched store operations", help: "The service batches writes; the default.", input: "bool", kinds: [Q, T, S] },
      { name: "enable_express", label: "Express", help: "Keep messages in memory before writing them (Standard).", input: "bool", kinds: [Q, T] },
      { name: "support_ordering", label: "Ordered delivery", help: "Subscriptions receive messages in the order they were sent.", input: "bool", kinds: [T] },
    ],
  },
  {
    label: "Metadata",
    fields: [{ name: "user_metadata", label: "User metadata", help: "Free text kept with the entity.", input: "text", kinds: [Q, T, S] }],
  },
];

/** Every field of a kind's form. */
export const fieldsOf = (kind: EntityKindName) => GROUPS.flatMap((g) => g.fields).filter((f) => f.kinds.includes(kind));

export type Values = Partial<Record<SettingName, string | number | boolean | null>>;

/** A text-like setting cleared to nothing is sent as `""`, which clears it; other settings cleared are left out. */
const CLEARABLE: SettingName[] = ["forward_to", "forward_dead_lettered_messages_to", "user_metadata"];

const blank = (v: unknown) => v === null || v === undefined || v === "";

/** The settings whose value differs from what the form opened with: the change to send. */
export function changesOf(initial: Values, values: Values): SettingsChange {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(values) as [SettingName, Values[SettingName]][]) {
    const was = initial[k];
    if (blank(v) && blank(was)) continue;
    if (v === was) continue;
    if (blank(v)) {
      if (CLEARABLE.includes(k)) out[k] = "";
      continue;
    }
    out[k] = v;
  }
  return out as SettingsChange;
}

/** The settings a create form sets: every value given. */
export function createdOf(values: Values): SettingsChange {
  return Object.fromEntries(Object.entries(values).filter(([, v]) => !blank(v))) as SettingsChange;
}
