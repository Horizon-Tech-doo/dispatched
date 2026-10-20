import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { keys } from "@/engine";
import { entityIdOfPath, invalidateEntity } from "./invalidate";

describe("invalidation after a job", () => {
  it("manage_job_invalidates_the_names_it_touched", async () => {
    const qc = new QueryClient();
    const id = "events/subscriptions/billing";
    for (const k of [keys.subscriptions("p", "events"), keys.rules("p", id), keys.settings("p", id), keys.policies("p", "orders")]) qc.setQueryData(k, []);
    invalidateEntity(qc, "p", entityIdOfPath("events/Subscriptions/billing"));
    invalidateEntity(qc, "p", "orders");
    for (const k of [keys.subscriptions("p", "events"), keys.rules("p", id), keys.settings("p", id), keys.policies("p", "orders")])
      expect(qc.getQueryState(k)?.isInvalidated, JSON.stringify(k)).toBe(true);
  });

  it("core paths become tab ids", () => {
    expect(entityIdOfPath("events/Subscriptions/billing")).toBe("events/subscriptions/billing");
    expect(entityIdOfPath("orders")).toBe("orders");
  });
});
