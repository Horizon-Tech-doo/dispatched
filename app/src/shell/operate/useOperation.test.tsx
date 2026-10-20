import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { setTransport, type JobStarted } from "@/engine";
import { useOperation } from "./useOperation";

describe("useOperation", () => {
  it("a job that starts after its view went away is cancelled, so nothing runs out of sight", async () => {
    const calls: { cmd: string; args?: Record<string, unknown> }[] = [];
    setTransport({
      async invoke<T>(cmd: string, args?: Record<string, unknown>) {
        calls.push({ cmd, args });
        return (cmd === "job_cancel" ? true : null) as T;
      },
      async listen() {
        return () => {};
      },
    });
    let release!: (s: JobStarted) => void;
    const { result, unmount } = renderHook(() => useOperation());
    let started!: Promise<void>;
    act(() => {
      started = result.current.start(() => new Promise<JobStarted>((r) => (release = r)));
    });
    unmount();
    await act(async () => {
      release({ id: "job-late" });
      await started;
    });
    expect(calls).toContainEqual({ cmd: "job_cancel", args: { id: "job-late" } });
  });
});
