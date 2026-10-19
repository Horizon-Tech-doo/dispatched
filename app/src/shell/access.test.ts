import { describe, expect, it } from "vitest";
import { accessAdvice, authKindOf, rightsOf } from "./access";

describe("rightsOf", () => {
  it("reads the rights an engine message names", () => {
    expect(rightsOf("access denied (needs Listen): Unauthorized", "Manage")).toEqual(["Listen"]);
    expect(rightsOf("This identity can't list the namespace (needs Manage,EntityRead).", "Listen")).toEqual(["Manage"]);
    expect(rightsOf("forbidden: 401", "Manage")).toEqual(["Manage"]);
  });
  it("maps EntityRead alone to Manage, and ignores a right it has no advice for", () => {
    expect(rightsOf("access denied (needs EntityRead): x", "Listen")).toEqual(["Manage"]);
    expect(rightsOf("access denied (needs Send): x", "Listen")).toEqual(["Listen"]);
    expect(rightsOf("access denied (needs Send,Listen): x", "Manage")).toEqual(["Listen"]);
  });
});

describe("accessAdvice", () => {
  it("names the Entra role for Listen and Manage", () => {
    expect(accessAdvice(["Listen"], "entra")).toBe(
      "Your account needs the Azure Service Bus Data Receiver role on this namespace or entity. Owner and Contributor don't include reading messages. After it's assigned, try again.",
    );
    expect(accessAdvice(["Manage"], "entra")).toMatch(/Data Owner role/);
    expect(accessAdvice(["Listen", "Manage"], "entra")).toMatch(/Data Owner role/);
  });
  it("names the key policy for a connection string", () => {
    expect(accessAdvice(["Listen"], "connection-string")).toBe("This key has no Listen right. Use a key whose policy includes Listen (or Manage).");
    expect(accessAdvice(["Manage"], "connection-string")).toBe("This key has no Manage right. Use a key whose policy includes Manage.");
  });
  it("ends with a neutral 'try again', or the verb the surface gives", () => {
    expect(accessAdvice(["Listen"], "entra")).toMatch(/After it's assigned, try again\.$/);
    expect(accessAdvice(["Listen"], "entra", "peek again")).toMatch(/After it's assigned, peek again\.$/);
    expect(accessAdvice(["Manage"], "entra")).toMatch(/After it's assigned, try again\.$/);
  });
  it("stays neutral when it isn't known how the connection signs in", () => {
    expect(accessAdvice(["Listen"], "unknown")).toBe("Your connection needs the Listen right: an Entra role or a key whose policy includes Listen.");
    expect(accessAdvice(["Manage"], "unknown")).toBe("Your connection needs the Manage right: an Entra role or a key whose policy includes Manage.");
  });
  it("has no em dashes", () => {
    for (const a of ["entra", "connection-string", "unknown"] as const) for (const r of [["Listen"], ["Manage"]] as const) expect(accessAdvice(r, a)).not.toContain("—");
  });
});

describe("authKindOf", () => {
  const entra = { auth: { kind: "entra" } } as never;
  const sas = { auth: { kind: "connection-string" } } as never;
  it("reads the saved profile first", () => {
    expect(authKindOf(entra, "SAS key reader")).toBe("entra");
    expect(authKindOf(sas, undefined)).toBe("connection-string");
  });
  it("falls back to the connection's own description of its auth", () => {
    expect(authKindOf(undefined, "Entra ID ops@contoso.example")).toBe("entra");
    expect(authKindOf(undefined, "SAS key RootManageSharedAccessKey")).toBe("connection-string");
  });
  it("is unknown with neither", () => {
    expect(authKindOf(undefined, undefined)).toBe("unknown");
    expect(authKindOf(undefined, "something new")).toBe("unknown");
  });
});
