import { beforeEach, describe, expect, it, vi } from "vitest";
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

  it("never moves a profile to another account when it is edited, as Rust does", async () => {
    await commands.signIn(null, false); // ops@contoso.example (oid-1)
    const p = await commands.profileSave({ name: "Ops ns", auth: { kind: "entra", namespace: "sb-ops" }, production: false, transport: "amqp", pinned_entities: [] });
    expect(p.auth).toMatchObject({ sign_in_id: "oid-1" });
    await commands.signIn(null, false); // another account (oid-2) is current now
    const edited = await commands.profileSave({ id: p.id, name: "Ops ns", auth: { kind: "entra", namespace: "sb-ops" }, production: false, transport: "amqp", pinned_entities: [] });
    expect(edited.auth).toMatchObject({ sign_in_id: "oid-1" });
    expect(edited.bound_sign_in).toMatchObject({ id: "oid-1", same_account: false });
  });

  it("lists the fixture namespace after connecting and emits activity", async () => {
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    await commands.connect("p-logistics");
    const l = await commands.listEntities("p-logistics");
    expect(l.queues.map((q) => q.name)).toContain("invoice-generate");
    expect(log).toContain("Connected to Logistics");
    expect(log).toContain("Listed 4 queues and 4 topics");
    // Subscriptions are read on every expand and polled by a topic's Overview: only a failure is logged.
    expect(await commands.listSubscriptions("p-logistics", "stock-levels")).toHaveLength(1);
    expect(log.some((a) => /subscription/i.test(a))).toBe(false);
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

describe("M1c commands on the mock", () => {
  it("peeks summaries, reads one message, and refuses another profile", async () => {
    const mock = createMockTransport("default", 0);
    setTransport(mock);
    await commands.connect("p-logistics");
    const page = await commands.peekPage("p-logistics", { entity: "invoice-generate", view: "dead-letter", from: { kind: "start" }, max: 10, filter: {} });
    expect(page.messages).toHaveLength(10);
    expect(page.scan_limit).toBe(5_000);
    expect(page.messages[0]).not.toHaveProperty("body");
    expect(page.messages[0].dead_letter_reason).toBeTruthy();
    const d = await commands.messageDetail("p-logistics", "invoice-generate", "dead-letter", page.messages[0].sequence_number);
    expect(d?.body.kind).toBeDefined();
    expect(await commands.messageDetail("p-logistics", "invoice-generate", "dead-letter", 1)).toBeNull();
    await expect(commands.peekPage("p-other", { entity: "x", view: "active", from: { kind: "start" }, max: 1, filter: {} })).rejects.toMatchObject({ kind: "not_connected" });
  });

  it("logs peek pages, details and the dashboard only when they fail, as Rust does", async () => {
    const mock = createMockTransport("default", 0);
    setTransport(mock);
    await commands.connect("p-logistics");
    const log: { action: string; error: unknown }[] = [];
    await onActivity((e) => log.push({ action: e.action, error: e.error }));
    await commands.peekPage("p-logistics", { entity: "invoice-generate", view: "active", from: { kind: "start" }, max: 50, filter: {} });
    await commands.messageDetail("p-logistics", "invoice-generate", "active", 20_000);
    await commands.entityDetails("p-logistics", "queue", "invoice-generate");
    await commands.dashboard("p-logistics");
    expect(log).toEqual([]);
    await expect(commands.entityDetails("p-logistics", "queue", "no-such-queue")).rejects.toMatchObject({ kind: "not_found" });
    expect(log).toEqual([{ action: "Reading the details of no-such-queue failed", error: expect.objectContaining({ kind: "not_found" }) }]);
    // A filter that does not compile is refused before any work, and not logged.
    await expect(commands.peekPage("p-logistics", { entity: "invoice-generate", view: "active", from: { kind: "start" }, max: 5, filter: { property: "a ==" } })).rejects.toMatchObject({ kind: "invalid" });
    expect(log).toHaveLength(1);
  });

  it("reads the newest messages from the end, oldest first", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.connect("p-logistics");
    const tail = await commands.peekPage("p-logistics", { entity: "invoice-generate", view: "dead-letter", from: { kind: "end" }, max: 5, filter: {} });
    expect(tail.messages.map((m) => m.sequence_number)).toEqual([41_032, 41_033, 41_034, 41_035, 41_036]);
    expect(tail).toMatchObject({ next_sequence: null, exhausted: true, scan_limit: 100_000 });
  });

  it("checks filters like the engine", async () => {
    setTransport(createMockTransport("default", 0));
    expect(await commands.checkFilter({ property: "sys.Subject = 'x'" })).toEqual([]);
    expect(await commands.checkFilter({ property: "   ", body_regex: "" })).toEqual([]);
    const issues = await commands.checkFilter({ property: "sys.Subject == 'x'", body_regex: "(" });
    expect(issues.map((i) => i.field)).toEqual(["property", "body_regex"]);
    expect(issues[0].message).toMatch(/column \d+/);
    expect((await commands.checkFilter({ body_json_equals: "1" })).map((i) => i.field)).toEqual(["body_json_equals"]);
    expect((await commands.checkFilter({ body_json_path: "shipmentId" })).map((i) => i.field)).toEqual(["body_json_path"]);
  });

  it("filters peeks by property, body and session", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.connect("p-logistics");
    const peek = (filter: object) =>
      commands.peekPage("p-logistics", { entity: "returns-intake", view: "active", from: { kind: "start" }, max: 500, filter });
    expect((await peek({})).messages).toHaveLength(48);
    const one = await peek({ session_id: "RMA-1003" });
    expect(one.messages.length).toBeGreaterThan(0);
    expect(one.messages.every((m) => m.session_id === "RMA-1003")).toBe(true);
    const subj = await peek({ property: "sys.Subject = 'invoice.ready'" });
    expect(subj.messages.every((m) => m.subject === "invoice.ready")).toBe(true);
  });

  it("uses regex and expected-value text as typed, spaces included, as the engine does", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.connect("p-logistics");
    const peek = (filter: object) =>
      commands.peekPage("p-logistics", { entity: "invoice-generate", view: "active", from: { kind: "start" }, max: 500, filter });
    expect((await peek({ body_regex: "^Label" })).messages.length).toBeGreaterThan(0);
    expect((await peek({ body_regex: " ^Label" })).messages).toHaveLength(0);
    expect((await peek({ body_regex: "   " })).messages).toHaveLength(500);
    expect((await peek({ body_json_path: "$.status", body_json_equals: "held" })).messages.length).toBeGreaterThan(0);
    expect((await peek({ body_json_path: "$.status", body_json_equals: " held" })).messages).toHaveLength(0);
  });

  it("exports to the path the save dialog picked, once, with progress and a summary", async () => {
    const mock = createMockTransport("default", 0);
    setTransport(mock);
    await commands.connect("p-logistics");
    const events: unknown[] = [];
    const log: { action: string; error: unknown }[] = [];
    await mock.listen("job", (e) => events.push(e));
    await onActivity((e) => log.push({ action: e.action, error: e.error }));
    const picked = await commands.pickExportPath("invoice-generate", "dead-letter", "json");
    expect(picked?.name).toMatch(/^invoice-generate-dead-letter-\d{8}-\d{4}\.json$/);
    expect(picked?.pick).toMatch(/^pick-/);
    const request = { entity: "invoice-generate", view: "dead-letter" as const, scope: { kind: "all" as const, filter: {} }, format: "json" as const, pick: picked!.pick, total_hint: 37 };
    // A request refused before the job starts keeps its pick.
    await expect(commands.exportStart("p-logistics", { ...request, scope: { kind: "all", filter: { body_regex: "(" } } })).rejects.toMatchObject({ kind: "invalid" });
    const { id } = await commands.exportStart("p-logistics", request);
    expect(events[0]).toMatchObject({ type: "started", id, title: "Export invoice-generate (dead-letter queue)", total: 37, file_name: picked!.name });
    await new Promise((r) => setTimeout(r, 700));
    expect(events.at(-1)).toMatchObject({ type: "finished", id, error: null, summary: `Exported 37 messages to ${picked!.name}`, output: `${picked!.dir}/${picked!.name}`, written: 37, missing: 0, file_name: picked!.name });
    expect(events.some((e) => (e as { type: string }).type === "progress")).toBe(true);
    expect(events.find((e) => (e as { type: string }).type === "progress")).toMatchObject({ id, total: 37, scanned: expect.any(Number) });
    expect(log).toEqual([{ action: `Exported 37 messages to ${picked!.name}`, error: null }]);
    await expect(commands.jobReveal(id)).resolves.toBeNull();
    // A pick authorises one export.
    await expect(commands.exportStart("p-logistics", request)).rejects.toMatchObject({ kind: "invalid", message: "Choose where to save the export." });
    await expect(commands.jobReveal("job-none")).rejects.toMatchObject({ kind: "not_found" });
  });

  it("cancels an export: no file, logged as its message without an error", async () => {
    const slow = createMockTransport("export-slow", 0);
    setTransport(slow);
    await commands.connect("p-logistics");
    const done: unknown[] = [];
    const log: { action: string; error: unknown }[] = [];
    await slow.listen("job", (e) => done.push(e));
    await onActivity((e) => log.push({ action: e.action, error: e.error }));
    const picked = await commands.pickExportPath("invoice-generate", "dead-letter", "ndjson");
    const job = await commands.exportStart("p-logistics", { entity: "invoice-generate", view: "dead-letter", scope: { kind: "all", filter: {} }, format: "ndjson", pick: picked!.pick });
    expect(await commands.jobCancel(job.id)).toBe(true);
    await new Promise((r) => setTimeout(r, 50));
    expect(done.at(-1)).toMatchObject({ type: "finished", error: { kind: "cancelled", message: "Export cancelled." }, summary: null, output: null, written: null, missing: null, file_name: null });
    expect(log).toEqual([{ action: "Export cancelled", error: null }]);
    expect(await commands.jobCancel(job.id)).toBe(false);
    await expect(commands.jobReveal(job.id)).rejects.toMatchObject({ kind: "not_found" });
  });

  it("stops running exports when the connection closes, as stopped (not cancelled)", async () => {
    const slow = createMockTransport("export-slow", 0);
    setTransport(slow);
    await commands.connect("p-logistics");
    const done: unknown[] = [];
    await slow.listen("job", (e) => done.push(e));
    const picked = await commands.pickExportPath("invoice-generate", "dead-letter", "json");
    await commands.exportStart("p-logistics", { entity: "invoice-generate", view: "dead-letter", scope: { kind: "selected", sequences: [41_000, 41_001, 1] }, format: "json", pick: picked!.pick });
    expect(done[0]).toMatchObject({ type: "started", total: 3 });
    await commands.disconnect();
    await new Promise((r) => setTimeout(r, 20));
    expect(done.at(-1)).toMatchObject({ type: "finished", error: { kind: "stopped", message: "Export stopped: the connection closed." } });
  });

  it("lists sessions, rules, details and the dashboard", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.connect("p-logistics");
    const s = await commands.listSessions("p-logistics", "returns-intake");
    expect(s.sessions.map((x) => x.id)).toContain("RMA-1001");
    expect(s.sessions.find((x) => x.id === "RMA-0999")).toMatchObject({ messages: 0, in_broker: true });
    expect(s.broker).toEqual({ state: "listed", capped: false });
    expect(s.sessions.reduce((n, x) => n + (x.messages ?? 0), 0)).toBe(48);
    const rules = await commands.listRules("p-logistics", "shipment-events/subscriptions/tracking-sync");
    expect(rules.map((r) => r.filter.kind)).toEqual(["sql", "sql", "correlation"]);
    const other = await commands.listRules("p-logistics", "carrier-updates/subscriptions/eta-recalc");
    expect(other.map((r) => r.filter)).toContainEqual({ kind: "unknown" });
    await expect(commands.listRules("p-logistics", "invoice-generate")).rejects.toMatchObject({ kind: "invalid" });
    const d = await commands.entityDetails("p-logistics", "queue", "invoice-generate");
    expect(d.counts?.dead_letter).toBe(37);
    expect(d.properties.some((p) => p.name === "LockDuration")).toBe(true);
    const sub = await commands.entityDetails("p-logistics", "subscription", "shipment-events/subscriptions/tracking-sync");
    expect(sub).toMatchObject({ kind: "subscription", name: "tracking-sync", topic: "shipment-events" });
    const dash = await commands.dashboard("p-logistics");
    expect(dash.rows.some((r) => r.id === "shipment-events/subscriptions/tracking-sync")).toBe(true);
    expect(dash.limited).toBeNull();
  });

  it("shows a limited dashboard of known entities when listing is not allowed", async () => {
    setTransport(createMockTransport("forbidden", 0));
    await commands.connect("p-logistics");
    const dash = await commands.dashboard("p-logistics");
    expect(dash.limited).toMatch(/can't list the namespace/);
    expect(dash.rows.map((r) => r.id)).toEqual(["orders", "shipment-events/subscriptions/tracking-sync"]);
    expect(dash.rows.every((r) => r.counts === null)).toBe(true);
  });

  it("never returns a connection string from the copy command", async () => {
    setTransport(createMockTransport("default", 0));
    const log: { action: string; error: unknown }[] = [];
    await onActivity((e) => log.push({ action: e.action, error: e.error }));
    expect(await commands.copyEntityConnectionString("p-logistics", "orders")).toBeNull();
    await expect(commands.copyEntityConnectionString("p-logistics-dev", "orders")).rejects.toMatchObject({ kind: "invalid" });
    await expect(commands.copyEntityConnectionString("p-logistics", "or;ders")).rejects.toMatchObject({ kind: "invalid" });
    expect(log.map((l) => l.action)).toEqual([
      "Copied the connection string for orders",
      "Copying the connection string for orders failed",
      "Copying the connection string for or;ders failed",
    ]);
  });

  it("tests a draft without saving it", async () => {
    setTransport(createMockTransport("default", 0));
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    const r = await commands.profileTestDraft({
      name: "Draft", auth: { kind: "connection-string", connection_string: "Endpoint=sb://draft.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=TOPSECRET" },
      production: false, transport: "amqp", pinned_entities: [],
    });
    expect(r.host).toBe("draft.servicebus.windows.net");
    expect(JSON.stringify(r)).not.toContain("TOPSECRET");
    expect(log).toEqual(["Tested Draft (unsaved): management ok, messaging ok"]);
    expect(JSON.stringify(log)).not.toContain("TOPSECRET");
    await expect(commands.profileTestDraft({ name: "E", auth: { kind: "entra", namespace: "sb-e" }, production: false, transport: "amqp", pinned_entities: [] }))
      .rejects.toMatchObject({ kind: "not_signed_in" });
    expect(log.at(-1)).toBe("Testing the connection failed");
  });

  it("lists sign-ins and signs one out, closing the connection that used it", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.signIn(null, false); // oid-1 current
    await commands.signIn("tenant-guest", false); // the same person's guest tenant sign-in
    const list = await commands.signInsList();
    expect(list.map((s) => s.id)).toEqual(["oid-1", "oid-2", "oid-guest"]);
    expect(list[0]).toMatchObject({ current: true, signed_in: true, same_account: true, profiles: ["Logistics dev"], display_name: "Ops" });
    expect(list[1]).toMatchObject({ current: false, signed_in: false, same_account: false, profiles: [] });
    expect(list[2]).toMatchObject({ current: false, signed_in: true, same_account: true, tenant_id: "tenant-guest" });
    const log: string[] = [];
    await onActivity((e) => log.push(e.action));
    expect(await commands.signOut("oid-guest")).toEqual({ closed_profile_id: null });
    await commands.connect("p-logistics-dev");
    expect(await commands.signOut("oid-1")).toEqual({ closed_profile_id: "p-logistics-dev" });
    expect(log).toEqual(["Signed out ops@contoso.example", "Connected to Logistics dev", "Signed out ops@contoso.example and closed Logistics dev"]);
    expect(await commands.accountCurrent()).toBeNull();
    await expect(commands.listEntities("p-logistics-dev")).rejects.toMatchObject({ kind: "not_connected" });
    await expect(commands.signOut("oid-1")).rejects.toMatchObject({ kind: "not_found" });
  });

  it("opens the sign-in page only while a sign-in waits", async () => {
    setTransport(createMockTransport("default", 0));
    await expect(commands.openSignInPage()).rejects.toMatchObject({ kind: "not_found" });
    const pending = commands.signIn(null, false);
    await new Promise((r) => setTimeout(r, 10));
    await expect(commands.openSignInPage()).resolves.toBeNull();
    await pending;
    await expect(commands.openSignInPage()).rejects.toMatchObject({ kind: "not_found" });
  });
});

