import { describe, expect, it } from "vitest";
import { describePeekError } from "./peekError";

describe("describePeekError", () => {
  it("names the Sessions tab for session-required errors", () => {
    const d = describePeekError("protocol", "peek-message returned 400: It is not possible for an entity that requires sessions to create a non-sessionful message receiver.", "active messages", "entra");
    expect(d.title).toBe("This entity requires sessions");
    expect(d.body).toMatch(/Open Sessions/);
    expect(d.fix).toBe("sessions");
  });
  it("says which right is missing and what to do for other kinds", () => {
    expect(describePeekError("forbidden", "access denied (needs Listen): x", "dead-letter queue", "entra")).toMatchObject({ title: "Can't read the dead-letter queue", fix: "edit" });
    expect(describePeekError("not_signed_in", "x", "active messages", "entra").fix).toBe("edit");
    expect(describePeekError("network", "reset", "active messages", "entra").title).toBe("Couldn't reach the namespace");
  });
});

describe("describePeekError for forbidden", () => {
  const raw = "access denied (needs Listen): Unauthorized access. 'Listen' claim(s) are required to perform this operation.";
  it("sends a connection string to a key with Listen, and edits the connection", () => {
    expect(describePeekError("forbidden", raw, "active messages", "connection-string")).toEqual({
      title: "Can't read the active messages",
      body: "This key has no Listen right. Use a key whose policy includes Listen (or Manage).",
      fix: "edit",
    });
  });
  it("sends an Entra account to the Data Receiver role", () => {
    const d = describePeekError("forbidden", raw, "active messages", "entra");
    expect(d.body).toMatch(/^Your account needs the Azure Service Bus Data Receiver role on this namespace or entity\. Owner and Contributor don't include reading messages\./);
    expect(d.fix).toBe("edit");
  });
});
