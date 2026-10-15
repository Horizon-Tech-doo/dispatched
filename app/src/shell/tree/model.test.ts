import { describe, expect, it } from "vitest";
import * as fx from "@/engine/mock/fixtures";
import type { EntityListing } from "@/engine";
import { buildNodes, treeRowOf, type SubsState } from "./model";

const listing: EntityListing = { queues: fx.queues, topics: fx.topics, limited: null, known: [] };
const loaded = (topic: string): SubsState => ({ data: fx.subscriptions[topic], error: null, pending: false });
const ids = (n: { id: string }[]) => n.map((x) => x.id);

describe("buildNodes", () => {
  it("builds namespace, groups and entities with depth and parents", () => {
    const n = buildNodes({ host: "sb-logistics-prod", listing, subs: {}, expanded: new Set(["group:queues", "group:topics"]), filter: "" });
    expect(n[0]).toMatchObject({ id: "ns", depth: 0, parentId: null, kind: "namespace" });
    expect(n.find((x) => x.id === "q:invoice-generate")).toMatchObject({ depth: 2, parentId: "group:queues", entity: { id: "invoice-generate", kind: "queue" } });
    expect(n.find((x) => x.id === "t:shipment-events")).toMatchObject({ expandable: true, expanded: false });
    expect(ids(n)).not.toContain("s:shipment-events/tracking-sync");
  });

  it("model_handles_empty_topics_and_unknown_counts", () => {
    const n = buildNodes({
      host: "h", listing,
      subs: { "archive-events": loaded("archive-events"), "stock-levels": loaded("stock-levels") },
      expanded: new Set(["group:topics", "t:archive-events", "t:stock-levels"]), filter: "",
    });
    expect(n.find((x) => x.id === "note:archive-events")).toMatchObject({ kind: "note", label: "No subscriptions", depth: 3 });
    expect(n.find((x) => x.id === "s:stock-levels/replenish")).toMatchObject({ counts: null, entity: { id: "stock-levels/subscriptions/replenish", kind: "subscription" } });
  });

  it("shows loading and error children for expanded topics", () => {
    const n = buildNodes({
      host: "h", listing,
      subs: { "carrier-updates": { pending: true }, "shipment-events": { pending: false, error: { message: "access denied" } } },
      expanded: new Set(["group:topics", "t:carrier-updates", "t:shipment-events"]), filter: "",
    });
    expect(n.find((x) => x.id === "loading:carrier-updates")?.kind).toBe("loading");
    expect(n.find((x) => x.id === "error:shipment-events")).toMatchObject({ kind: "error", message: "access denied" });
  });

  it("filter_is_case_insensitive_and_keeps_ancestors", () => {
    const n = buildNodes({
      host: "h", listing, subs: { "shipment-events": loaded("shipment-events") },
      expanded: new Set(), filter: "TRACK",
    });
    expect(ids(n)).toEqual(["ns", "group:topics", "t:shipment-events", "s:shipment-events/tracking-sync"]);
    expect(n.find((x) => x.id === "t:shipment-events")?.expanded).toBe(true);
  });

  it("limited_listing_shows_known_entities_and_the_reason", () => {
    const limited: EntityListing = {
      queues: [], topics: [], limited: "This identity can't list the namespace (needs Manage). Showing pinned entities.",
      known: [{ entity: "orders", subscription: null }, { entity: "shipment-events", subscription: "tracking-sync" }],
    };
    const n = buildNodes({ host: "h", listing: limited, subs: {}, expanded: new Set(), filter: "" });
    expect(ids(n)).toEqual(["ns", "group:pinned", "k:orders", "k:shipment-events/subscriptions/tracking-sync"]);
    expect(n[2]).toMatchObject({ entity: { id: "orders", kind: "queue" }, counts: null });
    expect(n[3]).toMatchObject({ entity: { id: "shipment-events/subscriptions/tracking-sync", kind: "subscription" } });
  });

  it("pinned_subscriptions_are_labelled_by_name_with_the_topic_alongside", () => {
    const limited: EntityListing = { queues: [], topics: [], limited: "x", known: [{ entity: "shipment-events", subscription: "tracking-sync" }] };
    const n = buildNodes({ host: "h", listing: limited, subs: {}, expanded: new Set(), filter: "" });
    expect(n[2]).toMatchObject({ label: "tracking-sync", topic: "shipment-events" });
  });

  it("keeps loaded subscriptions when a refresh fails and adds the error as a secondary row", () => {
    const n = buildNodes({
      host: "h", listing,
      subs: { "shipment-events": { data: fx.subscriptions["shipment-events"], error: { message: "timeout" }, pending: false } },
      expanded: new Set(["group:topics", "t:shipment-events"]), filter: "",
    });
    expect(ids(n.filter((x) => x.parentId === "t:shipment-events"))).toEqual([
      "s:shipment-events/billing", "s:shipment-events/tracking-sync", "s:shipment-events/warehouse-notify", "error:shipment-events",
    ]);
    expect(n.find((x) => x.id === "error:shipment-events")).toMatchObject({ message: "timeout", label: "Couldn't refresh subscriptions" });
  });

  it("filter_shows_every_subscription_of_an_expanded_topic_whose_name_matches", () => {
    const n = buildNodes({
      host: "h", listing, subs: { "shipment-events": loaded("shipment-events") },
      expanded: new Set(["group:topics", "t:shipment-events"]), filter: "shipment",
    });
    expect(ids(n.filter((x) => x.parentId === "t:shipment-events"))).toEqual([
      "s:shipment-events/billing", "s:shipment-events/tracking-sync", "s:shipment-events/warehouse-notify",
    ]);
  });

  it("pinned_entities_sit_under_their_group_like_queues_do", () => {
    const limited: EntityListing = { queues: [], topics: [], limited: "x", known: [{ entity: "orders", subscription: null }] };
    const n = buildNodes({ host: "h", listing: limited, subs: {}, expanded: new Set(), filter: "" });
    expect(n.find((x) => x.id === "group:pinned")).toMatchObject({ depth: 1, parentId: "ns", expandable: false });
    expect(n.find((x) => x.id === "k:orders")).toMatchObject({ depth: 2, parentId: "group:pinned" });
  });

  it("a collapsed group hides its entities", () => {
    const n = buildNodes({ host: "h", listing, subs: {}, expanded: new Set(), filter: "" });
    expect(ids(n)).toEqual(["ns", "group:queues", "group:topics"]);
    expect(n.find((x) => x.id === "group:queues")).toMatchObject({ expandable: true, expanded: false });
  });

  it("group rows carry the number of entities they list", () => {
    const n = buildNodes({ host: "h", listing, subs: {}, expanded: new Set(), filter: "" });
    expect(n.find((x) => x.id === "group:queues")?.count).toBe(4);
    expect(n.find((x) => x.id === "group:topics")?.count).toBe(4);
    const f = buildNodes({ host: "h", listing, subs: {}, expanded: new Set(), filter: "label" });
    expect(f.find((x) => x.id === "group:queues")?.count).toBe(1);
  });
});

describe("treeRowOf", () => {
  it("names the row of an entity and the rows that must be open above it", () => {
    expect(treeRowOf({ id: "invoice-generate", kind: "queue" }, false)).toEqual({ rowId: "q:invoice-generate", ancestors: ["group:queues"] });
    expect(treeRowOf({ id: "shipment-events", kind: "topic" }, false)).toEqual({ rowId: "t:shipment-events", ancestors: ["group:topics"] });
    expect(treeRowOf({ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription" }, false)).toEqual({
      rowId: "s:shipment-events/tracking-sync",
      ancestors: ["group:topics", "t:shipment-events"],
    });
    expect(treeRowOf({ id: "orders", kind: "queue" }, true)).toEqual({ rowId: "k:orders", ancestors: [] });
  });

  it("finds a subscription whose topic name contains a slash", () => {
    expect(treeRowOf({ id: "orders/eu/subscriptions/audit", kind: "subscription" }, false)).toEqual({
      rowId: "s:orders/eu/audit",
      ancestors: ["group:topics", "t:orders/eu"],
    });
  });
});
