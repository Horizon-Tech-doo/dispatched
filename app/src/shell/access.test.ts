import { describe, expect, it } from "vitest";
import { accessAdvice, rightsOf } from "./access";

describe("rightsOf", () => {
  it("reads the rights an engine message names", () => {
    expect(rightsOf("access denied (needs Listen): Unauthorized", "Manage")).toEqual(["Listen"]);
    expect(rightsOf("This identity can't list the namespace (needs Manage,EntityRead).", "Listen")).toEqual(["Manage"]);
    expect(rightsOf("forbidden: 401", "Manage")).toEqual(["Manage"]);
  });
});

describe("accessAdvice", () => {
  it("names the Entra role for Listen and Manage", () => {
    expect(accessAdvice(["Listen"], "entra")).toBe(
      "Your account needs the Azure Service Bus Data Receiver role on this namespace or entity. Owner and Contributor don't include reading messages. After it's assigned, peek again.",
    );
    expect(accessAdvice(["Manage"], "entra")).toMatch(/Data Owner role/);
    expect(accessAdvice(["Listen", "Manage"], "entra")).toMatch(/Data Owner role/);
  });
  it("names the key policy for a connection string", () => {
    expect(accessAdvice(["Listen"], "connection-string")).toBe("This key has no Listen right. Use a key whose policy includes Listen (or Manage).");
    expect(accessAdvice(["Manage"], "connection-string")).toBe("This key has no Manage right. Use a key whose policy includes Manage.");
  });
  it("has no em dashes", () => {
    for (const a of ["entra", "connection-string"] as const) for (const r of [["Listen"], ["Manage"]] as const) expect(accessAdvice(r, a)).not.toContain("—");
  });
});
