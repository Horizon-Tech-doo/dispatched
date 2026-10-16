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

  it("charges the compile budget per emitted node, so nested empty repeats cannot hang", () => {
    const started = performance.now();
    const r = compilePattern("(((((){100}){100}){100}){100}){100}");
    expect(performance.now() - started).toBeLessThan(50);
    // Either refused as too large or compiled to something that runs at once.
    if (!("error" in r)) expect(r.test("abc")).toBeDefined();
  });

  it("keeps a 161-character class fast on a long name", () => {
    // The one character the name is made of sits last in the class, so a member-by-member scan reads all 161.
    const chars = `${"b".repeat(160)}a`;
    const pattern = `[${chars}]{100}x`;
    const r = compilePattern(pattern);
    if ("error" in r) throw new Error(r.error);
    const name = "a".repeat(500);
    r.test(name); // warm up
    const started = performance.now();
    for (let i = 0; i < 5; i++) expect(r.test(name)).toBe(false);
    expect((performance.now() - started) / 5).toBeLessThan(8);
  });
});

/** The matcher is a subset of JavaScript's patterns: wherever it accepts one, it must agree with `new RegExp(p, "i")`. */
describe("agrees with RegExp on the names a pattern is tried against", () => {
  const patterns = [
    "[]", "[^]", "x{,3}", "x{3", "x{3}", "x{2,}", "x{1,2}y", "[\\d-z]", "[a-]", "[-a]", "[a\\-z]", "[\\b]", "(|a)b", "a*?b", "a+?", "\\B", "\\b", "\\bsync\\b",
    "orders", "^orders", "orders$", "^orders-(eu|us)$", "^(a|b)*c$", "^$", "^", "$", ".", "^.$", "a.c", "\\.", "^\\w+-\\d+$", "[A-Z]{2}", "[^a-c]+", "(?:ab)+", "^(?:ab){2}$",
    "a|b|", "|", "()", "(a)(b)", "^[\\w.-]+$", "\\s", "\\S+", "\\D", "[\\s\\d]", "-dlq$", "^invoice-", "x*", "^x*$", "é", "[à-ü]", "ß", "\\t", "[\\n]",
  ];
  const names = [
    "", "a", "b", "ab", "abc", "aab", "abab", "c", "bc", "xxx", "x", "x{,3}", "x{3", "xx", "xxy", "5", "-", "z", "\b", "Orders", "orders-eu", "ORDERS-US", "orders-ap", "x-orders", "invoice-generate", "label-print",
    "a.c", "abc.", "tracking-sync", "sync", "synchro", "AB", "AB12", "ab-12", "A1", "dlq", "orders-dlq", "orders-dlq-x", "é", "É", "ß", "ü", " ", "a b", "a\tb", "a\nb", "-a-", "a-z", "3-z",
  ];
  for (const p of patterns) {
    it(`/${p}/i`, () => {
      const mine = compilePattern(p);
      if ("error" in mine) throw new Error(`refused: ${mine.error}`);
      const re = new RegExp(p, "i");
      for (const n of names) expect(mine.test(n), `${JSON.stringify(n)}`).toBe(re.test(n));
    });
  }
});
