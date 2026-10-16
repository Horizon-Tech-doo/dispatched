import { describe, expect, it } from "vitest";
import { checkAdvice } from "./profileErrors";

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
