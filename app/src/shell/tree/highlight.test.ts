import { describe, expect, it } from "vitest";
import { compileRules, highlightOf } from "./highlight";

const c = (active: number, dead_letter: number) => ({ active, dead_letter, scheduled: 0, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false });

describe("highlight rules", () => {
  it("applies the first matching rule", () => {
    const rules = compileRules([
      { id: "1", color: "rose", match: { kind: "count", count: "dead-letter", atLeast: 100 } },
      { id: "2", color: "blue", match: { kind: "name", pattern: "^invoice-" } },
    ]);
    expect(highlightOf("invoice-generate", c(1, 300), rules)).toBe("rose");
    expect(highlightOf("invoice-generate", c(1, 3), rules)).toBe("blue");
    expect(highlightOf("label-print", c(1, 3), rules)).toBeNull();
  });
  it("skips bad patterns and unknown counts", () => {
    const rules = compileRules([
      { id: "1", color: "green", match: { kind: "name", pattern: "(" } },
      { id: "2", color: "slate", match: { kind: "count", count: "active", atLeast: 1 } },
    ]);
    expect(rules).toHaveLength(1);
    expect(highlightOf("x", null, rules)).toBeNull();
  });
  it("matches names case-insensitively", () => {
    expect(highlightOf("Orders-EU", c(0, 0), compileRules([{ id: "1", color: "violet", match: { kind: "name", pattern: "orders" } }]))).toBe("violet");
  });
  it("reads the scheduled and transfer dead-letter counts", () => {
    const counts = { ...c(0, 0), scheduled: 9, transfer_dead_letter: 4 };
    const rules = compileRules([
      { id: "1", color: "green", match: { kind: "count", count: "scheduled", atLeast: 5 } },
      { id: "2", color: "rose", match: { kind: "count", count: "transfer-dlq", atLeast: 4 } },
    ]);
    expect(highlightOf("x", counts, rules)).toBe("green");
    expect(highlightOf("x", { ...counts, scheduled: 0 }, rules)).toBe("rose");
    expect(highlightOf("x", { ...counts, scheduled: 0, transfer_dead_letter: 3 }, rules)).toBeNull();
  });
  it("does not hang on a catastrophic pattern", () => {
    const rules = compileRules([{ id: "1", color: "rose", match: { kind: "name", pattern: "^(a+)+$" } }]);
    const started = Date.now();
    expect(highlightOf(`${"a".repeat(60)}!`, null, rules)).toBeNull();
    expect(Date.now() - started).toBeLessThan(500);
  });
});
