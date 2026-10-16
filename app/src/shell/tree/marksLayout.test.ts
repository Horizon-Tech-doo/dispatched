import { describe, expect, it } from "vitest";
import { marksTier, rowNeed } from "./marksLayout";

const w = { active: 5, scheduled: 1, "transfer-dlq": 1, "dead-letter": 5 };
const ALL = ["active", "scheduled", "transfer-dlq", "dead-letter"] as const;

describe("tree marks layout", () => {
  it("needs more room for a deeper row, more counts and a swatch slot", () => {
    expect(rowNeed(3, ALL, w, true)).toBeGreaterThan(rowNeed(2, ALL, w, true));
    expect(rowNeed(3, ALL, w, true)).toBeGreaterThan(rowNeed(3, ["active", "dead-letter"], w, true));
    expect(rowNeed(3, ALL, w, true)).toBeGreaterThan(rowNeed(3, ALL, w, false));
  });
  it("takes an unmeasured pane as wide", () => {
    expect(marksTier(0, 3, ALL, w, true)).toEqual({ optional: true, fixed: true });
  });
  it("lets scheduled and transfer DLQ give way first, then the fixed slots", () => {
    const full = rowNeed(3, ALL, w, true);
    const base = rowNeed(3, ["active", "dead-letter"], w, true);
    expect(marksTier(full, 3, ALL, w, true)).toEqual({ optional: true, fixed: true });
    expect(marksTier(full - 1, 3, ALL, w, true)).toEqual({ optional: false, fixed: true });
    expect(marksTier(base, 3, ALL, w, true).fixed).toBe(true);
    expect(marksTier(base - 1, 3, ALL, w, true)).toEqual({ optional: false, fixed: false });
  });
  it("shows a row of only active and dead-letter counts at once when it fits", () => {
    expect(marksTier(rowNeed(3, ["active", "dead-letter"], w, true), 3, ["active", "dead-letter"], w, true)).toEqual({ optional: true, fixed: true });
  });
});
