import { describe, expect, it } from "vitest";
import { displayName, entityIdOf, splitSubscriptionId } from "./entityId";

describe("entity ids", () => {
  it("split a subscription id on its last /subscriptions/, so topic names may contain slashes", () => {
    expect(splitSubscriptionId("shipment-events/subscriptions/tracking-sync")).toEqual({ topic: "shipment-events", name: "tracking-sync" });
    expect(splitSubscriptionId("orders/eu/subscriptions/audit")).toEqual({ topic: "orders/eu", name: "audit" });
    expect(splitSubscriptionId("a/Subscriptions/b/SUBSCRIPTIONS/c")).toEqual({ topic: "a/Subscriptions/b", name: "c" });
  });

  it("round-trip with entityIdOf", () => {
    const id = entityIdOf("subscription", "audit", "orders/eu");
    expect(id).toBe("orders/eu/subscriptions/audit");
    expect(splitSubscriptionId(id)).toEqual({ topic: "orders/eu", name: "audit" });
    expect(entityIdOf("queue", "orders/eu")).toBe("orders/eu");
  });

  it("name an entity as a person reads it: a subscription as name (topic)", () => {
    expect(displayName("invoice-generate")).toBe("invoice-generate");
    expect(displayName("shipment-events")).toBe("shipment-events");
    expect(displayName("shipment-events/subscriptions/tracking-sync")).toBe("tracking-sync (shipment-events)");
    expect(displayName("a/b/subscriptions/s")).toBe("s (a/b)");
  });
});
