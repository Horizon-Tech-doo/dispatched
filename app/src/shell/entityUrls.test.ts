import { describe, expect, it } from "vitest";
import { deadLetterUrl, entityBase, entityUrl } from "./entityUrls";

describe("entity URLs", () => {
  it("builds sb:// URLs like the engine's entity paths", () => {
    expect(entityUrl("ns.servicebus.windows.net", "orders", "queue")).toBe("sb://ns.servicebus.windows.net/orders");
    expect(entityBase("orders/eu/subscriptions/audit", "subscription")).toBe("orders/eu/Subscriptions/audit");
    expect(deadLetterUrl("ns.servicebus.windows.net", "events/subscriptions/all", "subscription")).toBe("sb://ns.servicebus.windows.net/events/Subscriptions/all/$DeadLetterQueue");
    expect(deadLetterUrl("ns", "events", "topic")).toBeNull();
  });
});
