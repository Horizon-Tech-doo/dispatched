import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { createQueryClient, EngineError, setTransport } from "@/engine";
import { createMockTransport } from "./mock/transport";
import { usePeekStream, type PeekStreamOptions } from "./peek";

/** `fail(n)` makes the n-th `peek_page` call fail; `inFlight.max` is the most pages ever outstanding at once. */
function setup(scenario: Parameters<typeof createMockTransport>[0] = "default", delayMs = 0, fail: (n: number) => boolean = () => false) {
  const mock = createMockTransport(scenario, delayMs, { last_profile: "p-logistics" });
  const inFlight = { now: 0, max: 0, pages: 0 };
  setTransport({
    listen: mock.listen,
    invoke: async <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
      if (cmd !== "peek_page") return mock.invoke<T>(cmd, args);
      inFlight.pages++;
      inFlight.max = Math.max(inFlight.max, ++inFlight.now);
      try {
        const page = await mock.invoke<T>(cmd, args);
        if (fail(inFlight.pages)) throw new EngineError("network", "network error: connection reset by peer");
        return page;
      } finally {
        inFlight.now--;
      }
    },
  });
  const client = createQueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  return { mock, wrapper, client, inFlight };
}

const maxes = (mock: ReturnType<typeof createMockTransport>) =>
  mock.calls.filter((c) => c.cmd === "peek_page").map((c) => (c.args as { request: { max: number } }).request.max);
const scanLimits = (mock: ReturnType<typeof createMockTransport>) =>
  mock.calls.filter((c) => c.cmd === "peek_page").map((c) => (c.args as { request: { scan_limit?: number } }).request.scan_limit);

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
    expect(result.current.target).toBe(100);
    act(() => result.current.peekMore());
    expect(result.current.target).toBe(200);
    await waitFor(() => expect(result.current.messages).toHaveLength(200));
  });

  it("stream stops at the scan budget and reports it", async () => {
    const { mock, wrapper } = setup("many-messages");
    await connected(mock);
    const nothing = { property: "sys.Subject = 'no-such-subject'" };
    const { result } = renderHook(() => usePeekStream({ ...base, entity: "customs-docs", view: "active", filter: nothing, scanBudget: 6_000 }), { wrapper });
    await waitFor(() => expect(result.current.budgetHit).toBe(true));
    expect(result.current.messages).toHaveLength(0);
    // The last page asks to scan only what is left of the budget.
    expect(result.current.scanned).toBe(6_000);
    expect(scanLimits(mock)).toEqual([5_000, 1_000]);
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
    expect(scanLimits(mock)).toEqual([5_000, 1_000, 5_000, 1_000]);
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

  it("keeps the rows when a page fails mid-stream, and Peek more resumes from it", async () => {
    const { mock, wrapper } = setup("default", 0, (n) => n === 3);
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 1500 }), { wrapper });
    await waitFor(() => expect(result.current.error?.kind).toBe("network"));
    expect(result.current.messages).toHaveLength(1000);
    expect(result.current.canPeekMore).toBe(true);
    expect(result.current.canRetry).toBe(true);
    act(() => result.current.peekMore());
    await waitFor(() => expect(result.current.messages.length).toBeGreaterThan(1000));
    expect(result.current.error).toBeNull();
    const seqs = result.current.messages.map((m) => m.sequence_number);
    expect(new Set(seqs).size).toBe(seqs.length);
  });

  it("retry re-reads the failed page and the round continues", async () => {
    const { mock, wrapper } = setup("default", 0, (n) => n === 3);
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 1200 }), { wrapper });
    await waitFor(() => expect(result.current.error).not.toBeNull());
    expect(result.current.messages).toHaveLength(1000);
    act(() => result.current.retry());
    await waitFor(() => expect(result.current.messages).toHaveLength(1200));
    expect(result.current.canRetry).toBe(false);
    expect(maxes(mock)).toEqual([500, 500, 200, 200]);
  });

  it("retry after a failed first page loads it", async () => {
    const { mock, wrapper } = setup("default", 0, (n) => n === 1);
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 100 }), { wrapper });
    await waitFor(() => expect(result.current.canRetry).toBe(true));
    expect(result.current.canPeekMore).toBe(false);
    act(() => result.current.retry());
    await waitFor(() => expect(result.current.messages).toHaveLength(100));
  });

  it("a refetch of the whole stream asks for the same page sizes", async () => {
    const { mock, wrapper, client } = setup();
    await connected(mock);
    const { result } = renderHook(() => usePeekStream(base), { wrapper });
    await waitFor(() => expect(result.current.messages).toHaveLength(1200));
    await act(() => client.refetchQueries({ queryKey: ["peek"] }));
    expect(maxes(mock)).toEqual([500, 500, 200, 500, 500, 200]);
    expect(result.current.messages).toHaveLength(1200);
  });

  it("a reset query starts a fresh round at Top N", async () => {
    const { mock, wrapper, client } = setup();
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 100 }), { wrapper });
    await waitFor(() => expect(result.current.messages).toHaveLength(100));
    act(() => result.current.peekMore());
    await waitFor(() => expect(result.current.messages).toHaveLength(200));
    await act(() => client.resetQueries({ queryKey: ["peek"] }));
    await waitFor(() => expect(result.current.messages).toHaveLength(100));
    await new Promise((r) => setTimeout(r, 30));
    expect(result.current.messages).toHaveLength(100);
    expect(maxes(mock).at(-1)).toBe(100);
  });

  it("a reset query after Stop streams again", async () => {
    const { mock, wrapper, client } = setup("default", 20);
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 1200 }), { wrapper });
    await waitFor(() => expect(result.current.messages.length).toBeGreaterThanOrEqual(500));
    act(() => result.current.stop());
    await act(() => client.resetQueries({ queryKey: ["peek"] }));
    await waitFor(() => expect(result.current.messages).toHaveLength(1200));
    expect(result.current.stopped).toBe(false);
  });

  it("never has more than one page outstanding", async () => {
    const { mock, wrapper, inFlight } = setup("default", 20);
    await connected(mock);
    const { result } = renderHook(() => usePeekStream({ ...base, topN: 2600 }), { wrapper });
    await waitFor(() => expect(result.current.messages).toHaveLength(2600), { timeout: 3000 });
    expect(inFlight.pages).toBe(6);
    expect(inFlight.max).toBe(1);
  });
});
