import { formatBytes, formatNumber } from "@/ui";
import type { EntityProperty } from "@/engine";
import { formatIsoDuration } from "../duration";
import { formatTimestamp } from "../time";

const LABEL: Record<string, string> = {
  LockDuration: "Lock duration", MaxSizeInMegabytes: "Max size", RequiresDuplicateDetection: "Duplicate detection",
  RequiresSession: "Sessions", DefaultMessageTimeToLive: "Message time to live", DeadLetteringOnMessageExpiration: "Dead-letter expired messages",
  DeadLetteringOnFilterEvaluationExceptions: "Dead-letter filter errors", DuplicateDetectionHistoryTimeWindow: "Duplicate detection window",
  MaxDeliveryCount: "Max delivery count", EnableBatchedOperations: "Batched operations", EnablePartitioning: "Partitioning",
  EnableExpress: "Express", ForwardTo: "Forwards to", ForwardDeadLetteredMessagesTo: "Forwards dead letters to",
  AutoDeleteOnIdle: "Delete when idle", UserMetadata: "User metadata", CreatedAt: "Created", UpdatedAt: "Updated",
  AccessedAt: "Last accessed", MessageCount: "Messages", MaxMessageSizeInKilobytes: "Max message size", SupportOrdering: "Ordering",
  Status: "Status", SizeInBytes: "Size", SubscriptionCount: "Subscriptions", EntityAvailabilityStatus: "Availability",
};

/** A service property's name in plain words; a name this list does not know is split into lower-case words. */
export function labelOf(name: string): string {
  if (LABEL[name]) return LABEL[name];
  const words = name.replace(/([a-z0-9])([A-Z])/g, "$1 $2").toLowerCase();
  return words ? words[0].toUpperCase() + words.slice(1) : name;
}

const isCount = (v: string) => /^\d+$/.test(v);

/** Only these are rewritten: a name this list does not know keeps the value the service wrote, whatever it looks like. */
const FLAGS = new Set([
  "RequiresDuplicateDetection", "RequiresSession", "DeadLetteringOnMessageExpiration", "DeadLetteringOnFilterEvaluationExceptions",
  "EnableBatchedOperations", "EnablePartitioning", "EnableExpress", "SupportOrdering",
]);
const DURATIONS = new Set(["LockDuration", "DefaultMessageTimeToLive", "DuplicateDetectionHistoryTimeWindow", "AutoDeleteOnIdle"]);

/** `CreatedAt`, `UpdatedAt`, `AccessedAt`: shown as machine text. */
export const isTimestamp = (name: string) => /^[A-Z]\w*At$/.test(name);

/** A service property's raw value as the person reads it: flags, local times, sizes and durations of the properties
 *  that are known to hold them. Anything else as is. */
export function formatProperty(name: string, value: string): string {
  if (FLAGS.has(name) && (value === "true" || value === "false")) return value === "true" ? "Yes" : "No";
  if (isTimestamp(name)) return formatTimestamp(value) || value;
  if (DURATIONS.has(name)) return formatIsoDuration(value) ?? value;
  if (!isCount(value)) return value;
  const n = Number(value);
  switch (name) {
    case "MaxSizeInMegabytes": return `${formatNumber(n)} MB`;
    case "MaxMessageSizeInKilobytes": return `${formatNumber(n)} KB`;
    case "SizeInBytes": return formatBytes(n);
    case "MessageCount": case "SubscriptionCount": return formatNumber(n);
    default: return value;
  }
}

const GROUP_OF: Record<string, "Delivery" | "Features" | "Lifecycle"> = {
  LockDuration: "Delivery", MaxDeliveryCount: "Delivery", RequiresSession: "Delivery", ForwardTo: "Delivery", ForwardDeadLetteredMessagesTo: "Delivery",
  DeadLetteringOnMessageExpiration: "Delivery", DeadLetteringOnFilterEvaluationExceptions: "Delivery",
  RequiresDuplicateDetection: "Features", DuplicateDetectionHistoryTimeWindow: "Features", EnablePartitioning: "Features", EnableBatchedOperations: "Features",
  EnableExpress: "Features", SupportOrdering: "Features", MaxSizeInMegabytes: "Features", MaxMessageSizeInKilobytes: "Features", UserMetadata: "Features",
  DefaultMessageTimeToLive: "Lifecycle", AutoDeleteOnIdle: "Lifecycle", CreatedAt: "Lifecycle", UpdatedAt: "Lifecycle", AccessedAt: "Lifecycle",
};
export const GROUPS = ["Delivery", "Features", "Lifecycle", "Other"] as const;
export interface PropertyGroup { title: (typeof GROUPS)[number]; items: EntityProperty[] }

/** The properties in groups (each keeps the service's order; empty groups are left out). What the counts block shows is
 *  left out on request: `size` the size, `maxSize` its limit (only when the block could show it), `messageCount` the
 *  message count, `subscriptionCount` the number of subscriptions (a topic's table has it). */
export function groupProperties(props: EntityProperty[], drop: { size?: boolean; maxSize?: boolean; messageCount?: boolean; subscriptionCount?: boolean }): PropertyGroup[] {
  const shown = props.filter((p) => !(drop.size && p.name === "SizeInBytes") && !(drop.maxSize && p.name === "MaxSizeInMegabytes") && !(drop.messageCount && p.name === "MessageCount") && !(drop.subscriptionCount && p.name === "SubscriptionCount"));
  return GROUPS.map((title) => ({ title, items: shown.filter((p) => (GROUP_OF[p.name] ?? "Other") === title) })).filter((g) => g.items.length > 0);
}

/** The raw names and values as the service reported them, as JSON text. */
export const propertiesJson = (props: EntityProperty[]) => JSON.stringify(Object.fromEntries(props.map((p) => [p.name, p.value])), null, 2);
