import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { setTransport } from "./ipc";
import { createMockTransport } from "./mock/transport";
import * as commands from "./commands";
import { createQueryClient, useDeleteProfile, useEntities, useProfiles, useSaveProfile } from "./queries";

function wrapper() {
  const client = createQueryClient();
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("queries", () => {
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
});
