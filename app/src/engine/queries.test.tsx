import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { setTransport } from "./ipc";
import { createMockTransport } from "./mock/transport";
import * as commands from "./commands";
import { checkFilterNow, createQueryClient, keys, normalizeFilter, useDeleteProfile, useEntities, useEntityDetails, useFilterCheck, useProfiles, useSaveProfile, useSignOut, useTenants } from "./queries";
import { EngineError } from "./ipc";

function wrapper() {
  const client = createQueryClient();
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("queries", () => {
  it("reads the account again when a listing says the sign-in is gone", async () => {
    const mock = createMockTransport("default");
    let signedIn = true;
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "account_current") return (signedIn ? { id: "oid-1", user_principal: "ops@contoso.example", display_name: null, tenant_id: "tenant-1" } : null) as T;
        if (cmd === "list_tenants") {
          signedIn = false; // the engine cleared the sign-in whose refresh token is gone
          throw new EngineError("not_signed_in", "the refresh token has expired; sign in again");
        }
        return mock.invoke<T>(cmd, args);
      },
    });
    const client = createQueryClient();
    const w = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const { result } = renderHook(() => useTenants(), { wrapper: w });
    // The listing failed, the account was read again and is gone, so the listing waits for a sign-in.
    await waitFor(() => expect(client.getQueryData(keys.account)).toBeNull());
    expect(client.getQueryState(keys.tenants("oid-1"))?.status).toBe("error");
    expect(result.current.fetchStatus).toBe("idle");
  });

  it("load profiles and refresh them after a save", async () => {
    setTransport(createMockTransport("default"));
    const w = wrapper();
    const { result } = renderHook(() => ({ profiles: useProfiles(), save: useSaveProfile() }), { wrapper: w });
    await waitFor(() => expect(result.current.profiles.data?.length).toBe(3));
    await commands.signIn(null, false);
    await result.current.save.mutateAsync({ name: "Fourth", auth: { kind: "entra", namespace: "fourth" }, production: false, transport: "amqp", pinned_entities: [] });
    await waitFor(() => expect(result.current.profiles.data?.length).toBe(4));
  });

  it("refreshes entities after a save and drops a deleted profile's cached listings", async () => {
    setTransport(createMockTransport("default"));
    const client = createQueryClient();
    const w = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    client.setQueryData(["entities", "p-logistics"], { queues: [] });
    client.setQueryData(["subscriptions", "p-logistics", "orders"], []);
    client.setQueryData(["entities", "p-partner"], { queues: [] });
    const { result } = renderHook(() => ({ save: useSaveProfile(), del: useDeleteProfile() }), { wrapper: w });
    await result.current.save.mutateAsync({
      name: "Fifth", auth: { kind: "connection-string", connection_string: "Endpoint=sb://f.servicebus.windows.net/;SharedAccessKeyName=k;SharedAccessKey=s" },
      production: false, transport: "amqp", pinned_entities: [],
    });
    expect(client.getQueryState(["entities", "p-partner"])?.isInvalidated).toBe(true);
    await result.current.del.mutateAsync("p-logistics");
    expect(client.getQueryData(["entities", "p-logistics"])).toBeUndefined();
    expect(client.getQueryData(["subscriptions", "p-logistics", "orders"])).toBeUndefined();
    expect(client.getQueryData(["entities", "p-partner"])).toBeDefined();
  });

  it("keeps the previous listing only while the same profile reloads, never another profile's", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    await commands.connect("p-logistics");
    const { result, rerender } = renderHook(({ id }) => useEntities(id), { wrapper: wrapper(), initialProps: { id: "p-logistics" as string | null } });
    await waitFor(() => expect(result.current.data?.queues.length).toBe(4));
    await commands.connect("p-partner");
    rerender({ id: "p-partner" });
    expect(result.current.data).toBeUndefined();
    expect(result.current.isPlaceholderData).toBe(false);
    await waitFor(() => expect(result.current.data?.queues.length).toBe(4));
  });

  it("checks a filter only when a checkable field is set", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    const { result, rerender } = renderHook(({ f }) => useFilterCheck(f), { wrapper: wrapper(), initialProps: { f: { property: "  ", body_contains: "x" } as Record<string, string> } });
    expect(result.current.fetchStatus).toBe("idle");
    expect(mock.calls.some((c) => c.cmd === "check_filter")).toBe(false);
    rerender({ f: { property: "a ==" } });
    await waitFor(() => expect(result.current.data?.map((i) => i.field)).toEqual(["property"]));
  });

  it("keys a filter by what it means: nulls, blanks and an empty filter are one filter", () => {
    expect(normalizeFilter({ property: null, body_contains: "  ", session_id: undefined })).toEqual({});
    expect(keys.peek("p", "q", "active", { kind: "start" }, { body_regex: null }, null)).toEqual(keys.peek("p", "q", "active", { kind: "start" }, {}, null));
    // Text keeps its spaces: it is sent as typed.
    expect(normalizeFilter({ body_regex: " a b ", property: "x" })).toEqual({ property: "x", body_regex: " a b " });
    // A check looks at the checked fields only.
    expect(keys.filterCheck({ property: "a", body_contains: "x" })).toEqual(keys.filterCheck({ property: "a", body_contains: "y" }));
  });

  it("checks a filter now: the cached or running answer, else one call; nothing to check is no call", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    const client = createQueryClient();
    expect(await checkFilterNow(client, { body_contains: "x" })).toEqual([]);
    expect(mock.calls.filter((c) => c.cmd === "check_filter")).toHaveLength(0);
    const [a, b] = await Promise.all([checkFilterNow(client, { property: "a ==" }), checkFilterNow(client, { property: "a ==" })]);
    expect(a.map((i) => i.field)).toEqual(["property"]);
    expect(b).toEqual(a);
    expect(mock.calls.filter((c) => c.cmd === "check_filter")).toHaveLength(1);
  });

  it("refreshes sign-ins, the account, profiles, tenants and discovery after a sign-out", async () => {
    setTransport(createMockTransport("default"));
    await commands.signIn(null, false);
    const client = createQueryClient();
    const w = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const seeded = [keys.signIns, keys.account, keys.profiles, keys.tenants("oid-1"), keys.discovery("oid-1", null), keys.entities("p-partner")];
    seeded.forEach((k) => client.setQueryData(k, []));
    const { result } = renderHook(() => useSignOut(), { wrapper: w });
    expect(await result.current.mutateAsync("oid-1")).toEqual({ closed_profile_id: null });
    expect(seeded.slice(0, 5).map((k) => client.getQueryState(k)?.isInvalidated)).toEqual([true, true, true, true, true]);
    expect(client.getQueryState(keys.entities("p-partner"))?.isInvalidated).toBe(false);
  });

  it("never shows another entity's details while the next one loads", async () => {
    const mock = createMockTransport("default", 20);
    setTransport(mock);
    await commands.connect("p-logistics");
    type P = { kind: "queue" | "subscription"; entity: string };
    const { result, rerender } = renderHook(({ kind, entity }: P) => useEntityDetails("p-logistics", kind, entity), {
      wrapper: wrapper(),
      initialProps: { kind: "queue", entity: "invoice-generate" } as P,
    });
    await waitFor(() => expect(result.current.data?.name).toBe("invoice-generate"));
    rerender({ kind: "queue", entity: "label-print" });
    expect(result.current.data).toBeUndefined();
    await waitFor(() => expect(result.current.data?.name).toBe("label-print"));
    rerender({ kind: "subscription", entity: "shipment-events/subscriptions/tracking-sync" });
    expect(result.current.data).toBeUndefined();
  });
});
