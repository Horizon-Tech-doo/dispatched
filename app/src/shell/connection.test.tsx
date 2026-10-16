import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { createQueryClient, setTransport } from "@/engine";
import { createMockTransport } from "@/engine/mock/transport";
import { useConnectionController } from "./connection";

/** The mock, with `connect` for one profile held until the test releases it. */
function slowConnect(profileId: string) {
  const mock = createMockTransport("default");
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  setTransport({
    listen: mock.listen,
    invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "connect" && args?.profileId === profileId) await gate;
      return mock.invoke<T>(cmd, args);
    },
  });
  return release;
}

function wrapper() {
  const client = createQueryClient();
  return ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("connection controller", () => {
  it("keeps the latest connect when an earlier one finishes after it", async () => {
    const release = slowConnect("p-logistics");
    const { result } = renderHook(() => useConnectionController(), { wrapper: wrapper() });
    let first!: Promise<void>;
    act(() => void (first = result.current.connect("p-logistics")));
    await act(() => result.current.connect("p-partner"));
    expect(result.current.status).toBe("connected");
    expect(result.current.view?.profile_id).toBe("p-partner");
    release();
    await act(() => first);
    expect(result.current.profileId).toBe("p-partner");
    expect(result.current.view?.profile_id).toBe("p-partner");
  });

  it("drops a connect that finishes after a disconnect", async () => {
    const release = slowConnect("p-logistics");
    const { result } = renderHook(() => useConnectionController(), { wrapper: wrapper() });
    let first!: Promise<void>;
    act(() => void (first = result.current.connect("p-logistics")));
    await act(() => result.current.disconnect());
    release();
    await act(() => first);
    await waitFor(() => expect(result.current.status).toBe("idle"));
  });

  it("drops the state of a connection the engine closed, without calling disconnect again", async () => {
    const mock = createMockTransport("default");
    setTransport(mock);
    const { result } = renderHook(() => useConnectionController(), { wrapper: wrapper() });
    await act(() => result.current.connect("p-logistics"));
    expect(result.current.status).toBe("connected");
    act(() => result.current.closedByEngine("p-partner")); // some other connection: nothing to do
    expect(result.current.status).toBe("connected");
    act(() => result.current.closedByEngine("p-logistics"));
    expect(result.current.status).toBe("idle");
    expect(result.current.profileId).toBeNull();
    expect(mock.calls.some((c) => c.cmd === "disconnect")).toBe(false);
    expect(mock.calls).toContainEqual({ cmd: "settings_set", args: { key: "last_profile", value: null } });
  });

  it("drops peeks and message details on connect and disconnect, so another connection never shows them", async () => {
    setTransport(createMockTransport("default"));
    const client = createQueryClient();
    const { result } = renderHook(() => useConnectionController(), {
      wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
    });
    const seed = () => {
      client.setQueryData(["peek", "p-logistics", "invoice-generate", "active"], { pages: [], pageParams: [] });
      client.setQueryData(["message", "p-logistics", "invoice-generate", "active", 1], null);
    };
    const cached = () => client.getQueryCache().findAll().filter((q) => q.queryKey[0] === "peek" || q.queryKey[0] === "message").length;
    seed();
    await act(() => result.current.connect("p-logistics"));
    expect(cached()).toBe(0);
    seed();
    await act(() => result.current.disconnect());
    expect(cached()).toBe(0);
    seed();
    await act(() => result.current.connect("p-logistics"));
    seed();
    act(() => result.current.closedByEngine("p-logistics"));
    expect(cached()).toBe(0);
  });
});
