import { describe, expect, it } from "vitest";
import { describePeekError } from "./peekError";

describe("describePeekError", () => {
  it("names the Sessions tab for session-required errors", () => {
    const d = describePeekError("protocol", "peek-message returned 400: It is not possible for an entity that requires sessions to create a non-sessionful message receiver.", "active messages");
    expect(d.title).toBe("This entity requires sessions");
    expect(d.body).toMatch(/Open Sessions/);
    expect(d.fix).toBe("sessions");
  });
  it("says which right is missing and what to do for other kinds", () => {
    expect(describePeekError("forbidden", "access denied (needs Listen): x", "dead-letter queue")).toMatchObject({ title: "Can't read the dead-letter queue", fix: "retry" });
    expect(describePeekError("forbidden", "access denied (needs Listen): x", "dead-letter queue").body).toMatch(/Listen/);
    expect(describePeekError("not_signed_in", "x", "active messages").fix).toBe("edit");
    expect(describePeekError("network", "reset", "active messages").title).toBe("Couldn't reach the namespace");
  });
});
