import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";
import { activityStore, jobStore } from "./activity";

describe("Drawer", () => {
  it("logs engine calls with durations and keeps jobs empty with an explanation", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    expect(await within(drawer).findByText("Connected to Logistics")).toBeInTheDocument();
    expect(await within(drawer).findByText(/Listed 4 queues and 4 topics/)).toBeInTheDocument();
    // jsdom panels swallow pointer-downs; Radix tabs activate on focus.
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    expect(within(drawer).getByText(/Exports and bulk operations show their progress here/)).toBeInTheDocument();
  });

  it("shows failures with their reason", async () => {
    renderShell("network", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    expect(await within(drawer).findByText("Listing the namespace failed")).toBeInTheDocument();
    expect(within(drawer).getByText(/connection reset by peer/)).toBeInTheDocument();
  });

  it("shows a cancelled operation as a plain line, not an error", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    await within(drawer).findByText("Connected to Logistics");
    act(() => activityStore.add({ at: "2026-10-15T09:41:12Z", action: "Export dead letters", duration_ms: 800, error: { kind: "cancelled", message: "Cancelled" } }));
    const line = (await within(drawer).findByText("Export dead letters")).closest("div")!;
    expect(within(line).getByText("cancelled")).toBeInTheDocument();
    expect(line.querySelector("svg")).toBeNull();
    expect(within(drawer).queryByText("Cancelled")).not.toBeInTheDocument();
  });

  it("lists running jobs with progress and a count on the tab", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => jobStore.apply({ type: "started", id: "j1", title: "Export dead letters", total: 3417 }));
    act(() => jobStore.apply({ type: "progress", id: "j1", done: 1920, total: 3417 }));
    // jsdom panels swallow pointer-downs; Radix tabs activate on focus.
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    expect(within(drawer).getByText("Export dead letters")).toBeInTheDocument();
    expect(within(drawer).getByText("1,920 / 3,417")).toBeInTheDocument();
    expect(within(drawer).getByRole("tab", { name: "Jobs 1" })).toBeInTheDocument();
  });

  it("hides with the collapse button and comes back from the status bar", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    await userEvent.click(within(drawer).getByRole("button", { name: "Hide drawer" }));
    expect(screen.queryByRole("region", { name: "Jobs and activity" })).not.toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "Show activity" }));
    expect(await screen.findByRole("region", { name: "Jobs and activity" })).toBeInTheDocument();
  });
});
