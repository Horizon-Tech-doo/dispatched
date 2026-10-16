import { useQuery } from "@tanstack/react-query";
import { commands, keys, useEntities, type EntityCounts, type PeekView } from "@/engine";
import { useShell } from "../context";
import { splitSubscriptionId } from "../entityId";
import type { EntityKind, EntityTab, SubTabId } from "../state";

export interface SubTabDef {
  id: SubTabId;
  label: string;
  description: string;
  count?(c: EntityCounts): { value: number; attention?: boolean };
}

const DEFS: Record<SubTabId, SubTabDef> = {
  overview: { id: "overview", label: "Overview", description: "Properties, settings and live counts for this entity." },
  messages: { id: "messages", label: "Messages", description: "Active messages, read by peeking so nothing is locked or removed.", count: (c) => ({ value: c.active }) },
  "dead-letter": { id: "dead-letter", label: "Dead-letter", description: "Messages that could not be delivered or processed, with the reason each was dead-lettered.", count: (c) => ({ value: c.dead_letter, attention: true }) },
  "transfer-dlq": { id: "transfer-dlq", label: "Transfer DLQ", description: "Messages that failed to forward to another entity.", count: (c) => ({ value: c.transfer_dead_letter, attention: true }) },
  scheduled: { id: "scheduled", label: "Scheduled", description: "Messages waiting for their scheduled enqueue time.", count: (c) => ({ value: c.scheduled }) },
  deferred: { id: "deferred", label: "Deferred", description: "Messages a receiver set aside to process later." },
  sessions: { id: "sessions", label: "Sessions", description: "Sessions and their state, for session-enabled entities." },
  rules: { id: "rules", label: "Rules", description: "Filter rules that decide which messages reach this subscription." },
  listener: { id: "listener", label: "Listener", description: "New messages as they arrive." },
};

const QUEUE: SubTabId[] = ["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "listener"];
const SUBSCRIPTION: SubTabId[] = ["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "rules", "listener"];

export function subTabsFor(kind: EntityKind): SubTabDef[] {
  const ids = kind === "queue" ? QUEUE : kind === "subscription" ? SUBSCRIPTION : (["overview"] as SubTabId[]);
  return ids.map((id) => DEFS[id]);
}

/** The peek view a sub-tab shows; null for sub-tabs that are not message lists. */
export const VIEW_OF: Partial<Record<SubTabId, PeekView>> = {
  messages: "active",
  "dead-letter": "dead-letter",
  "transfer-dlq": "transfer-dead-letter",
  scheduled: "scheduled",
  deferred: "deferred",
};
export const viewOf = (sub: SubTabId): PeekView | null => VIEW_OF[sub] ?? null;

/** A view's name inside a sentence ("End of the dead-letter queue", "Can't read the scheduled messages"). */
export const VIEW_LABEL: Record<PeekView, string> = {
  active: "active messages",
  "dead-letter": "dead-letter queue",
  "transfer-dead-letter": "transfer dead-letter queue",
  scheduled: "scheduled messages",
  deferred: "deferred messages",
};

/** A subscription's tab id is `${topic}/subscriptions/${name}`. */
export const splitSubscription = splitSubscriptionId;

const iso = (d: string | null) => {
  const m = d?.match(/^PT(?:(\d+)M)?(?:(\d+)S)?$/);
  if (!m) return d;
  const min = Number(m[1] ?? 0), sec = Number(m[2] ?? 0);
  return min && !sec ? `${min} min` : min ? `${min} min ${sec} s` : `${sec} s`;
};

/** The sub-tab to show: a persisted one that does not apply to this kind of entity falls back to Overview. */
export function validSubTab(kind: EntityKind, subTab: SubTabId): SubTabId {
  return subTabsFor(kind).some((d) => d.id === subTab) ? subTab : "overview";
}

export interface EntityInfo {
  status: "loading" | "found" | "gone" | "unknown";
  counts: EntityCounts | null;
  facts: string[];
  parent?: string;
}

export function useEntityInfo(tab: EntityTab): EntityInfo {
  const { conn } = useShell();
  const entities = useEntities(conn.profileId);
  const sub = tab.kind === "subscription" ? splitSubscription(tab.id) : null;
  const subs = useQuery({
    queryKey: keys.subscriptions(conn.profileId, sub?.topic ?? ""),
    queryFn: () => commands.listSubscriptions(conn.profileId!, sub!.topic),
    enabled: sub !== null && conn.profileId !== null && !entities.data?.limited && !!entities.data?.topics.some((t) => t.name === sub.topic),
  });
  const listing = entities.data;
  // A listing that failed with nothing cached is a state to show, not a loading state to wait on forever.
  if (!listing) return { status: entities.isError ? "unknown" : "loading", counts: null, facts: [], parent: sub?.topic };
  if (listing.limited) return { status: "unknown", counts: null, facts: [], parent: sub?.topic };
  if (sub && !listing.topics.some((t) => t.name === sub.topic)) return { status: "gone", counts: null, facts: [], parent: sub.topic };
  if (sub && subs.isPending) return { status: "loading", counts: null, facts: [], parent: sub.topic };
  if (tab.kind === "queue") {
    const q = listing.queues.find((x) => x.name === tab.id);
    if (!q) return { status: "gone", counts: null, facts: [] };
    const facts = [q.max_delivery_count != null && `Max delivery ${q.max_delivery_count}`, q.lock_duration && `lock ${iso(q.lock_duration)}`, q.requires_session && "sessions", q.forward_to && `forwards to ${q.forward_to}`].filter(Boolean) as string[];
    return { status: "found", counts: q.counts, facts };
  }
  if (tab.kind === "topic") {
    const t = listing.topics.find((x) => x.name === tab.id);
    if (!t) return { status: "gone", counts: null, facts: [] };
    return { status: "found", counts: null, facts: t.subscription_count != null ? [`${t.subscription_count} subscriptions`] : [] };
  }
  const s = subs.data?.find((x) => x.name === sub!.name);
  if (!s) return { status: subs.error ? "unknown" : "gone", counts: null, facts: [], parent: sub!.topic };
  const facts = [s.max_delivery_count != null && `Max delivery ${s.max_delivery_count}`, s.requires_session && "sessions", s.forward_to && `forwards to ${s.forward_to}`].filter(Boolean) as string[];
  return { status: "found", counts: s.counts, facts, parent: sub!.topic };
}
