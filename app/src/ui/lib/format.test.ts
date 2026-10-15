import { describe, expect, it } from "vitest";
import { formatCount, formatDuration, formatNumber } from "./format";
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
