import { describe, expect, it, vi } from "vitest";
import { ACTIVITY_CAP, createActivityStore, createJobStore } from "./activity";

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
    }));
    try {
      const { startEventCapture } = await import("./activity");
      await expect(startEventCapture()).rejects.toThrow("listen failed");
      expect(off).toHaveBeenCalledTimes(1);
    } finally {
      vi.doUnmock("@/engine");
    }
  });
});

describe("job store", () => {
  it("replaces a job that starts again with the same id", () => {
    const s = createJobStore();
    s.apply({ type: "started", id: "j1", title: "First", total: 10 });
    s.apply({ type: "progress", id: "j1", done: 5, total: 10 });
    s.apply({ type: "started", id: "j1", title: "Second", total: 20 });
    expect(s.all()).toHaveLength(1);
    expect(s.all()[0]).toMatchObject({ title: "Second", done: 0, total: 20, finished: false });
  });

  it("tracks start, progress and finish", () => {
    const s = createJobStore();
    s.apply({ type: "started", id: "j1", title: "Export dead letters", total: 3417 });
    s.apply({ type: "progress", id: "j1", done: 1920, total: 3417 });
    expect(s.all()[0]).toMatchObject({ done: 1920, total: 3417, finished: false });
    s.apply({ type: "finished", id: "j1", error: null });
    expect(s.all()[0].finished).toBe(true);
    s.apply({ type: "progress", id: "unknown", done: 1, total: null });
    expect(s.all()).toHaveLength(1);
  });
});
