import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { createQueryClient, setTransport } from "@/engine";
import { createMockTransport } from "./mock/transport";
import { usePeekStream, type PeekStreamOptions } from "./peek";

function setup(scenario: Parameters<typeof createMockTransport>[0] = "default", delayMs = 0) {
  const mock = createMockTransport(scenario, delayMs, { last_profile: "p-logistics" });
  setTransport(mock);
  const client = createQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  return { mock, wrapper };
}

async function connected(mock: ReturnType<typeof createMockTransport>) {
  await mock.invoke("connect", { profileId: "p-logistics" });
}

const base: PeekStreamOptions = {
  profileId: "p-logistics",
  entity: "shipment-events/subscriptions/tracking-sync",
  view: "dead-letter",
  from: { kind: "start" },
  topN: 1200,
  filter: {},
};

describe("usePeekStream", () => {
  it("streams pages until Top N and asks for no more", async () => {
    const { mock, wrapper } = setup();
    await connected(mock);
    const { result } = renderHook(() => usePeekStream(base), { wrapper });
    await waitFor(() => expect(result.current.messages).toHaveLength(1200));
    const pages = mock.calls.filter((c) => c.cmd === "peek_page");
    expect(pages.map((c) => (c.args as { request: { max: number } }).request.max)).toEqual([500, 500, 200]);
    expect(result.current.canPeekMore).toBe(true);
    expect(result.current.exhausted).toBe(false);
  });

  it("peek more reads another Top N", async () => {
    const { mock, wrapper } = setup();
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 100 }), { wrapper });
    await waitFor(() => expect(result.current.messages).toHaveLength(100));
    act(() => result.current.peekMore());
    await waitFor(() => expect(result.current.messages).toHaveLength(200));
  });

  it("stream stops at the scan budget and reports it", async () => {
    const { mock, wrapper } = setup("many-messages");
    await connected(mock);
    const nothing = { property: "sys.Subject = 'no-such-subject'" };
    const { result } = renderHook(() => usePeekStream({ ...base, entity: "customs-docs", view: "active", filter: nothing, scanBudget: 6_000 }), { wrapper });
    await waitFor(() => expect(result.current.budgetHit).toBe(true));
    expect(result.current.messages).toHaveLength(0);
    expect(result.current.scanned).toBe(10_000);
    expect(result.current.canPeekMore).toBe(true);
  });

  it("peek more after the budget ran out scans a fresh budget", async () => {
    const { mock, wrapper } = setup("many-messages");
    await connected(mock);
    const nothing = { property: "sys.Subject = 'no-such-subject'" };
    const { result } = renderHook(() => usePeekStream({ ...base, entity: "customs-docs", view: "active", filter: nothing, scanBudget: 6_000 }), { wrapper });
    await waitFor(() => expect(result.current.budgetHit).toBe(true));
    act(() => result.current.peekMore());
    // 12,000 messages: the second round reaches the end before its budget.
    await waitFor(() => expect(result.current.exhausted).toBe(true));
    expect(result.current.scanned).toBe(12_000);
    expect(result.current.budgetHit).toBe(false);
    expect(result.current.canPeekMore).toBe(false);
  });

  it("stop keeps the rows already loaded", async () => {
    // A 20 ms mock, so a page is still in flight when Stop runs (a 0 ms one could finish first).
    const { mock, wrapper } = setup("default", 20);
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 3000 }), { wrapper });
    await waitFor(() => expect(result.current.messages.length).toBeGreaterThanOrEqual(500));
    act(() => result.current.stop());
    const kept = result.current.messages.length;
    await new Promise((r) => setTimeout(r, 50));
    expect(result.current.messages.length).toBeLessThanOrEqual(kept + 500);
    expect(result.current.stopped).toBe(true);
  });

  it("from the end returns the last N with no continuation", async () => {
    const { mock, wrapper } = setup();
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, from: { kind: "end" }, topN: 10 }), { wrapper });
    await waitFor(() => expect(result.current.messages).toHaveLength(10));
    const seqs = result.current.messages.map((m) => m.sequence_number);
    expect(seqs).toEqual([...seqs].sort((a, b) => a - b));
    expect(seqs.at(-1)).toBe(48213);
    expect(result.current.canPeekMore).toBe(false);
    expect(result.current.exhausted).toBe(true);
    expect(result.current.olderNotExamined).toBe(false);
  });

  it("peek stream ignores pages after the key changes", async () => {
    const { mock, wrapper } = setup();
    await connected(mock);
    const { result, rerender } = renderHook((o: PeekStreamOptions) => usePeekStream(o), { wrapper, initialProps: base });
    await waitFor(() => expect(result.current.messages.length).toBeGreaterThan(0));
    rerender({ ...base, entity: "invoice-generate" });
    await waitFor(() => expect(result.current.messages).toHaveLength(37));
    expect(result.current.messages.every((m) => m.sequence_number < 44_000)).toBe(true);
  });

  it("reports a failed page as the stream's error and keeps the rows", async () => {
    const { mock, wrapper } = setup();
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, profileId: "p-partner" }), { wrapper });
    await waitFor(() => expect(result.current.error?.kind).toBe("not_connected"));
    expect(result.current.messages).toEqual([]);
    expect(result.current.loading).toBe(false);
  });
});
