import { describe, expect, it } from "vitest";
import { compilePattern, MAX_PATTERN } from "./safeRegex";

const test = (pattern: string, text: string) => {
  const r = compilePattern(pattern);
  if ("error" in r) throw new Error(r.error);
  return r.test(text);
};

describe("safe name patterns", () => {
  it("searches anywhere, ignoring case", () => {
    expect(test("orders", "eu-Orders-1")).toBe(true);
    expect(test("orders", "invoices")).toBe(false);
  });
  it("anchors", () => {
    expect(test("^invoice-", "invoice-generate")).toBe(true);
    expect(test("^invoice-", "x-invoice-generate")).toBe(false);
    expect(test("-dlq$", "orders-dlq")).toBe(true);
    expect(test("-dlq$", "orders-dlq-x")).toBe(false);
  });
  it("handles classes, escapes, groups, alternation and repeats", () => {
    expect(test("^[a-c]+\\d{2,3}$", "abc12")).toBe(true);
    expect(test("^[a-c]+\\d{2,3}$", "abc1")).toBe(false);
    expect(test("^[a-c]+\\d{2,3}$", "abc1234")).toBe(false);
    expect(test("^(orders|returns)-(eu|us)$", "returns-us")).toBe(true);
    expect(test("^(?:orders|returns)-(eu|us)$", "orders-ap")).toBe(false);
    expect(test("^a.c$", "abc")).toBe(true);
    expect(test("^a\\.c$", "abc")).toBe(false);
    expect(test("^[^x]+$", "abc")).toBe(true);
    expect(test("^[^x]+$", "abxc")).toBe(false);
    expect(test("colou?r", "color")).toBe(true);
    expect(test("\\bsync\\b", "tracking-sync")).toBe(true);
    expect(test("\\bsync\\b", "tracking-synchro")).toBe(false);
  });
  it("reports the first problem in plain words", () => {
    for (const bad of ["(", ")", "[a", "a{2,1}", "*a", "a**", "(?=a)", "(?<=a)b", "(a)\\1", "\\", "a{200}"]) {
      expect(compilePattern(bad), bad).toHaveProperty("error");
    }
  });
  it("caps the pattern and the compiled size", () => {
    expect(compilePattern("a".repeat(MAX_PATTERN + 1))).toHaveProperty("error");
    expect(compilePattern("((a{50}){50}){50}")).toHaveProperty("error");
  });
  it("runs in linear time on patterns that make a backtracking engine hang", () => {
    const started = Date.now();
    expect(test("^(a+)+$", `${"a".repeat(80)}!`)).toBe(false);
    expect(test("^(a|aa)+$", `${"a".repeat(80)}!`)).toBe(false);
    expect(test("(x+x+)+y", "x".repeat(80))).toBe(false);
    expect(Date.now() - started).toBeLessThan(500);
  });
});
