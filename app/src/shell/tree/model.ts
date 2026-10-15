import type { EntityCounts, EntityListing, EntityRef, SubscriptionInfo } from "@/engine";
import type { TreeNode } from "@/ui";
import type { EntityKind } from "../state";

export type NodeKind = "namespace" | "group" | "queue" | "topic" | "subscription" | "known" | "note" | "loading" | "error";

export interface NsNode extends TreeNode {
  kind: NodeKind;
  entity?: { id: string; kind: EntityKind };
  counts?: EntityCounts | null;
  topic?: string;
  message?: string;
  /** Entities in a group, shown at its right edge. */
  count?: number;
}

export interface SubsState {
  data?: SubscriptionInfo[];
  error?: { message: string } | null;
  pending: boolean;
}

/** The tab id of an entity, which is also a valid `EntityPath`. */
export function entityIdOf(kind: EntityKind, name: string, topic?: string): string {
  return kind === "subscription" ? `${topic}/subscriptions/${name}` : name;
}

const knownId = (k: EntityRef) => (k.subscription ? entityIdOf("subscription", k.subscription, k.entity) : k.entity);

/** The visible rows of the namespace tree, flattened in display order. */
export function buildNodes(args: {
  host: string;
  listing: EntityListing;
  subs: Record<string, SubsState>;
  expanded: ReadonlySet<string>;
  filter: string;
}): NsNode[] {
  const { host, listing, subs, expanded } = args;
  const term = args.filter.trim().toLowerCase();
  const filtering = term.length > 0;
  const hit = (name: string) => !filtering || name.toLowerCase().includes(term);
  const out: NsNode[] = [];
  const push = (n: Omit<NsNode, "expandable" | "expanded"> & { expandable?: boolean; expanded?: boolean }) =>
    out.push({ expandable: false, expanded: false, ...n });

  push({ id: "ns", parentId: null, depth: 0, label: host, kind: "namespace" });

  if (listing.limited) {
    const known = listing.known.filter((k) => hit(knownId(k)));
    if (known.length) {
      push({ id: "group:pinned", parentId: "ns", depth: 1, label: "Pinned entities", kind: "group", count: known.length });
      for (const k of known) {
        const id = knownId(k);
        push({
          id: `k:${id}`, parentId: "group:pinned", depth: 2, label: k.subscription ?? k.entity,
          kind: "known", counts: null, entity: { id, kind: k.subscription ? "subscription" : "queue" },
          ...(k.subscription ? { topic: k.entity } : {}),
        });
      }
    }
    return out;
  }

  const queues = listing.queues.filter((q) => hit(q.name));
  if (queues.length) {
    const open = filtering || expanded.has("group:queues");
    push({ id: "group:queues", parentId: "ns", depth: 1, label: "Queues", kind: "group", count: queues.length, expandable: true, expanded: open });
    if (open) {
      for (const q of queues) {
        push({ id: `q:${q.name}`, parentId: "group:queues", depth: 2, label: q.name, kind: "queue", counts: q.counts, entity: { id: q.name, kind: "queue" } });
      }
    }
  }

  const topicRows = listing.topics
    .map((t) => {
      const s = subs[t.name];
      const matchingSubs = (s?.data ?? []).filter((x) => hit(x.name));
      return { t, s, matchingSubs, show: hit(t.name) || matchingSubs.length > 0 };
    })
    .filter((r) => r.show);

  if (topicRows.length) {
    const open = filtering || expanded.has("group:topics");
    push({ id: "group:topics", parentId: "ns", depth: 1, label: "Topics", kind: "group", count: topicRows.length, expandable: true, expanded: open });
    if (open) {
      for (const { t, s, matchingSubs } of topicRows) {
        const tid = `t:${t.name}`;
        const tOpen = (filtering && matchingSubs.length > 0) || expanded.has(tid);
        push({ id: tid, parentId: "group:topics", depth: 2, label: t.name, kind: "topic", expandable: true, expanded: tOpen, entity: { id: t.name, kind: "topic" } });
        if (!tOpen) continue;
        const loadedSubs = s?.data;
        if (!s || (s.pending && !loadedSubs)) {
          push({ id: `loading:${t.name}`, parentId: tid, depth: 3, label: "Loading subscriptions", kind: "loading", topic: t.name });
          continue;
        }
        // A topic that matched by name shows all its subscriptions; otherwise only the ones that match.
        const shown = filtering && !hit(t.name) ? matchingSubs : (loadedSubs ?? []);
        if (shown.length === 0 && !s.error) {
          push({ id: `note:${t.name}`, parentId: tid, depth: 3, label: "No subscriptions", kind: "note", topic: t.name });
        }
        for (const sub of shown) {
          push({
            id: `s:${t.name}/${sub.name}`, parentId: tid, depth: 3, label: sub.name, kind: "subscription",
            counts: sub.counts, topic: t.name, entity: { id: entityIdOf("subscription", sub.name, t.name), kind: "subscription" },
          });
        }
        // A failed refresh keeps what was already loaded and adds the error after it.
        if (s.error) {
          push({
            id: `error:${t.name}`, parentId: tid, depth: 3, kind: "error", topic: t.name, message: s.error.message,
            label: loadedSubs ? "Couldn't refresh subscriptions" : "Couldn't list subscriptions",
          });
        }
      }
    }
  }
  return out;
}
