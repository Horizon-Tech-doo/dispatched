import { describe, expect, it } from "vitest";
import { defaultColumns, formatTtl, mergeColumns, optionalColumns, parseColumns, propText, seqWidth } from "./columns";

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
    expect(seqWidth(17)).toBe("129px");
  });
  it("offer only the columns a view can fill", () => {
    expect(optionalColumns("active", false)).not.toContain("reason");
    expect(optionalColumns("active", false)).not.toContain("session");
    expect(optionalColumns("active", true)).toContain("session");
    expect(optionalColumns("dead-letter", false)).toEqual(expect.arrayContaining(["reason", "description", "source"]));
    expect(optionalColumns("scheduled", false)).toContain("scheduled");
    expect(optionalColumns("deferred", false)).not.toContain("scheduled");
    for (const v of ["active", "dead-letter", "transfer-dead-letter", "scheduled", "deferred"] as const)
      for (const s of [true, false]) for (const id of defaultColumns(v, s)) if (id !== "seq") expect(optionalColumns(v, s)).toContain(id);
  });
  it("read a saved column list, falling back on anything unknown or unoffered", () => {
    expect(parseColumns(["seq", "subject"], "active", false)).toEqual(["seq", "subject"]);
    expect(parseColumns(["seq", "toString"], "active", false)).toEqual(defaultColumns("active", false));
    expect(parseColumns(["seq", "constructor"], "active", false)).toEqual(defaultColumns("active", false));
    expect(parseColumns([], "active", false)).toEqual(defaultColumns("active", false));
    expect(parseColumns("seq", "active", false)).toEqual(defaultColumns("active", false));
    // A column saved for the group that this entity cannot fill is not shown blank.
    expect(parseColumns(["seq", "session", "prop:region"], "dead-letter", false)).toEqual(["seq", "prop:region"]);
  });
  it("keep the columns an entity cannot show when it changes the saved list", () => {
    const saved = ["seq", "session", "subject", "prop:region"];
    // A non-session entity shows [seq, subject, prop:region]; hiding Subject must not drop Session for session entities.
    expect(mergeColumns(saved, ["seq", "prop:region"], "active", false)).toEqual(["seq", "session", "prop:region"]);
    // Adding a column to what is shown leaves the hidden ones where they were.
    expect(mergeColumns(saved, ["seq", "subject", "size", "prop:region"], "active", false)).toEqual(["seq", "session", "subject", "size", "prop:region"]);
    // A session entity can show everything: nothing is hidden, so the shown list is the saved list.
    expect(mergeColumns(saved, ["seq", "subject"], "active", true)).toEqual(["seq", "subject"]);
    // Nothing saved yet: the shown list is saved as it is.
    expect(mergeColumns(undefined, ["seq", "subject"], "active", false)).toEqual(["seq", "subject"]);
    // A hidden column at the front stays at the front.
    expect(mergeColumns(["session", "seq"], ["seq", "size"], "active", false)).toEqual(["session", "seq", "size"]);
  });
});
