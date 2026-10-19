import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { ActivityRecord } from "@/engine";
import { renderShell } from "../test-utils";
import { activityStore, recordStore } from "./activity";

const rec = (p: Partial<ActivityRecord>): ActivityRecord => ({
  id: 100, op_id: "op-x", at: "2026-10-19T09:00:00Z", origin: "cli", profile: "Logistics", entity: "label-print", action: "purge", kind: "mutating",
  phase: "finished", outcome: "succeeded", count: 6, succeeded: 6, failed: 0, skipped: 0, summary: "Removed 6 messages from label-print", error: null, note: null, ...p,
});

async function activityTab() {
  const utils = renderShell("default", { last_profile: "p-logistics" });
  const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
  await within(drawer).findByText("Connected to Logistics");
  return { drawer, ...utils };
}

/** The log's lines, top to bottom, by their text. */
const lines = (drawer: HTMLElement) => [...drawer.querySelectorAll("[data-activity-row]")].map((r) => r.textContent ?? "");

describe("ActivityLog", () => {
  it("merges persisted and session rows by time", async () => {
    const { drawer } = await activityTab();
    act(() => {
      activityStore.clear();
      recordStore.clear();
      activityStore.add({ at: "2026-10-19T09:00:01Z", action: "Peeked label-print", duration_ms: 12, error: null });
      activityStore.add({ at: "2026-10-19T09:00:05Z", action: "Listed 4 queues", duration_ms: 30, error: null });
      recordStore.merge([rec({ id: 7, at: "2026-10-19T09:00:03Z" }), rec({ id: 9, at: "2026-10-19T09:00:07Z", origin: "mcp", summary: "Moved 2 messages from returns-intake to orders" })]);
    });
    const text = lines(drawer);
    expect(text).toHaveLength(4);
    expect(text[0]).toContain("Peeked label-print");
    expect(text[1]).toMatch(/CLI.*Removed 6 messages from label-print/);
    expect(text[2]).toContain("Listed 4 queues");
    expect(text[3]).toMatch(/MCP.*Moved 2 messages/);
  });

  it("shows a started entry only until its finished entry arrives, and says how a change ended", async () => {
    const { drawer } = await activityTab();
    act(() => {
      activityStore.clear();
      recordStore.clear();
      recordStore.merge([rec({ id: 1, op_id: "a", phase: "started", outcome: "pending", summary: "Delete 3 messages from orders on Logistics", origin: "gui" })]);
    });
    expect(lines(drawer)).toEqual([expect.stringMatching(/GUI.*Delete 3 messages from orders on Logistics.*running/)]);
    act(() => recordStore.merge([rec({ id: 2, op_id: "a", origin: "gui", outcome: "partly-failed", succeeded: 2, failed: 1, summary: "Deleted 2 of 3 messages from orders; 1 failed" })]));
    expect(lines(drawer)).toEqual([expect.stringMatching(/GUI.*Deleted 2 of 3 messages.*partly failed/)]);
    act(() => recordStore.merge([rec({ id: 3, op_id: "b", origin: "gui", outcome: "denied", summary: "Purge orders on Logistics", error: "not confirmed" })]));
    expect(lines(drawer)[1]).toMatch(/not confirmed/);
  });

  it("filters by origin, outcome and the current connection", async () => {
    const { drawer } = await activityTab();
    act(() => {
      activityStore.clear();
      recordStore.clear();
      activityStore.add({ at: "2026-10-19T09:00:01Z", action: "Peeked label-print", duration_ms: 12, error: null });
      recordStore.merge([
        rec({ id: 1, origin: "cli", profile: "Logistics" }),
        rec({ id: 2, origin: "gui", profile: "Logistics", summary: "Deleted 1 message from orders" }),
        rec({ id: 3, origin: "cli", profile: "Logistics dev", outcome: "failed", summary: "Purge of label-print failed", error: "access denied" }),
      ]);
    });
    await userEvent.click(within(drawer).getByRole("radio", { name: "CLI" }));
    expect(lines(drawer)).toHaveLength(2);
    expect(lines(drawer).every((l) => l.includes("CLI"))).toBe(true);
    await userEvent.click(within(drawer).getByRole("button", { name: "This connection only" }));
    expect(lines(drawer)).toEqual([expect.stringContaining("Removed 6 messages from label-print")]);
    await userEvent.click(within(drawer).getByRole("button", { name: "This connection only" }));
    await userEvent.click(within(drawer).getByRole("button", { name: "Failed only" }));
    expect(lines(drawer)).toEqual([expect.stringContaining("Purge of label-print failed")]);
    await userEvent.click(within(drawer).getByRole("radio", { name: "All" }));
    expect(lines(drawer)).toEqual([expect.stringContaining("Purge of label-print failed")]);
  });

  it("loads the shared log when the shell starts, and Export… writes it", async () => {
    const { drawer, mock } = await activityTab();
    await waitFor(() => expect(lines(drawer).some((l) => /CLI.*Removed 6 messages from label-print/.test(l))).toBe(true));
    await userEvent.click(within(drawer).getByRole("button", { name: "Export…" }));
    expect(await within(drawer).findByText(/Exported \d+ entries/)).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "activity_export")).toBe(true);
  });
});
