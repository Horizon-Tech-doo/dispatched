import { describe, expect, it } from "vitest";
import { suggestedFileName } from "./exportName";

describe("suggested export names", () => {
  it("are stamped in the user's local time, as the save dialog's name is", () => {
    const at = new Date(2026, 9, 16, 9, 41); // 09:41 local, whatever the test machine's zone
    expect(suggestedFileName("events/subscriptions/audit", "dead-letter", "json", at)).toBe("events-subscriptions-audit-dead-letter-20261016-0941.json");
    expect(suggestedFileName("a b:c", "active", "NDJSON", at)).toBe("a-b-c-active-20261016-0941.ndjson");
  });
});
