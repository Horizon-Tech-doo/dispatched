import { describe, expect, it } from "vitest";
import { createQueryClient, keys, setTransport } from "@/engine";
import { createMockTransport } from "@/engine/mock/transport";
import { recordStore } from "./activity";
import { createExternalChanges } from "./externalChanges";

const finishedBy = (origin: "cli" | "gui" | "mcp", entity: string, profile = "Logistics") => ({
  op_id: `op-${origin}`, origin, profile, entity, action: "delete", kind: "mutating", phase: "finished" as const, outcome: "succeeded" as const,
  count: 1, succeeded: 1, failed: 0, skipped: 0, summary: `Deleted 1 message from ${entity}`, error: null, note: null,
});

/** A query client holding the views of two queues, all fresh. */
function views() {
  const qc = createQueryClient();
  const peek = keys.peek("p-logistics", "label-print", "active", { kind: "start" }, {}, null);
  const other = keys.peek("p-logistics", "customs-docs", "active", { kind: "start" }, {}, null);
  const details = keys.details("p-logistics", "queue", "label-print");
  for (const k of [peek, other, details, keys.dashboard("p-logistics")]) qc.setQueryData(k, { pages: [], pageParams: [] });
  const stale = (k: readonly unknown[]) => qc.getQueryState(k)?.isInvalidated || qc.getQueryState(k) === undefined || qc.getQueryState(k)?.data === undefined;
  return { qc, peek, other, details, stale };
}

describe("external changes", () => {
  it("cli entries invalidate the affected entity", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    recordStore.clear();
    const v = views();
    const x = createExternalChanges({ qc: v.qc, profile: () => ({ id: "p-logistics", name: "Logistics" }) });
    await x.poll(); // starts following from the newest entry
    mock.external(finishedBy("cli", "label-print"));
    await x.poll();
    expect(v.stale(v.peek)).toBe(true);
    expect(v.stale(v.details)).toBe(true);
    expect(v.stale(keys.dashboard("p-logistics"))).toBe(true);
    expect(v.stale(v.other)).toBe(false);
    // The Activity tab shows the CLI entry.
    expect(recordStore.all().some((r) => r.origin === "cli" && r.entity === "label-print" && r.summary === "Deleted 1 message from label-print")).toBe(true);
  });

  it("gui entries do not trigger refetch, nor another profile's", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    const v = views();
    const x = createExternalChanges({ qc: v.qc, profile: () => ({ id: "p-logistics", name: "Logistics" }) });
    await x.poll();
    mock.external(finishedBy("gui", "label-print"));
    mock.external(finishedBy("mcp", "label-print", "Logistics dev"));
    mock.external({ ...finishedBy("cli", "label-print"), phase: "started", outcome: "pending" });
    await x.poll();
    expect(v.stale(v.peek)).toBe(false);
    expect(v.stale(v.details)).toBe(false);
  });

  it("polls only while the window has focus", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    const v = views();
    let focused = false;
    const x = createExternalChanges({ qc: v.qc, profile: () => ({ id: "p-logistics", name: "Logistics" }), focused: () => focused, intervalMs: 5 });
    const stop = x.start();
    await new Promise((r) => setTimeout(r, 30));
    expect(mock.calls.filter((c) => c.cmd === "activity_since")).toHaveLength(0);
    focused = true;
    await new Promise((r) => setTimeout(r, 30));
    expect(mock.calls.filter((c) => c.cmd === "activity_since").length).toBeGreaterThan(0);
    stop();
  });
});
