import { describe, expect, it } from "vitest";
import { formatTimestamp } from "../time";
import { formatIsoDuration, formatProperty, labelOf } from "./labels";

describe("property labels and values", () => {
  it("labels known names and splits unknown ones", () => {
    expect(labelOf("LockDuration")).toBe("Lock duration");
    expect(labelOf("SomeNewServiceFlag")).toBe("Some new service flag");
  });
  it("formats ISO 8601 durations, with the maximum as never", () => {
    expect(formatIsoDuration("PT1M")).toBe("1 min");
    expect(formatIsoDuration("PT30S")).toBe("30 s");
    expect(formatIsoDuration("P14D")).toBe("14 days");
    expect(formatIsoDuration("P1DT2H")).toBe("1 day 2 h");
    expect(formatIsoDuration("PT1.5S")).toBe("1.5 s");
    expect(formatIsoDuration("P10675199DT2H48M5.4775807S")).toBe("Never");
    expect(formatIsoDuration("nonsense")).toBeNull();
  });
  it("formats flags, sizes and times", () => {
    expect(formatProperty("RequiresSession", "true")).toBe("Yes");
    expect(formatProperty("MaxSizeInMegabytes", "1024")).toBe("1,024 MB");
    expect(formatProperty("SizeInBytes", "81920")).toBe("80.0 KiB");
    // Local time: compare with the same formatter, so the test passes in every time zone.
    expect(formatProperty("CreatedAt", "2026-09-01T08:00:00Z")).toBe(formatTimestamp("2026-09-01T08:00:00Z"));
    expect(formatProperty("CreatedAt", "2026-09-01T08:00:00Z")).toMatch(/^2026-0[89]-\d\d \d\d:\d\d:00$/);
    expect(formatProperty("MessageCount", "1321")).toBe("1,321");
    expect(formatProperty("MaxSizeInMegabytes", "n/a")).toBe("n/a");
    expect(formatProperty("UserMetadata", "team=ops")).toBe("team=ops");
  });
});
