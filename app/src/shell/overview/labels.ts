import { formatBytes, formatNumber } from "@/ui";
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

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** `PT1M` becomes `1 min`, `P14D` becomes `14 days`; .NET's TimeSpan.MaxValue (`P10675199D...`) is "Never".
 *  Null for text that is not a day-and-time ISO 8601 duration. */
export function formatIsoDuration(v: string): string | null {
  const m = /^P(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?)?$/.exec(v);
  if (!m || v === "P" || v.endsWith("T")) return null;
  const [d, h, min, s] = [m[1], m[2], m[3], m[4]].map((x) => (x ? Number(x) : 0));
  if (d >= 10_675_199) return "Never";
  const parts = [d && plural(d, "day", "days"), h && `${h} h`, min && `${min} min`, s && `${Number(s.toFixed(3))} s`].filter(Boolean);
  return parts.length ? parts.join(" ") : "0 s";
}

const isCount = (v: string) => /^\d+$/.test(v);

/** A service property's raw value as the person reads it: flags, local times, sizes and durations. Anything else as is. */
export function formatProperty(name: string, value: string): string {
  if (value === "true") return "Yes";
  if (value === "false") return "No";
  if (/At$/.test(name)) return formatTimestamp(value) || value;
  if (name === "MaxSizeInMegabytes" && isCount(value)) return `${formatNumber(Number(value))} MB`;
  if (name === "MaxMessageSizeInKilobytes" && isCount(value)) return `${formatNumber(Number(value))} KB`;
  if ((name === "MessageCount" || name === "SubscriptionCount") && isCount(value)) return formatNumber(Number(value));
  if (name === "SizeInBytes" && isCount(value)) return formatBytes(Number(value));
  if (/^P/.test(value)) return formatIsoDuration(value) ?? value;
  return value;
}
