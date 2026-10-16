import { describe, expect, it } from "vitest";
import { compileRules, highlightOf } from "./highlight";

const c = (active: number, dead_letter: number) => ({ active, dead_letter, scheduled: 0, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false });

describe("highlight rules", () => {
  it("applies the first matching rule", () => {
    const rules = compileRules([
      { id: "1", color: "rose", match: { kind: "count", count: "dead-letter", atLeast: 100 } },
      { id: "2", color: "blue", match: { kind: "name", pattern: "^invoice-" } },
    ]);
    expect(highlightOf("invoice-generate", c(1, 300), rules)?.color).toBe("rose");
    expect(highlightOf("invoice-generate", c(1, 3), rules)?.color).toBe("blue");
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
    expect(highlightOf("Orders-EU", c(0, 0), compileRules([{ id: "1", color: "violet", match: { kind: "name", pattern: "orders" } }]))?.color).toBe("violet");
  });
  it("reads the scheduled and transfer dead-letter counts", () => {
    const counts = { ...c(0, 0), scheduled: 9, transfer_dead_letter: 4 };
    const rules = compileRules([
      { id: "1", color: "green", match: { kind: "count", count: "scheduled", atLeast: 5 } },
      { id: "2", color: "rose", match: { kind: "count", count: "transfer-dlq", atLeast: 4 } },
    ]);
    expect(highlightOf("x", counts, rules)?.color).toBe("green");
    expect(highlightOf("x", { ...counts, scheduled: 0 }, rules)?.color).toBe("rose");
    expect(highlightOf("x", { ...counts, scheduled: 0, transfer_dead_letter: 3 }, rules)).toBeNull();
  });
  it("does not hang on a catastrophic pattern", () => {
    const rules = compileRules([{ id: "1", color: "rose", match: { kind: "name", pattern: "^(a+)+$" } }]);
    const started = Date.now();
    expect(highlightOf(`${"a".repeat(60)}!`, null, rules)).toBeNull();
    expect(Date.now() - started).toBeLessThan(500);
  });
  it("says which rule matched, in words for a tooltip", () => {
    const rules = compileRules([
      { id: "1", color: "rose", match: { kind: "count", count: "dead-letter", atLeast: 200 } },
      { id: "2", color: "violet", match: { kind: "name", pattern: "^returns" } },
      { id: "3", color: "green", match: { kind: "count", count: "transfer-dlq", atLeast: 1 } },
    ]);
    expect(highlightOf("x", c(0, 250), rules)).toEqual({ color: "rose", description: "dead-letter at least 200" });
    expect(highlightOf("returns-intake", c(0, 1), rules)).toEqual({ color: "violet", description: "name matches ^returns" });
    expect(highlightOf("x", { ...c(0, 0), transfer_dead_letter: 2 }, rules)?.description).toBe("transfer DLQ at least 1");
  });
  it("compiles a hand-edited setting of twenty hostile rules at once", () => {
    const hostile = ["(((((){100}){100}){100}){100}){100}", "^(a+)+$", "(x+x+)+y", "[a-z]{100}[a-z]{100}", "((a{50}){50}){50}"];
    const rules = Array.from({ length: 20 }, (_, i) => ({ id: String(i), color: "rose" as const, match: { kind: "name" as const, pattern: hostile[i % hostile.length] } }));
    const started = performance.now();
    const compiled = compileRules(rules);
    for (let i = 0; i < 50; i++) highlightOf(`${"a".repeat(60)}!`, null, compiled);
    expect(performance.now() - started).toBeLessThan(500);
  });
  it("remembers a name's answer, so a scroll does not run the matcher again", () => {
    const [rule] = compileRules([{ id: "1", color: "rose", match: { kind: "name", pattern: "^a" } }]);
    expect(rule.test("abc", null)).toBe(true);
    expect(rule.test("abc", null)).toBe(true);
    expect(rule.test("xbc", null)).toBe(false);
  });
});
