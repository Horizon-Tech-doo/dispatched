import { describe, expect, it } from "vitest";
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

describe("job store", () => {
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
