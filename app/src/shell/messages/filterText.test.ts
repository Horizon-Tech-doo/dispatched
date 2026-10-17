import { describe, expect, it } from "vitest";
import { describeIssue, filterSummary } from "./filterText";

describe("describeIssue", () => {
  it("says it in plain words and keeps the engine's text without its prefix and backticks", () => {
    expect(describeIssue("property", "filter: expected a value at column 9", "sys.A = ")).toEqual({
      sentence: "This expression can't be read. Check it at column 9, then apply.",
      detail: "expected a value at column 9",
    });
    expect(describeIssue("property", "filter: unexpected `=` at column 14", "sys.Subject == 'x'")).toEqual({ sentence: "Use = to compare.", detail: "unexpected = at column 14" });
    expect(describeIssue("body_regex", "body regex: unterminated group", "(").sentence).toMatch(/regular expression/);
    expect(describeIssue("body_json_path", "body JSONPath `a`: expected `$` at position 1", "a")).toMatchObject({ detail: "body JSONPath a: expected $ at position 1" });
    expect(describeIssue("body_json_equals", "add a JSONPath to compare this value with", "x").sentence).toMatch(/Add a JSONPath/);
  });
});

describe("filterSummary", () => {
  it("lists the applied conditions on one line", () => {
    expect(filterSummary({})).toBe("");
    expect(filterSummary({ property: " attempt >= 2 ", body_contains: "held" })).toBe('attempt >= 2 · body contains "held"');
    expect(filterSummary({ body_json_path: "$.status", body_json_equals: "held", session_id: "RMA-1001" })).toBe("$.status = held · session RMA-1001");
    expect(filterSummary({ body_json_path: "$.status" })).toBe("$.status exists");
    expect(filterSummary({ property: "  ", body_contains: null })).toBe("");
  });
});
