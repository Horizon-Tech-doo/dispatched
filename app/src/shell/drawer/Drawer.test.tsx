import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderShell } from "../test-utils";
import { ACTIVITY_CAP, activityStore, jobStore } from "./activity";

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

  it("logs a cancelled sign-in as cancelled, never as a failure", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    await within(drawer).findByText("Connected to Logistics");
    const pending = mock.invoke("sign_in", { tenant: null, deviceCode: false }).catch(() => {});
    await act(async () => {
      await mock.invoke("sign_in_cancel");
      await pending;
    });
    expect(await within(drawer).findByText("Sign-in cancelled")).toBeInTheDocument();
    expect(within(drawer).queryByText(/Sign-in failed/)).not.toBeInTheDocument();
    expect(within(drawer).queryByText("cancelled")).not.toBeInTheDocument();
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

  it("fills a successful job to 100% even when it finished short of its total", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => jobStore.apply({ type: "started", id: "j1", title: "Export dead letters", total: 100 }));
    act(() => jobStore.apply({ type: "progress", id: "j1", done: 40, total: 100 }));
    act(() => jobStore.apply({ type: "finished", id: "j1", error: null, summary: null, output: null }));
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    expect(within(drawer).getByText("Done")).toBeInTheDocument();
    expect(fill(drawer)).toBe("100%");
  });

  it("keeps a failed job's progress and shows its reason under the title", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => jobStore.apply({ type: "started", id: "j1", title: "Export dead letters", total: 100 }));
    act(() => jobStore.apply({ type: "progress", id: "j1", done: 40, total: 100 }));
    act(() => jobStore.apply({ type: "finished", id: "j1", error: { kind: "network", message: "connection reset by peer" }, summary: null, output: null }));
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    expect(fill(drawer)).toBe("40%");
    const item = within(drawer).getByRole("listitem");
    expect(within(item).getByText("connection reset by peer")).toBeInTheDocument();
    expect(item.querySelector("svg")).not.toBeNull();
    expect(within(item).getByText("40 / 100")).toBeInTheDocument();
  });

  it("shows a cancelled job without an alert", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => jobStore.apply({ type: "started", id: "j1", title: "Export dead letters", total: 100 }));
    act(() => jobStore.apply({ type: "progress", id: "j1", done: 40, total: 100 }));
    act(() => jobStore.apply({ type: "finished", id: "j1", error: { kind: "cancelled", message: "Cancelled" }, summary: null, output: null }));
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    const item = within(drawer).getByRole("listitem");
    expect(within(item).getByText("Cancelled")).toBeInTheDocument();
    expect(item.querySelector("svg")).toBeNull();
    expect(fill(drawer)).toBe("40%");
  });

  it("offers Show activity with the running job count whenever the drawer is hidden", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    await userEvent.click(within(drawer).getByRole("button", { name: "Hide drawer" }));
    expect(await screen.findByRole("button", { name: "Show activity" })).toBeInTheDocument();
    act(() => jobStore.apply({ type: "started", id: "j1", title: "Export dead letters", total: 100 }));
    expect(await screen.findByRole("button", { name: "Show activity · 1 running" })).toBeInTheDocument();
    act(() => jobStore.apply({ type: "finished", id: "j1", error: null, summary: null, output: null }));
    expect(await screen.findByRole("button", { name: "Show activity" })).toBeInTheDocument();
  });
});

const fill = (drawer: HTMLElement) => (drawer.querySelector("li [aria-hidden] > span") as HTMLElement).style.width;

describe("Drawer activity scrolling", () => {
  const sizes = { scrollHeight: 1000, clientHeight: 100 };
  const patched: PropertyDescriptor[] = [];
  const stub = () => {
    for (const [k, v] of Object.entries(sizes)) {
      patched.push(Object.getOwnPropertyDescriptor(HTMLElement.prototype, k) ?? { configurable: true, value: undefined });
      Object.defineProperty(HTMLElement.prototype, k, { configurable: true, get: () => v });
    }
  };
  afterEach(() => {
    for (const k of Object.keys(sizes)) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[k];
    patched.length = 0;
    vi.restoreAllMocks();
  });
  const entry = (i: number) => ({ at: "2026-10-15T09:41:12Z", action: `Step ${i}`, duration_ms: 1, error: null });
  const log = (drawer: HTMLElement) => drawer.querySelector<HTMLElement>("[data-selectable]")!;

  it("starts at the newest entry when the tab is opened again", async () => {
    stub();
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    act(() => { for (let i = 0; i < 50; i++) activityStore.add(entry(i)); });
    act(() => within(drawer).getByRole("tab", { name: "Activity" }).focus());
    expect(log(drawer).scrollTop).toBe(1000);
  });

  it("follows new entries when at the bottom, also once the cap is reached", async () => {
    stub();
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => { for (let i = 0; i < ACTIVITY_CAP; i++) activityStore.add(entry(i)); });
    const box = log(drawer);
    box.scrollTop = 900; // bottom: scrollHeight - clientHeight
    act(() => { box.dispatchEvent(new Event("scroll")); });
    act(() => activityStore.add(entry(ACTIVITY_CAP)));
    expect(activityStore.all()).toHaveLength(ACTIVITY_CAP);
    expect(box.scrollTop).toBe(1000);
  });

  it("leaves the scroll position alone when the user scrolled up", async () => {
    stub();
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => { for (let i = 0; i < 50; i++) activityStore.add(entry(i)); });
    const box = log(drawer);
    box.scrollTop = 200;
    act(() => { box.dispatchEvent(new Event("scroll")); });
    act(() => activityStore.add(entry(99)));
    expect(box.scrollTop).toBe(200);
  });
});
