import { describe, expect, it, vi } from "vitest";
import { ACTIVITY_CAP, createActivityStore, createJobStore } from "./activity";
import { finished, itemEvent, progress, report, started } from "./jobEvents";

const entry = (i: number) => ({ at: "2026-10-15T09:41:12Z", action: `Step ${i}`, duration_ms: 12, error: null });

describe("activity store", () => {
  it("keeps the newest entries up to the cap", () => {
    const s = createActivityStore();
    for (let i = 0; i < ACTIVITY_CAP + 25; i++) s.add(entry(i));
    expect(s.all()).toHaveLength(ACTIVITY_CAP);
    expect(s.all()[0].action).toBe("Step 25");
    expect(s.all().at(-1)!.action).toBe(`Step ${ACTIVITY_CAP + 24}`);
  });
});

describe("activity ids", () => {
  it("gives each entry an increasing id that survives the cap", () => {
    const s = createActivityStore();
    for (let i = 0; i < ACTIVITY_CAP + 10; i++) s.add(entry(i));
    const ids = s.all().map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual([...ids].sort((a, b) => a - b));
    expect(ids[0]).toBeGreaterThan(0);
    s.clear();
    s.add(entry(0));
    expect(s.all()[0].id).toBeGreaterThan(ids.at(-1)!);
  });
});

describe("startEventCapture", () => {
  it("unsubscribes the listener that succeeded when the other one fails", async () => {
    vi.resetModules();
    const off = vi.fn();
    vi.doMock("@/engine", () => ({
      onActivity: vi.fn(async () => off),
      onJob: vi.fn(async () => { throw new Error("listen failed"); }),
      onConfirm: vi.fn(async () => off),
      onActivityRecord: vi.fn(async () => off),
    }));
    try {
      const { startEventCapture } = await import("./activity");
      await expect(startEventCapture()).rejects.toThrow("listen failed");
      expect(off).toHaveBeenCalledTimes(3);
    } finally {
      vi.doUnmock("@/engine");
    }
  });
});

describe("job store", () => {
  it("replaces a job that starts again with the same id", () => {
    const s = createJobStore();
    s.apply(started("j1", "First", 10));
    s.apply(progress("j1", 5, 10));
    s.apply(started("j1", "Second", 20));
    expect(s.all()).toHaveLength(1);
    expect(s.all()[0]).toMatchObject({ title: "Second", done: 0, total: 20, finished: false });
  });

  it("tracks start, progress and finish", () => {
    const s = createJobStore();
    s.apply(started("j1", "Export dead letters", 3417));
    s.apply(progress("j1", 1920, 3417));
    expect(s.all()[0]).toMatchObject({ done: 1920, total: 3417, finished: false });
    s.apply(finished("j1", { error: null }));
    expect(s.all()[0].finished).toBe(true);
    s.apply(progress("unknown", 1, null));
    expect(s.all()).toHaveLength(1);
  });
});

describe("job store with operate jobs", () => {
  it("folds started, three items and finished into one job with succeeded 3", () => {
    const s = createJobStore();
    s.apply(started("j1", "Delete 3 from orders", null, null, "delete"));
    for (const n of ["1", "2", "3"]) s.apply(itemEvent("j1", { sequence_number: n, outcome: "succeeded" }));
    expect(s.all()[0]).toMatchObject({ kind: "delete", finished: false, tally: { succeeded: 3, failed: 0 } });
    s.apply(finished("j1", { report: report({ items: 3, succeeded: 3, summary_line: "Deleted 3 messages from orders" }), summary: "Deleted 3 messages from orders" }));
    expect(s.all()).toHaveLength(1);
    expect(s.all()[0]).toMatchObject({ finished: true, report: { succeeded: 3 }, tally: { succeeded: 3 } });
  });

  it("keeps the failed count of a partly failed job", () => {
    const s = createJobStore();
    s.apply(started("j1", "Delete 3 from orders", null, null, "delete"));
    s.apply(itemEvent("j1", { outcome: "succeeded" }));
    s.apply(itemEvent("j1", { outcome: "failed", reason: "lock lost" }));
    s.apply(finished("j1", { report: report({ outcome: "partly-failed", items: 3, succeeded: 2, failed: 1, incomplete: 1 }) }));
    expect(s.all()[0]).toMatchObject({ report: { outcome: "partly-failed", failed: 1 }, tally: { failed: 1 } });
  });

  it("holds a job's waiting confirmation until it is answered or the job ends", () => {
    const s = createJobStore();
    const ask = { id: "c1", job_id: "j1", verb: "delete", target: "orders", summary: "Delete 1 message from orders on Logistics", item_count: "1", reversible: false, production: false, step: 1 };
    // A question can arrive before its job's `started`.
    s.confirm(ask);
    s.apply(started("j1", "Delete 1 from orders", null, null, "delete"));
    expect(s.all()[0].confirm).toEqual(ask);
    s.answered("j1");
    expect(s.all()[0].confirm).toBeNull();
    s.confirm({ ...ask, id: "c2", step: 2 });
    s.apply(finished("j1", { error: { kind: "cancelled", message: "Cancelled." } }));
    expect(s.all()[0].confirm).toBeNull();
  });
});
