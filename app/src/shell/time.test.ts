import { describe, expect, it } from "vitest";
import { formatBytes } from "./format";
import { formatTimestamp } from "./time";

describe("formatBytes", () => {
  it("uses binary units with one decimal above a KiB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(812)).toBe("812 B");
    expect(formatBytes(1229)).toBe("1.2 KiB");
    expect(formatBytes(4 * 1024 * 1024)).toBe("4.0 MiB");
    expect(formatBytes(1.1 * 1024 ** 3)).toBe("1.1 GiB");
  });
});

describe("formatTimestamp", () => {
  it("is local wall-clock time, empty for null or garbage", () => {
    const d = new Date(2026, 9, 15, 9, 41, 12);
    expect(formatTimestamp(d.toISOString())).toBe("2026-10-15 09:41:12");
    expect(formatTimestamp(null)).toBe("");
    expect(formatTimestamp("not a time")).toBe("");
  });
});
