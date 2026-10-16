import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { jobStore } from "./activity";
import { JobAnnouncer, SAID_FOR_MS } from "./JobAnnouncer";
import { finished, started } from "./jobEvents";

describe("JobAnnouncer", () => {
  beforeEach(() => {
    jobStore.clear();
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("forgets what it said after a while, and a new announcement restarts the wait", () => {
    render(<JobAnnouncer />);
    const live = () => screen.getByRole("status", { name: "Job updates" });
    act(() => jobStore.apply(started("j1", "Export a", 2)));
    expect(live()).toHaveTextContent("Export a started.");
    act(() => void vi.advanceTimersByTime(SAID_FOR_MS - 1000));
    expect(live()).toHaveTextContent("Export a started.");
    // Another line arrives: the wait starts over.
    act(() => jobStore.apply(finished("j1", { written: 2, missing: 0 })));
    act(() => void vi.advanceTimersByTime(SAID_FOR_MS - 1000));
    expect(live()).toHaveTextContent("Export a finished: 2 messages written.");
    act(() => void vi.advanceTimersByTime(1000));
    expect(live()).toBeEmptyDOMElement();
  });
});
