import { describe, expect, it } from "vitest";
import { describeConnectError } from "./connectError";

const profile = { name: "Logistics", host: "sb-logistics-prod.servicebus.windows.net" };
const err = (kind: string, message: string) => ({ kind, message });

describe("describeConnectError", () => {
  it("explains a missing keychain secret", () => {
    const d = describeConnectError(err("invalid", "profile `Logistics` has no stored connection string; remove and add it again"), profile);
    expect(d.message).toBe("The connection string for Logistics isn't in the system keychain. Edit the connection and paste it again.");
    expect(d.retryable).toBe(false);
  });

  it("maps sign-in, credentials, network and permission failures", () => {
    expect(describeConnectError(err("not_signed_in", "x"), profile)).toEqual({
      message: "Your Entra sign-in for Logistics has expired. Edit the connection to sign in again.",
      retryable: false,
    });
    expect(describeConnectError(err("auth", "x"), profile)).toEqual({
      message: "sb-logistics-prod rejected the credentials. Edit the connection to replace them.",
      retryable: false,
    });
    expect(describeConnectError(err("network", "reset"), profile)).toEqual({
      message: "Couldn't reach sb-logistics-prod. Check your network or proxy, then try again.",
      retryable: true,
    });
    expect(describeConnectError(err("timeout", "slow"), profile).retryable).toBe(true);
    expect(describeConnectError(err("forbidden", "x"), profile).message).toBe("This identity isn't allowed to connect to sb-logistics-prod.");
  });

  it("falls back to the message, capitalised and ended with a full stop", () => {
    expect(describeConnectError(err("weird", "something broke"), profile)).toEqual({ message: "Something broke.", retryable: true });
    expect(describeConnectError(err("weird", "Already done."), profile).message).toBe("Already done.");
    expect(describeConnectError(err("invalid", "bad host name"), profile)).toEqual({ message: "Bad host name.", retryable: false });
    expect(describeConnectError(null, profile).message).toBe("Something went wrong.");
  });
});
