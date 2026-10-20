import { describe, expect, it } from "vitest";
import type { ReceivedView } from "@/engine";
import { ROLLING_ROWS, createListenerStore, ratesOf } from "./listenerStore";

const m = (seq: number, size = 10): ReceivedView => ({ sequence_number: String(seq), body_size: size }) as ReceivedView;

describe("listener store", () => {
  it("ring_keeps_last_1000_newest_first", () => {
    const s = createListenerStore();
    s.apply({ type: "started", id: "j", kind: "listen", title: "Watch orders", total: null, file_name: null });
    for (let i = 1; i <= ROLLING_ROWS + 5; i++) s.apply({ type: "received", id: "j", message: m(i) }, 1_000);
    const l = s.get("j")!;
    expect(l.rows).toHaveLength(ROLLING_ROWS);
    expect(l.rows[0].sequence_number).toBe(String(ROLLING_ROWS + 5));
    expect(l.total).toBe(ROLLING_ROWS + 5);
  });

  it("ignores jobs that are not listeners", () => {
    const s = createListenerStore();
    s.apply({ type: "started", id: "r", kind: "receive", title: "Receive", total: null, file_name: null });
    s.apply({ type: "received", id: "r", message: m(1) }, 0);
    expect(s.get("r")).toBeUndefined();
  });

  it("rate_buckets_roll_over_60_seconds", () => {
    const s = createListenerStore();
    s.apply({ type: "started", id: "j", kind: "listen", title: "Watch", total: null, file_name: null });
    s.apply({ type: "received", id: "j", message: m(1, 100) }, 10_000);
    s.apply({ type: "received", id: "j", message: m(2, 50) }, 10_500);
    s.apply({ type: "received", id: "j", message: m(3, 10) }, 11_200);
    let r = ratesOf(s.get("j")!, 11);
    expect(r.messages.at(-1)).toBe(1);
    expect(r.messages.at(-2)).toBe(2);
    expect(r.bytes.at(-2)).toBe(150);
    expect(r.messages).toHaveLength(60);
    // A minute later the old seconds have rolled out.
    s.apply({ type: "received", id: "j", message: m(4, 1) }, 75_000);
    r = ratesOf(s.get("j")!, 75);
    expect(r.messages.reduce((a, b) => a + b, 0)).toBe(1);
  });
});
