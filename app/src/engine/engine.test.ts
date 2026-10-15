import { beforeEach, describe, expect, it } from "vitest";
import * as commands from "./commands";
import { EngineError, setTransport, toEngineError } from "./ipc";
import { createMockTransport } from "./mock/transport";
import { onActivity } from "./events";

describe("ipc", () => {
  it("maps Rust CommandError objects to EngineError", () => {
    const e = toEngineError({ kind: "forbidden", message: "access denied (needs Listen): no" });
    expect(e).toBeInstanceOf(EngineError);
    expect(e.kind).toBe("forbidden");
    expect(toEngineError(new Error("boom")).kind).toBe("unknown");
    expect(toEngineError("text").message).toBe("text");
  });
});

describe("commands over the mock transport", () => {
  let mock: ReturnType<typeof createMockTransport>;
  beforeEach(() => {
    mock = createMockTransport("default");
    setTransport(mock);
  });

  it("pass camelCase top-level arguments", async () => {
    await commands.connect("p-logistics");
    await commands.signIn(null, true);
    expect(mock.calls).toContainEqual({ cmd: "connect", args: { profileId: "p-logistics" } });
    expect(mock.calls).toContainEqual({ cmd: "sign_in", args: { tenant: null, deviceCode: true } });
  });

  it("lists the fixture namespace after connecting and emits activity", async () => {
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    await commands.connect("p-logistics");
    const l = await commands.listEntities();
    expect(l.queues.map((q) => q.name)).toContain("invoice-generate");
    expect(log).toContain("Connected to Logistics");
  });

  it("rejects listing before connecting with not_connected", async () => {
    await expect(commands.listEntities()).rejects.toMatchObject({ kind: "not_connected" });
  });

  it("never returns a stored connection string", async () => {
    const p = await commands.profileSave({
      name: "New",
      auth: { kind: "connection-string", connection_string: "Endpoint=sb://n.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=TOPSECRET" },
      production: false,
      transport: "amqp",
      pinned_entities: [],
    });
    expect(JSON.stringify(p)).not.toContain("TOPSECRET");
    expect(JSON.stringify(await commands.profilesList())).not.toContain("TOPSECRET");
  });

  it("serves scenarios", async () => {
    setTransport(createMockTransport("forbidden"));
    await commands.connect("p-logistics");
    const l = await commands.listEntities();
    expect(l.limited).toMatch(/can't list/);
    expect(l.known.length).toBeGreaterThan(0);
    setTransport(createMockTransport("empty"));
    expect(await commands.profilesList()).toEqual([]);
  });
});
