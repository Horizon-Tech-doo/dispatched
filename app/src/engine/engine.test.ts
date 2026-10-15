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

  it("signs in to a tenant as its own sign-in, and to the home tenant as the account itself, as Rust does", async () => {
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    const first = await commands.signIn(null, false);
    expect(first).toEqual({ account: expect.objectContaining({ id: "oid-1", tenant_id: "tenant-1" }), tenant_sign_in: null });
    const guest = await commands.signIn("tenant-guest", false);
    expect(guest.account.id).toBe("oid-1");
    expect(guest.tenant_sign_in).toEqual({ tenant_id: "tenant-guest", user_principal: "ops@contoso.example", same_account: true });
    expect(log).toContain("Signed in to tenant tenant-guest as ops@contoso.example");
    // The current account's home tenant, in any case: the same account, current, no tenant sign-in.
    const home = await commands.signIn("TENANT-1", false);
    expect(home).toEqual({ account: expect.objectContaining({ id: "oid-1" }), tenant_sign_in: null });
    expect(await commands.accountCurrent()).toMatchObject({ id: "oid-1" });
    // The tenant lists with that sign-in.
    const tenants = await commands.listTenants();
    expect(tenants.find((t) => t.tenant_id === "tenant-guest")?.sign_in_upn).toBe("ops@contoso.example");
  });

  it("lists the fixture namespace after connecting and emits activity", async () => {
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    await commands.connect("p-logistics");
    const l = await commands.listEntities("p-logistics");
    expect(l.queues.map((q) => q.name)).toContain("invoice-generate");
    expect(log).toContain("Connected to Logistics");
    expect(log).toContain("Listed 4 queues and 4 topics");
    await commands.listSubscriptions("p-logistics", "stock-levels");
    expect(log).toContain("Listed 1 subscription of stock-levels");
  });

  it("rejects listing before connecting with not_connected", async () => {
    await expect(commands.listEntities("p-logistics")).rejects.toMatchObject({ kind: "not_connected" });
  });

  it("rejects listing a profile that is no longer the connected one", async () => {
    await commands.connect("p-logistics");
    await commands.connect("p-partner");
    await expect(commands.listEntities("p-logistics")).rejects.toMatchObject({ kind: "not_connected", message: "Choose a connection first." });
    await expect(commands.listSubscriptions("p-logistics", "shipment-events")).rejects.toMatchObject({ kind: "not_connected" });
    await expect(commands.listEntities("p-partner")).resolves.toMatchObject({ limited: null });
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
    const l = await commands.listEntities("p-logistics");
    expect(l.limited).toMatch(/can't list/);
    expect(l.known.length).toBeGreaterThan(0);
    setTransport(createMockTransport("empty"));
    expect(await commands.profilesList()).toEqual([]);
  });

  it("cancels a pending sign-in and logs it as cancelled, not failed", async () => {
    const log: { action: string; error: unknown }[] = [];
    await onActivity((e) => log.push({ action: e.action, error: e.error }));
    const pending = commands.signIn(null, false);
    const outcome = pending.then(() => null, (e: unknown) => e);
    await commands.signInCancel();
    expect(await outcome).toMatchObject({ kind: "cancelled", message: "Sign-in cancelled." });
    expect(await commands.accountCurrent()).toBeNull();
    expect(log).toEqual([{ action: "Sign-in cancelled", error: null }]);
  });

  it("validates new drafts like the engine does", async () => {
    await expect(
      commands.profileSave({ name: "A", auth: { kind: "connection-string" }, production: false, transport: "amqp", pinned_entities: [] }),
    ).rejects.toMatchObject({ kind: "invalid", message: expect.stringContaining("paste the namespace's connection string") });
    await expect(
      commands.profileSave({ name: "B", auth: { kind: "entra", namespace: "sb-b" }, production: false, transport: "amqp", pinned_entities: [] }),
    ).rejects.toMatchObject({ kind: "not_signed_in", message: expect.stringContaining("sign in with Entra ID first") });
    await commands.signIn(null, false);
    await expect(
      commands.profileSave({ name: "B", auth: { kind: "entra", namespace: "sb-b" }, production: false, transport: "amqp", pinned_entities: [] }),
    ).resolves.toMatchObject({ name: "B" });
  });

  it("removes the profile's shell state when it is deleted", async () => {
    mock = createMockTransport("default", 0, { "shell:p-logistics": { tabs: [], activeTab: null } });
    setTransport(mock);
    expect(await commands.settingsGet("shell:p-logistics")).not.toBeNull();
    await commands.profileDelete("p-logistics");
    expect(await commands.settingsGet("shell:p-logistics")).toBeNull();
  });
});
