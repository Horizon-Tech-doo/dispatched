import { describe, expect, it } from "vitest";
import { formatShortTimestamp, formatTimestamp } from "./time";

describe("formatTimestamp", () => {
  it("is local wall-clock time, empty for null or garbage", () => {
    const d = new Date(2026, 9, 15, 9, 41, 12);
    expect(formatTimestamp(d.toISOString())).toBe("2026-10-15 09:41:12");
    expect(formatTimestamp(null)).toBe("");
    expect(formatTimestamp("not a time")).toBe("");
  });
});

describe("formatShortTimestamp", () => {
  it("is day, month and local time, with the year only when asked", () => {
    const iso = new Date(2026, 9, 15, 9, 41, 7).toISOString();
    expect(formatShortTimestamp(iso, false)).toBe("15 Oct 09:41:07");
    expect(formatShortTimestamp(iso, true)).toBe("15 Oct 2026 09:41:07");
    expect(formatShortTimestamp(new Date(2026, 0, 3, 23, 5, 0).toISOString(), false)).toBe("03 Jan 23:05:00");
    expect(formatShortTimestamp(null, false)).toBe("");
    expect(formatShortTimestamp("nope", true)).toBe("");
  });
});
