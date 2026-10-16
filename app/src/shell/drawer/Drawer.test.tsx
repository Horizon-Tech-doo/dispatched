import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { commands, setTransport } from "@/engine";
import { renderShell } from "../test-utils";
import { ACTIVITY_CAP, activityStore, createJobStore, jobStore } from "./activity";
import { finished, progress, started } from "./jobEvents";

/** The workbench with the drawer on its Jobs tab. */
async function jobsTab() {
  renderShell("default", { last_profile: "p-logistics" });
  const jobs = await screen.findByRole("region", { name: "Jobs and activity" });
  // jsdom panels swallow pointer-downs; Radix tabs activate on focus.
  act(() => within(jobs).getByRole("tab", { name: /Jobs/ }).focus());
  return jobs;
}

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
    act(() => jobStore.apply(started("j1", "Export dead letters", 3417)));
    act(() => jobStore.apply(progress("j1", 1920, 3417)));
    // jsdom panels swallow pointer-downs; Radix tabs activate on focus.
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    expect(within(drawer).getByText("Export dead letters")).toBeInTheDocument();
    expect(within(drawer).getByText("1,920 of 3,417")).toBeInTheDocument();
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

  it("shows a running job as a progress bar with its count, and Cancel named by the job", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("j1", "Export orders (dead-letter queue)", 37, "orders.json")));
    act(() => jobStore.apply(progress("j1", 5, 37)));
    const row = within(jobs).getByRole("listitem");
    const bar = within(row).getByRole("progressbar", { name: "Export orders (dead-letter queue)" });
    expect(bar).toHaveAttribute("aria-valuenow", "5");
    expect(bar).toHaveAttribute("aria-valuemax", "37");
    expect(within(row).getByText("5 of 37")).toBeInTheDocument();
    expect(within(row).getByText("orders.json")).toHaveClass("font-mono");
    const cancel = within(row).getByRole("button", { name: "Cancel Export orders (dead-letter queue)" });
    expect(cancel).toHaveTextContent("Cancel");
  });

  it("shows no bar fill and the examined count while the total is unknown", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("j1", "Export orders (dead-letter queue)", null, "orders.json")));
    act(() => jobStore.apply(progress("j1", 5, null, 1200)));
    const row = within(jobs).getByRole("listitem");
    expect(within(row).queryByRole("progressbar")).toBeNull();
    expect(within(row).getByText("5 written · 1,200 scanned")).toBeInTheDocument();
    // Without a filter nothing is skipped: only what was written.
    act(() => jobStore.apply(progress("j1", 7, null, 7)));
    expect(within(row).getByText("7 written")).toBeInTheDocument();
  });

  it("finished jobs keep their summary and offer Show in folder", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const jobs = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => within(jobs).getByRole("tab", { name: /Jobs/ }).focus());
    act(() => jobStore.apply(started("j1", "Export x (dead-letter queue)", 2, "x.json")));
    // Running: Cancel, no folder button yet.
    expect(within(jobs).queryByRole("button", { name: /^Show in/ })).toBeNull();
    act(() => jobStore.apply(finished("j1", { summary: "Exported 2 messages to x.json", output: "/tmp/x.json", written: 2, missing: 0, file_name: "x.json" })));
    const row = within(jobs).getByRole("listitem");
    expect(within(row).getByText("2 messages written")).toBeInTheDocument();
    expect(within(row).getByText("x.json")).toBeInTheDocument();
    expect(within(row).getByText("Done")).toBeInTheDocument();
    // A finished row has no bar and nothing to cancel.
    expect(within(row).queryByRole("progressbar")).toBeNull();
    expect(within(row).queryByRole("button", { name: /^Cancel/ })).toBeNull();
    expect(within(row).queryByText(/no longer there/)).toBeNull();
    // The visible words are in the name; the file tells rows apart.
    await userEvent.click(within(row).getByRole("button", { name: "Show in folder: x.json" }));
    expect(mock.calls.some((c) => c.cmd === "job_reveal" && (c.args as { id: string }).id === "j1")).toBe(true);
  });

  it("says plainly how many selected messages were no longer there, never truncated and never amber", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("j1", "Export x (dead-letter queue)", 5, "x.json")));
    act(() => jobStore.apply(finished("j1", { summary: "s", output: "/tmp/x.json", written: 3, missing: 2, file_name: "x.json" })));
    const line = within(jobs).getByText("2 selected messages were no longer there");
    expect(line).not.toHaveClass("truncate");
    expect(line.className).not.toMatch(/accent/);
    expect(within(jobs).getByText("3 messages written")).toBeInTheDocument();
    act(() => jobStore.apply(started("j2", "Export y (dead-letter queue)", 2, "y.json")));
    act(() => jobStore.apply(finished("j2", { summary: "s", output: "/tmp/y.json", written: 1, missing: 1, file_name: "y.json" })));
    expect(within(jobs).getByText("1 selected message was no longer there")).toBeInTheDocument();
    expect(within(jobs).getByText("1 message written")).toBeInTheDocument();
  });

  it("a failed reveal says so beside the job", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("gone", "Export x (dead-letter queue)", 2, "x.json")));
    act(() => jobStore.apply(finished("gone", { summary: "s", output: "/tmp/x.json", written: 2, missing: 0, file_name: "x.json" })));
    await userEvent.click(within(jobs).getByRole("button", { name: /^Show in folder/ }));
    expect(await within(jobs).findByText(/Couldn't show the file/)).toBeInTheDocument();
  });

  it("keeps a failed job's progress, names the failure and what to do, and offers Try again", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("j1", "Export orders (dead-letter queue)", 100, "orders.json")));
    act(() => jobStore.apply(progress("j1", 40, 100)));
    act(() => jobStore.apply(finished("j1", { error: { kind: "network", message: "network error: connection reset by peer" } })));
    const row = within(jobs).getByRole("listitem");
    expect(within(row).getByText("Export failed")).toBeInTheDocument();
    expect(row.querySelector("svg")).not.toBeNull();
    expect(within(row).getByText(/Check the connection, then try again/)).toBeInTheDocument();
    const detail = within(row).getByText("network error: connection reset by peer");
    expect(detail).toHaveClass("font-mono");
    expect(within(row).getByText("40 of 100")).toBeInTheDocument();
    expect(within(row).queryByRole("progressbar")).toBeNull();
    // Without how the export was started there is nothing to retry.
    expect(within(row).queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("Try again opens the export sheet with the same entity, view, scope and format", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const jobs = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => within(jobs).getByRole("tab", { name: /Jobs/ }).focus());
    act(() => jobStore.apply(started("j1", "Export orders (dead-letter queue)", 3, "orders.ndjson")));
    act(() =>
      jobStore.setRetry("j1", { entity: "invoice-generate", view: "dead-letter", filter: {}, scope: "selected", selected: [41000, 41001, 41002], format: "ndjson", total: 37 }),
    );
    act(() => jobStore.apply(finished("j1", { error: { kind: "io", message: "cannot write export: disk full" } })));
    expect(within(jobs).getByText(/Try again, or choose another folder/)).toBeInTheDocument();
    await userEvent.click(within(jobs).getByRole("button", { name: "Try again" }));
    const sheet = await screen.findByRole("dialog", { name: "Export messages" });
    expect(within(sheet).getByRole("radio", { name: "Selected (3)" })).toBeChecked();
    expect(within(sheet).getByRole("radio", { name: "NDJSON" })).toBeChecked();
  });

  it("a retry registered before the job is announced still reaches it", () => {
    const s = createJobStore();
    s.setRetry("j1", { entity: "e", view: "active", filter: {}, scope: "all", selected: [], format: "json", total: null });
    s.apply(started("j1", "Export e (active messages)"));
    expect(s.all()[0].retry).toMatchObject({ entity: "e", scope: "all" });
  });

  it("shows a cancelled job without an alert, and says no file was written", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("j1", "Export orders (dead-letter queue)", 37, "orders.json")));
    act(() => jobStore.apply(progress("j1", 5, 37)));
    act(() => jobStore.apply(finished("j1", { error: { kind: "cancelled", message: "Export cancelled." } })));
    const row = within(jobs).getByRole("listitem");
    expect(within(row).getByText("Cancelled after 5 of 37. No file was written.")).toBeInTheDocument();
    expect(within(row).getByText("Cancelled")).toBeInTheDocument();
    expect(row.querySelector("svg")).toBeNull();
    expect(within(row).queryByRole("button", { name: /^Show in/ })).toBeNull();
    expect(within(row).queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("Clear finished forgets finished jobs and keeps running ones", async () => {
    const jobs = await jobsTab();
    expect(within(jobs).queryByRole("button", { name: "Clear finished" })).toBeNull();
    act(() => jobStore.apply(started("done", "Export a (active messages)", 1, "a.json")));
    act(() => jobStore.apply(finished("done", { summary: "s", output: "/tmp/a.json", written: 1, missing: 0, file_name: "a.json" })));
    act(() => jobStore.apply(started("run", "Export b (active messages)", 10, "b.json")));
    await userEvent.click(within(jobs).getByRole("button", { name: "Clear finished" }));
    expect(within(jobs).getAllByRole("listitem")).toHaveLength(1);
    expect(within(jobs).getByText("Export b (active messages)")).toBeInTheDocument();
    expect(within(jobs).queryByRole("button", { name: "Clear finished" })).toBeNull();
  });

  it("announces a job's start and end politely, and nothing in between", async () => {
    await jobsTab();
    const live = () => screen.getByRole("status", { name: "Job updates" });
    expect(live()).toBeEmptyDOMElement();
    act(() => jobStore.apply(started("j1", "Export a (active messages)", 2, "a.json")));
    await waitFor(() => expect(live()).toHaveTextContent("Export a (active messages) started."));
    act(() => jobStore.apply(finished("j1", { summary: "s", output: "/tmp/a.json", written: 2, missing: 0, file_name: "a.json" })));
    await waitFor(() => expect(live()).toHaveTextContent("Export a (active messages) finished: 2 messages written."));
    act(() => jobStore.apply(started("j2", "Export b (active messages)", 2, "b.json")));
    act(() => jobStore.apply(finished("j2", { error: { kind: "cancelled", message: "Export cancelled." } })));
    await waitFor(() => expect(live()).toHaveTextContent("Export b (active messages) was cancelled."));
    act(() => jobStore.apply(started("j3", "Export c (active messages)", 2, "c.json")));
    act(() => jobStore.apply(finished("j3", { error: { kind: "io", message: "disk full" } })));
    await waitFor(() => expect(live()).toHaveTextContent("Export c (active messages) failed."));
  });

  it("does not announce progress, and the numbers are not a live region", async () => {
    const jobs = await jobsTab();
    const live = () => screen.getByRole("status", { name: "Job updates" });
    act(() => jobStore.apply(started("j1", "Export a (active messages)", null, "a.json")));
    await waitFor(() => expect(live()).toHaveTextContent("started."));
    const before = live().firstChild;
    act(() => jobStore.apply(progress("j1", 5, null, 1200)));
    act(() => jobStore.apply(progress("j1", 9, null, 2400)));
    expect(live().firstChild).toBe(before);
    expect(live()).toHaveTextContent(/^Export a \(active messages\) started\.$/);
    const row = within(jobs).getByRole("listitem");
    expect(within(row).getByText("9 written · 2,400 scanned").closest("[role=status]")).toBeNull();
    expect(within(row).queryByRole("status")).toBeNull();
  });

  it("announces a job that ends while the drawer is hidden", async () => {
    const jobs = await jobsTab();
    act(() => jobStore.apply(started("j1", "Export a (active messages)", 2, "a.json")));
    await userEvent.click(within(jobs).getByRole("button", { name: "Hide drawer" }));
    expect(screen.queryByRole("region", { name: "Jobs and activity" })).not.toBeInTheDocument();
    act(() => jobStore.apply(finished("j1", { summary: "s", output: "/tmp/a.json", written: 2, missing: 0, file_name: "a.json" })));
    await waitFor(() => expect(screen.getByRole("status", { name: "Job updates" })).toHaveTextContent("Export a (active messages) finished: 2 messages written."));
  });

  it("announces jobs with the same title each time they end", async () => {
    await jobsTab();
    const live = () => screen.getByRole("status", { name: "Job updates" });
    const title = "Export orders (dead-letter queue)";
    act(() => jobStore.apply(started("j1", title, 2, "orders-1.json")));
    act(() => jobStore.apply(finished("j1", { summary: "s", output: "/tmp/1.json", written: 2, missing: 0, file_name: "orders-1.json" })));
    await waitFor(() => expect(live()).toHaveTextContent(`${title} finished: 2 messages written.`));
    const first = live().querySelector("p");
    // A second job with the same title and the same result says it again: a new node, so a screen reader speaks it.
    act(() => jobStore.apply(started("j2", title, 2, "orders-2.json")));
    act(() => jobStore.apply(finished("j2", { summary: "s", output: "/tmp/2.json", written: 2, missing: 0, file_name: "orders-2.json" })));
    await waitFor(() => expect(live().querySelector("p")).not.toBe(first));
    expect(live()).toHaveTextContent(`${title} finished: 2 messages written.`);
    // Two ending in the same moment are both said.
    act(() => {
      jobStore.apply(started("j3", title, 1));
      jobStore.apply(started("j4", title, 1));
    });
    act(() => {
      jobStore.apply(finished("j3", { written: 1, missing: 0 }));
      jobStore.apply(finished("j4", { error: { kind: "io", message: "disk full" } }));
    });
    await waitFor(() => expect(live().querySelectorAll("p")).toHaveLength(2));
    expect(live()).toHaveTextContent(`${title} finished: 1 message written.`);
    expect(live()).toHaveTextContent(`${title} failed.`);
  });

  it("tints the drawer tabs' edge fade with the drawer's own surface", async () => {
    const jobs = await jobsTab();
    expect(within(jobs).getByRole("tablist", { name: "Drawer" }).className).toContain("[--tabs-fade:var(--c-side)]");
  });

  it("does not announce jobs that were already running or finished when the page mounted", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("region", { name: "Jobs and activity" });
    expect(screen.getByRole("status", { name: "Job updates" })).toBeEmptyDOMElement();
  });

  it("cancelling disables Cancel until the job ends", async () => {
    const { mock } = renderShell("export-slow", { last_profile: "p-logistics" });
    const jobs = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => within(jobs).getByRole("tab", { name: /Jobs/ }).focus());
    await screen.findByRole("tree", { name: "Namespace" });
    // The engine takes a moment to answer the cancel.
    let answer: () => void = () => {};
    setTransport({
      listen: mock.listen,
      invoke: <T,>(cmd: string, args?: Record<string, unknown>) =>
        (cmd === "job_cancel" ? new Promise<void>((r) => (answer = r)).then(() => mock.invoke(cmd, args)) : mock.invoke(cmd, args)) as Promise<T>,
    });
    const pick = await commands.pickExportPath("invoice-generate", "dead-letter", "json");
    await commands.exportStart("p-logistics", { entity: "invoice-generate", view: "dead-letter", scope: { kind: "all", filter: {} }, format: "json", pick: pick!.pick });
    await userEvent.click(await within(jobs).findByRole("button", { name: /^Cancel Export invoice-generate/ }));
    const cancelling = within(jobs).getByRole("button", { name: /^Cancelling Export invoice-generate/ });
    expect(cancelling).toBeDisabled();
    expect(cancelling).toHaveTextContent("Cancelling");
    answer();
    expect(await within(jobs).findByText("Cancelled")).toBeInTheDocument();
  });

  it("offers Show activity with the running job count whenever the drawer is hidden", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    await userEvent.click(within(drawer).getByRole("button", { name: "Hide drawer" }));
    expect(await screen.findByRole("button", { name: "Show activity" })).toBeInTheDocument();
    act(() => jobStore.apply(started("j1", "Export dead letters", 100)));
    expect(await screen.findByRole("button", { name: "Show activity · 1 running" })).toBeInTheDocument();
    act(() => jobStore.apply(finished("j1", { error: null })));
    expect(await screen.findByRole("button", { name: "Show activity" })).toBeInTheDocument();
  });
});


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
