import { useQuery } from "@tanstack/react-query";
import { commands, keys, useEntities, type EntityCounts } from "@/engine";
import { useShell } from "../context";
import type { EntityKind, OpenTab, SubTabId } from "../state";

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

/** A subscription's tab id is `${topic}/subscriptions/${name}`. */
export function splitSubscription(id: string): { topic: string; name: string } {
  const [topic, , name] = id.split("/");
  return { topic, name };
}

const iso = (d: string | null) => {
  const m = d?.match(/^PT(?:(\d+)M)?(?:(\d+)S)?$/);
  if (!m) return d;
  const min = Number(m[1] ?? 0), sec = Number(m[2] ?? 0);
  return min && !sec ? `${min} min` : min ? `${min} min ${sec} s` : `${sec} s`;
};

export interface EntityInfo {
  status: "loading" | "found" | "gone" | "unknown";
  counts: EntityCounts | null;
  facts: string[];
  parent?: string;
}

export function useEntityInfo(tab: OpenTab): EntityInfo {
  const { conn } = useShell();
  const entities = useEntities(conn.profileId);
  const sub = tab.kind === "subscription" ? splitSubscription(tab.id) : null;
  const subs = useQuery({
    queryKey: keys.subscriptions(conn.profileId, sub?.topic ?? ""),
    queryFn: () => commands.listSubscriptions(sub!.topic),
    enabled: sub !== null && conn.profileId !== null && !entities.data?.limited,
  });
  const listing = entities.data;
  if (!listing || (sub && subs.isPending && !listing.limited)) return { status: "loading", counts: null, facts: [] };
  if (listing.limited) return { status: "unknown", counts: null, facts: [], parent: sub?.topic };
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
