import { describe, expect, it } from "vitest";
import { describeListError, troubleOf } from "./listError";

const sas = { name: "Logistics", host: "sb-logistics-prod", auth: "connection-string" as const };
const entra = { name: "Logistics dev", host: "sb-logistics-dev", auth: "entra" as const };
const unknown = { name: "Logistics", host: "sb-logistics-prod", auth: "unknown" as const };

describe("describeListError", () => {
  it("explains a network failure and the next step", () => {
    expect(describeListError("network", sas)).toEqual({
      message: "The connection to sb-logistics-prod was interrupted. Check your network or VPN, then try again.",
      fix: "retry",
    });
    expect(describeListError("timeout", sas).message).toContain("interrupted");
  });
  it("explains missing rights by how the connection signs in, and sends the user to the connection", () => {
    expect(describeListError("forbidden", sas)).toEqual({
      message: "This key has no Manage right. Use a key whose policy includes Manage. You can also pin entities on the connection.",
      fix: "edit",
    });
    const e = describeListError("forbidden", entra);
    expect(e.message).toMatch(/^Your account needs the Azure Service Bus Data Owner role on this namespace\./);
    expect(e.message).toMatch(/You can also pin entities on the connection\.$/);
    expect(e.fix).toBe("edit");
    expect(describeListError("forbidden", unknown).message).toMatch(/^Your connection needs the Manage right: an Entra role or a key/);
  });
  it("sends an expired or revoked Entra sign-in to the connection sheet", () => {
    for (const kind of ["not_signed_in", "auth"]) {
      expect(describeListError(kind, entra)).toEqual({
        message: "Your sign-in for Logistics dev has expired. Edit the connection to sign in again.",
        fix: "edit",
      });
    }
  });
  it("sends rejected connection-string credentials to the connection sheet", () => {
    expect(describeListError("auth", sas)).toEqual({
      message: "sb-logistics-prod rejected the credentials for Logistics. Edit the connection to replace them.",
      fix: "edit",
    });
  });
  it("falls back to a general sentence", () => {
    expect(describeListError("unknown", sas)).toEqual({
      message: "Something went wrong listing sb-logistics-prod. Try again, or check the activity log for details.",
      fix: "retry",
    });
  });
});

describe("troubleOf", () => {
  it("tells a lost connection from an expired sign-in", () => {
    expect(troubleOf("network", false)).toBe("lost");
    expect(troubleOf("timeout", true)).toBe("lost");
    expect(troubleOf("not_signed_in", true)).toBe("sign-in");
    expect(troubleOf("auth", true)).toBe("sign-in");
    expect(troubleOf("auth", false)).toBe("credentials");
    expect(troubleOf("forbidden", false)).toBeNull();
    expect(troubleOf(undefined, false)).toBeNull();
  });
});
