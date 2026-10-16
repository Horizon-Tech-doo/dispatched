import { describe, expect, it } from "vitest";
import { formatBytes, formatCount, formatDuration, formatNumber } from "./format";
import { shortcutLabel } from "./platform";

describe("format", () => {
  it("formats counts like the CLI", () => {
    expect(formatNumber(34621)).toBe("34,621");
    expect(formatCount(null)).toBe("?");
    expect(formatCount(undefined)).toBe("?");
    expect(formatCount(3417)).toBe("3,417");
    expect(formatCount(120, { approximate: true })).toBe("~120");
    expect(formatCount(10000, { approximate: true, truncated: true })).toBe("~10,000+");
  });

  it("formats byte sizes in binary units with one decimal above a KiB", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(812)).toBe("812 B");
    expect(formatBytes(1229)).toBe("1.2 KiB");
    expect(formatBytes(4 * 1024 * 1024)).toBe("4.0 MiB");
    expect(formatBytes(1.1 * 1024 ** 3)).toBe("1.1 GiB");
  });

  it("formats durations", () => {
    expect(formatDuration(312)).toBe("312 ms");
    expect(formatDuration(1400)).toBe("1.4 s");
    expect(formatDuration(123000)).toBe("2 min 3 s");
  });

  it("labels shortcuts for the platform", () => {
    expect(shortcutLabel("mod+k", true)).toBe("⌘K");
    expect(shortcutLabel("mod+k", false)).toBe("Ctrl+K");
    expect(shortcutLabel("mod+shift+p", true)).toBe("⇧⌘P");
    expect(shortcutLabel("f5", false)).toBe("F5");
  });
});
