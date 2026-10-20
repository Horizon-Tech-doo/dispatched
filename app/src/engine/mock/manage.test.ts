import { describe, expect, it } from "vitest";
import { vi } from "vitest";
import { commands, onConfirm, onJob, setTransport, type ConfirmRequest, type JobEvent } from "@/engine";
import { createMockTransport, type Scenario } from "./transport";

type Finished = Extract<JobEvent, { type: "finished" }>;

/** A mock connected to the production profile that answers every confirmation with the typed name. */
async function connected(scenario: Scenario = "operate-prod") {
  const mock = createMockTransport(scenario);
  setTransport(mock);
  await commands.connect("p-logistics");
  const confirms: ConfirmRequest[] = [];
  const events: JobEvent[] = [];
  const offs = await Promise.all([
    onConfirm((c) => {
      confirms.push(c);
      void commands.confirmAnswer(c.id, { type: "typed", text: c.target });
    }),
    onJob((e) => events.push(e)),
  ]);
  const finished = async (id: string) => {
    await vi.waitFor(() => expect(events.find((e) => e.type === "finished" && e.id === id)).toBeDefined());
    return events.find((e): e is Finished => e.type === "finished" && e.id === id)!;
  };
  return { mock, confirms, events, finished, off: () => offs.forEach((f) => f()) };
}

describe("manage mock", () => {
  it("a created queue is listed, and its settings read back", async () => {
    const t = await connected();
    const { id } = await commands.opEntityCreate("p-logistics", "queue", "orders", { lock_duration: "45s" }, null);
    expect((await t.finished(id)).error).toBeNull();
    expect(t.confirms[0]).toMatchObject({ verb: "create-entity", target: "orders" });
    const listing = await commands.listEntities("p-logistics");
    expect(listing.queues.map((q) => q.name)).toContain("orders");
    const v = await commands.entitySettings("p-logistics", "queue", "orders");
    expect(v.settings.lock_duration).toBe("45s");
    t.off();
  });

  it("a deleted queue is gone from the listing", async () => {
    const t = await connected();
    const { id } = await commands.opEntityDelete("p-logistics", [{ kind: "queue", entity: "label-print" }]);
    expect((await t.finished(id)).report?.succeeded).toBe(1);
    expect((await commands.listEntities("p-logistics")).queues.map((q) => q.name)).not.toContain("label-print");
    t.off();
  });

  it("a rename lists the new name and asks for both", async () => {
    const t = await connected();
    const { id } = await commands.opEntityCopy("p-logistics", { kind: "topic", entity: "archive-events" }, "archive-2", true);
    await t.finished(id);
    expect(t.confirms.map((c) => c.target)).toEqual(["archive-events", "archive-2"]);
    const names = (await commands.listEntities("p-logistics")).topics.map((q) => q.name);
    expect(names).toContain("archive-2");
    expect(names).not.toContain("archive-events");
    // A queue that holds messages is not renamed: they would be lost.
    await expect(commands.opEntityCopy("p-logistics", { kind: "queue", entity: "label-print" }, "label-print-2", true)).rejects.toMatchObject({ kind: "conflict" });
    t.off();
  });

  it("a rule added to a subscription is listed", async () => {
    const t = await connected();
    const sub = "shipment-events/subscriptions/billing";
    const { id } = await commands.opRuleSave("p-logistics", sub, { name: "eu", filter: { kind: "sql", expression: "region = 'eu'" }, action: null }, false);
    await t.finished(id);
    expect((await commands.listRules("p-logistics", sub)).map((r) => r.name)).toContain("eu");
    t.off();
  });

  it("a listener emits received messages until it is cancelled, then ends without an error", async () => {
    const t = await connected("operate-ok");
    const { id } = await commands.opListen("p-logistics", {
      entity: "label-print", sub_queue: "active", mode: "watch", sessions: { type: "none" }, concurrency: 1, prefetch: 0,
    });
    await vi.waitFor(() => expect(t.events.filter((e) => e.type === "received" && e.id === id).length).toBeGreaterThanOrEqual(2), { timeout: 3000 });
    expect(await commands.jobCancel(id)).toBe(true);
    expect((await t.finished(id)).error).toBeNull();
    t.off();
  });

  it("policies list names and rights, never keys", async () => {
    await connected();
    const ps = await commands.entityPolicies("p-logistics", "queue", "invoice-generate");
    expect(ps.length).toBeGreaterThan(0);
    expect(JSON.stringify(ps)).not.toMatch(/key"/i);
  });
});
