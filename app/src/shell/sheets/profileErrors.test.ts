import { describe, expect, it } from "vitest";
import { checkAdvice, placeError, sentence } from "./profileErrors";

const invalid = (message: string) => ({ kind: "invalid", message });

describe("placeError", () => {
  it("puts a duplicate name under Name, without code quotes", () => {
    expect(placeError(invalid("a profile named `Orders` already exists"))).toEqual({ name: "A profile named Orders already exists." });
    expect(placeError(invalid("a profile needs a name; give it one that is not blank")).name).toBeDefined();
  });

  it("puts connection string problems under Connection string, including Endpoint errors", () => {
    expect(placeError(invalid("paste the namespace's connection string")).connectionString).toBeDefined();
    for (const m of ["Endpoint must start with sb://", "Endpoint is not a valid URL", "Endpoint has no host", "connection string has no `Endpoint=`"]) {
      expect(placeError(invalid(m)).connectionString, m).toBeDefined();
    }
  });

  it("places namespace, proxy and pin errors, and keeps the rest general", () => {
    expect(placeError(invalid("invalid namespace: expected a name like `myns`")).namespace).toBeDefined();
    expect(placeError(invalid("proxy must look like http://host:port")).proxy).toBeDefined();
    expect(placeError({ kind: "not_signed_in", message: "sign in with Entra ID first, then add the profile" }).signIn).toBeDefined();
    expect(placeError({ kind: "io", message: "disk full" })).toEqual({ general: "Disk full." });
  });
});

describe("sentence", () => {
  it("strips backticks, capitalises and ends with a full stop", () => {
    expect(sentence("`x` is bad")).toBe("X is bad.");
    expect(sentence("Already done.")).toBe("Already done.");
    expect(sentence("  ")).toBe("Something went wrong.");
  });
});

describe("checkAdvice for forbidden", () => {
  it("tells a connection string which right its key lacks", () => {
    expect(checkAdvice("Management", "forbidden", "connection-string")).toBe("This key has no Manage right. Use a key whose policy includes Manage.");
    expect(checkAdvice("Messaging", "forbidden", "connection-string")).toBe("This key has no Listen right. Use a key whose policy includes Listen (or Manage).");
  });
  it("tells an Entra account which role it lacks", () => {
    expect(checkAdvice("Management", "forbidden", "entra")).toMatch(/Data Owner role/);
    expect(checkAdvice("Messaging", "forbidden", "entra")).toMatch(/Data Receiver role/);
  });
  it("leaves other kinds as they were", () => {
    expect(checkAdvice("Management", "network", "entra")).toBe("Couldn't reach the namespace. Check the network, the proxy and the transport.");
  });
});
