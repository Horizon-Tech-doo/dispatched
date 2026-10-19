import { describe, expect, it } from "vitest";
import { formatIsoDuration, formatTtl } from "../duration";
import { formatTimestamp } from "../time";
import { formatProperty, groupProperties, isTimestamp, labelOf, propertiesJson } from "./labels";

describe("property labels and values", () => {
  it("labels known names and splits unknown ones", () => {
    expect(labelOf("LockDuration")).toBe("Lock duration");
    expect(labelOf("SomeNewServiceFlag")).toBe("Some new service flag");
  });
  it("formats ISO 8601 durations in the same style as time to live, with the maximum as never", () => {
    expect(formatIsoDuration("PT1M")).toBe("1 min");
    expect(formatIsoDuration("PT30S")).toBe("30 s");
    expect(formatIsoDuration("P14D")).toBe("14 d");
    expect(formatIsoDuration("P1DT2H")).toBe("1 d 2 h");
    expect(formatIsoDuration("PT1.5S")).toBe("1.5 s");
    expect(formatIsoDuration("PT0S")).toBe("0 s");
    expect(formatIsoDuration("P10675199DT2H48M5.4775807S")).toBe("Never");
    expect(formatIsoDuration("nonsense")).toBeNull();
    expect(formatIsoDuration("P")).toBeNull();
    expect(formatIsoDuration("P1DT")).toBeNull();
    // The detail pane's milliseconds read the same way.
    expect(formatTtl(14 * 86_400_000)).toBe(formatIsoDuration("P14D"));
    expect(formatTtl(922337203685477)).toBe(formatIsoDuration("P10675199DT2H48M5.4775807S"));
  });
  it("formats flags, sizes and times of known properties", () => {
    expect(formatProperty("RequiresSession", "true")).toBe("Yes");
    expect(formatProperty("EnablePartitioning", "false")).toBe("No");
    expect(formatProperty("MaxSizeInMegabytes", "1024")).toBe("1,024 MB");
    expect(formatProperty("MaxSizeInMegabytes", "n/a")).toBe("n/a");
    expect(formatProperty("SizeInBytes", "81920")).toBe("80.0 KiB");
    expect(formatProperty("MessageCount", "1321")).toBe("1,321");
    expect(formatProperty("LockDuration", "PT30S")).toBe("30 s");
    // Local time: compare with the same formatter, so the test passes in every time zone.
    expect(formatProperty("CreatedAt", "2026-09-01T08:00:00Z")).toBe(formatTimestamp("2026-09-01T08:00:00Z"));
    expect(formatProperty("CreatedAt", "2026-09-01T08:00:00Z")).toMatch(/^2026-0[89]-\d\d \d\d:\d\d:00$/);
    expect(isTimestamp("UpdatedAt")).toBe(true);
    expect(isTimestamp("Status")).toBe(false);
  });
  it("leaves values of unknown or free-text properties as the service wrote them", () => {
    expect(formatProperty("UserMetadata", "team=ops")).toBe("team=ops");
    expect(formatProperty("UserMetadata", "true")).toBe("true");
    expect(formatProperty("UserMetadata", "PT5M")).toBe("PT5M");
    expect(formatProperty("SomeNewServiceFlag", "false")).toBe("false");
    expect(formatProperty("SomeNewServiceFlag", "P14D")).toBe("P14D");
  });
});

describe("property groups", () => {
  const p = (name: string, value = "x") => ({ name, value });
  it("sorts known properties into Delivery, Features and Lifecycle, and the rest into Other, in the service's order", () => {
    const groups = groupProperties([p("CreatedAt"), p("LockDuration"), p("EnablePartitioning"), p("MaxDeliveryCount"), p("ABrandNewKey"), p("AutoDeleteOnIdle")], {});
    expect(groups.map((g) => g.title)).toEqual(["Delivery", "Features", "Lifecycle", "Other"]);
    expect(groups.map((g) => g.items.map((i) => i.name))).toEqual([["LockDuration", "MaxDeliveryCount"], ["EnablePartitioning"], ["CreatedAt", "AutoDeleteOnIdle"], ["ABrandNewKey"]]);
  });
  it("drops what the counts block already shows, and empty groups", () => {
    const props = [p("LockDuration"), p("SizeInBytes"), p("MaxSizeInMegabytes"), p("MessageCount")];
    expect(groupProperties(props, { size: true, maxSize: true, messageCount: true }).flatMap((g) => g.items.map((i) => i.name))).toEqual(["LockDuration"]);
    expect(groupProperties(props, {}).flatMap((g) => g.items.map((i) => i.name))).toEqual(["LockDuration", "MaxSizeInMegabytes", "SizeInBytes", "MessageCount"]);
    expect(groupProperties([p("LockDuration")], {}).map((g) => g.title)).toEqual(["Delivery"]);
  });
  it("keeps a maximum size the counts block could not show", () => {
    const props = [p("SizeInBytes"), p("MaxSizeInMegabytes", "n/a")];
    expect(groupProperties(props, { size: true }).flatMap((g) => g.items.map((i) => i.name))).toEqual(["MaxSizeInMegabytes"]);
  });
  it("can leave out the subscription count a topic's table already shows", () => {
    const props = [p("SubscriptionCount", "3"), p("EnablePartitioning")];
    expect(groupProperties(props, { subscriptionCount: true }).flatMap((g) => g.items.map((i) => i.name))).toEqual(["EnablePartitioning"]);
  });
  it("copies the raw names and values as JSON", () => {
    expect(JSON.parse(propertiesJson([p("LockDuration", "PT30S"), p("RequiresSession", "true")]))).toEqual({ LockDuration: "PT30S", RequiresSession: "true" });
  });
});
