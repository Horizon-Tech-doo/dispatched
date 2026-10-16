import { describe, expect, it } from "vitest";
import { defaultColumns, formatTtl, propText, seqWidth } from "./columns";

describe("message columns", () => {
  it("default per view, with Session only for session entities", () => {
    expect(defaultColumns("active", true)).toContain("session");
    expect(defaultColumns("active", false)).not.toContain("session");
    expect(defaultColumns("transfer-dead-letter", false)).toEqual(defaultColumns("dead-letter", false));
    expect(defaultColumns("scheduled", false)).toContain("scheduled");
  });
  it("say a property's value, null only where asked", () => {
    expect(propText(undefined)).toBe("");
    expect(propText({ type: "null" })).toBe("");
    expect(propText({ type: "null" }, "null")).toBe("null");
    expect(propText({ type: "int", value: 7 })).toBe("7");
    expect(propText({ type: "bool", value: false })).toBe("false");
  });
  it("show time to live in its two largest units", () => {
    expect(formatTtl(null)).toBe("");
    expect(formatTtl(14 * 86_400_000)).toBe("14 d");
    expect(formatTtl(90 * 60_000)).toBe("1 h 30 min");
    expect(formatTtl(45_000)).toBe("45 s");
    expect(formatTtl(250)).toBe("250 ms");
    expect(formatTtl(922337203685477)).toBe("never");
  });
  it("size Seq to the longest sequence number", () => {
    expect(seqWidth(5)).toBe("56px");
    expect(seqWidth(17)).toBe("125px");
  });
});