describe("suggestedFileName", () => {
  it("names the file as the save dialog does, in UTC", async () => {
    const { suggestedFileName } = await import("./exportName");
    const at = new Date("2026-10-16T09:41:59Z");
    expect(suggestedFileName("invoice-generate", "dead-letter", "json", at)).toBe("invoice-generate-dead-letter-20261016-0941.json");
    expect(suggestedFileName("shipment-events/subscriptions/tracking-sync", "active", "NDJSON", at)).toBe("shipment-events-subscriptions-tracking-sync-active-20261016-0941.ndjson");
  });
});

describe("export job events on the mock", () => {
  const run = async (scenario: "default" | "export-fails" | "export-missing", scope: Parameters<typeof commands.exportStart>[1]["scope"], wait = 700) => {
    const mock = createMockTransport(scenario, 0);
    setTransport(mock);
    await commands.connect("p-logistics");
    const events: { type: string; [k: string]: unknown }[] = [];
    await mock.listen("job", (e) => events.push(e as never));
    const picked = await commands.pickExportPath("invoice-generate", "dead-letter", "json");
    await commands.exportStart("p-logistics", { entity: "invoice-generate", view: "dead-letter", scope, format: "json", pick: picked!.pick });
    await new Promise((r) => setTimeout(r, wait));
    return events;
  };

  it("a filtered export without a total examines more than it writes", async () => {
    const events = await run("default", { kind: "all", filter: { body_contains: "SHP" } });
    expect(events[0]).toMatchObject({ type: "started", total: null });
    const last = events.filter((e) => e.type === "progress").at(-1)!;
    expect(last.scanned as number).toBeGreaterThanOrEqual(last.done as number);
    expect(events.at(-1)).toMatchObject({ type: "finished", written: last.done, missing: 0 });
  });

  it("reports the selected messages that were no longer there", async () => {
    const events = await run("export-missing", { kind: "selected", sequences: [41_000, 41_001, 41_002, 41_003] });
    expect(events.at(-1)).toMatchObject({ type: "finished", error: null, written: 2, missing: 2, summary: expect.stringContaining("2 selected messages were no longer there") });
  });

  it("a failing export ends with the engine's error and no counts", async () => {
    const events = await run("export-fails", { kind: "all", filter: {} }, 500);
    expect(events.filter((e) => e.type === "progress")).toHaveLength(1);
    expect(events.at(-1)).toMatchObject({ type: "finished", error: { kind: "network" }, written: null, missing: null, file_name: null });
  });

  it("logs a failed export as Rust does: the title, then the file, then failed", async () => {
    const mock = createMockTransport("export-fails", 0);
    setTransport(mock);
    await commands.connect("p-logistics");
    const log: { action: string; error: unknown }[] = [];
    await mock.listen("activity", (e) => log.push(e as never));
    const picked = await commands.pickExportPath("invoice-generate", "dead-letter", "json");
    await commands.exportStart("p-logistics", { entity: "invoice-generate", view: "dead-letter", scope: { kind: "all", filter: {} }, format: "json", pick: picked!.pick });
    await vi.waitFor(() => expect(log.some((e) => e.error)).toBe(true), { timeout: 2000 });
    const line = log.find((e) => e.error)!;
    expect(line.action).toBe(`Export invoice-generate (dead-letter queue) to ${picked!.name} failed`);
  });
});
