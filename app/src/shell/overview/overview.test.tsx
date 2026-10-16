import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { keys, setTransport } from "@/engine";
import { activityStore } from "../drawer/activity";
import { renderShell } from "../test-utils";

const seed = (id: string, kind: string, profile = "p-logistics") => ({
  last_profile: profile,
  [`shell:${profile}`]: { tabs: [{ id, kind, subTab: "overview" }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

describe("overview", () => {
  it("shows counts with the dead-letter number in amber, and the properties", async () => {
    renderShell("default", seed("invoice-generate", "queue"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    const dlq = within(counts).getByText("37");
    expect(dlq.className).toContain("text-accent");
    expect(within(counts).getByText("1,284")).toBeInTheDocument();
    const props = screen.getByRole("region", { name: "Properties" });
    expect(within(props).getByText("Lock duration")).toBeInTheDocument();
    expect(within(props).getByText("30 s")).toBeInTheDocument();
    expect(within(props).getByText("Never")).toBeInTheDocument();
  });

  it("copies the URL and the dead-letter URL", async () => {
    const user = userEvent.setup(); // installs the clipboard stub
    renderShell("default", seed("invoice-generate", "queue"));
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    await user.click(within(addresses).getByRole("button", { name: "Copy URL" }));
    expect(await navigator.clipboard.readText()).toBe("sb://sb-logistics-prod.servicebus.windows.net/invoice-generate");
    await user.click(within(addresses).getByRole("button", { name: "Copy dead-letter URL" }));
    expect(await navigator.clipboard.readText()).toBe("sb://sb-logistics-prod.servicebus.windows.net/invoice-generate/$DeadLetterQueue");
  });

  it("asks the engine to copy the connection string and never shows it", async () => {
    const { mock } = renderShell("default", seed("invoice-generate", "queue"));
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    await userEvent.click(within(addresses).getByRole("button", { name: "Copy connection string" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "copy_entity_connection_string")).toBe(true));
    expect(screen.queryByText(/SharedAccessKey/)).toBeNull();
  });

  it("reading the details logs nothing unless it fails", async () => {
    // Overview polls every 10 s; Rust and the mock log entity_details only on failure.
    renderShell("default", seed("invoice-generate", "queue"));
    await screen.findByRole("region", { name: "Counts" });
    expect(activityStore.all().some((e) => /details/i.test(e.action))).toBe(false);
  });

  it("Entra connections explain why there is no connection string", async () => {
    renderShell("default", seed("invoice-generate", "queue", "p-logistics-dev"));
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    expect(within(addresses).getByRole("button", { name: "Copy connection string" })).toBeDisabled();
    expect(within(addresses).getByText(/Entra ID connections have no connection string/)).toBeInTheDocument();
  });

  it("a topic shows its size first, its subscriptions, and only its own address", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default", seed("shipment-events", "topic"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    expect(within(counts).getByText(/4\.0 MiB/)).toHaveTextContent("4.0 MiB of 1.0 GiB");
    expect(within(counts).queryByText("Active")).toBeNull();
    const addresses = screen.getByRole("region", { name: "Addresses" });
    expect(within(addresses).queryByRole("button", { name: "Copy dead-letter URL" })).toBeNull();
    // A topic can be copied as a connection string too.
    await waitFor(() => expect(within(addresses).getByRole("button", { name: "Copy connection string" })).toBeEnabled());
    const table = await screen.findByRole("table", { name: "Subscriptions" });
    const row = within(table).getByRole("row", { name: /tracking-sync/ });
    expect(within(row).getByText("2,906").className).toContain("text-fg");
    expect(within(row).getByText("3,417").className).toContain("text-accent");
    expect(within(within(table).getByRole("row", { name: /billing/ })).getAllByText("0").every((e) => e.className.includes("text-muted"))).toBe(true);
    // The topic's own size is not repeated in the properties.
    expect(within(screen.getByRole("region", { name: "Properties" })).queryByText("Size")).toBeNull();
    await user.click(within(table).getByRole("button", { name: "tracking-sync" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "entity_details" && (c.args as { entity?: string })?.entity === "shipment-events/subscriptions/tracking-sync")).toBe(true));
  });

  it("a subscription uses the Subscriptions path in its URLs", async () => {
    renderShell("default", seed("shipment-events/subscriptions/tracking-sync", "subscription"));
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    expect(within(addresses).getByText("sb://sb-logistics-prod.servicebus.windows.net/shipment-events/Subscriptions/tracking-sync")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Counts" })).getByText("3,417")).toBeInTheDocument();
  });

  it("counts the service did not report show as unknown, and say so", async () => {
    const { mock } = renderShell("default", seed("invoice-generate", "queue"));
    setTransport({
      listen: (e, cb) => mock.listen(e, cb),
      async invoke<T>(cmd: string, args?: Record<string, unknown>) {
        const r = await mock.invoke<T>(cmd, args);
        return (cmd === "entity_details" ? { ...(r as object), counts: null } : r) as T;
      },
    });
    await screen.findByRole("region", { name: "Properties" });
    // The next poll or a refresh reads through the wrapper; F5 does the latter.
    await userEvent.keyboard("{F5}");
    const counts = await screen.findByRole("region", { name: "Counts" });
    await waitFor(() => expect(within(counts).getAllByText("?").length).toBe(5));
    expect(within(counts).getByText(/The service didn't report counts/)).toBeInTheDocument();
  });

  it("a failed connection string copy says why, next to the button", async () => {
    const { mock } = renderShell("default", seed("invoice-generate", "queue"));
    setTransport({
      listen: (e, cb) => mock.listen(e, cb),
      async invoke<T>(cmd: string, args?: Record<string, unknown>) {
        if (cmd === "copy_entity_connection_string") throw Object.assign(new Error("Logistics has no stored connection string."), { kind: "invalid" });
        return mock.invoke<T>(cmd, args);
      },
    });
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    await userEvent.click(within(addresses).getByRole("button", { name: "Copy connection string" }));
    expect(await within(addresses).findByRole("alert")).toHaveTextContent("Logistics has no stored connection string.");
  });

  it("draws counts as data: zero is quiet, a number is primary, only dead letters turn amber", async () => {
    renderShell("default", seed("invoice-generate", "queue"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    expect(within(counts).getByText("1,284").className).toContain("text-fg");
    expect(within(counts).getByText("37").className).toContain("text-accent");
    for (const zero of within(counts).getAllByText("0")) expect(zero.className).toContain("text-muted");
  });

  it("shows one Size row with the limit, and leaves the size, the limit and the message count out of the properties", async () => {
    renderShell("default", seed("invoice-generate", "queue"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    expect(within(counts).getByText(/80\.0 KiB/)).toHaveTextContent("80.0 KiB of 1.0 GiB");
    const props = screen.getByRole("region", { name: "Properties" });
    for (const gone of ["Max size", "Messages", "Size"]) expect(within(props).queryByText(gone)).toBeNull();
  });

  it("groups the properties, with the raw value on hover and machine text for times", async () => {
    const user = userEvent.setup();
    renderShell("default", seed("shipment-events/subscriptions/tracking-sync", "subscription"));
    const props = await screen.findByRole("region", { name: "Properties" });
    expect(within(props).getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["Delivery", "Lifecycle"]);
    expect(within(props).getByText("30 s")).toHaveAttribute("title", "PT30S");
    expect(within(props).getByText("14 d")).toBeInTheDocument();
    const created = within(props).getByText("Created").nextElementSibling!;
    expect(created.className).toContain("font-mono");
    expect(created.className).toContain("tabular-nums");
    // Copy JSON puts the raw names and values on the clipboard.
    await user.click(within(props).getByRole("button", { name: "Copy JSON" }));
    const json = JSON.parse(await navigator.clipboard.readText());
    expect(json).toMatchObject({ LockDuration: "PT30S", RequiresSession: "false", CreatedAt: "2026-09-01T08:00:00Z" });
  });

  it("a failed connection string copy explains itself, and clears on the next attempt", async () => {
    const { mock } = renderShell("default", seed("invoice-generate", "queue"));
    let fail = true;
    setTransport({
      listen: (e, cb) => mock.listen(e, cb),
      async invoke<T>(cmd: string, args?: Record<string, unknown>) {
        if (cmd === "copy_entity_connection_string" && fail) throw Object.assign(new Error("this connection string only works for `orders`"), { kind: "invalid" });
        return mock.invoke<T>(cmd, args);
      },
    });
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    await waitFor(() => expect(within(addresses).getByRole("button", { name: "Copy connection string" })).toBeEnabled());
    await userEvent.click(within(addresses).getByRole("button", { name: "Copy connection string" }));
    const alert = await within(addresses).findByRole("alert");
    expect(within(alert).getByText("Couldn't copy the connection string")).toBeInTheDocument();
    expect(alert).toHaveTextContent(/scoped to another entity/);
    expect(alert).toHaveTextContent("only works for `orders`");
    fail = false;
    await userEvent.click(within(addresses).getByRole("button", { name: "Copy connection string" }));
    await waitFor(() => expect(within(addresses).queryByRole("alert")).toBeNull());
  });

  it("keeps the connection string button off until the connection list has loaded", async () => {
    const { mock, client } = renderShell("default", seed("invoice-generate", "queue"));
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    await waitFor(() => expect(within(addresses).getByRole("button", { name: "Copy connection string" })).toBeEnabled());
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    setTransport({
      listen: (e, cb) => mock.listen(e, cb),
      async invoke<T>(cmd: string, args?: Record<string, unknown>) {
        if (cmd === "profiles_list") await gate;
        return mock.invoke<T>(cmd, args);
      },
    });
    // Not awaited: the reset waits for the list that is held back.
    act(() => void client.resetQueries({ queryKey: keys.profiles, exact: true }));
    await waitFor(() => expect(within(addresses).getByRole("button", { name: "Copy connection string" })).toBeDisabled());
    release();
    await waitFor(() => expect(within(addresses).getByRole("button", { name: "Copy connection string" })).toBeEnabled());
  });
});

describe("overview details the counts block cannot show", () => {
  it("keeps a maximum size that is not a number in the properties", async () => {
    const { mock } = renderShell("default", seed("shipment-events", "topic"));
    setTransport({
      listen: (e, cb) => mock.listen(e, cb),
      async invoke<T>(cmd: string, args?: Record<string, unknown>) {
        const r = await mock.invoke<T>(cmd, args);
        if (cmd !== "entity_details") return r;
        const d = r as { properties: { name: string; value: string }[] };
        return { ...d, properties: d.properties.map((p) => (p.name === "MaxSizeInMegabytes" ? { ...p, value: "unlimited" } : p)) } as T;
      },
    });
    await userEvent.keyboard("{F5}");
    const props = await screen.findByRole("region", { name: "Properties" });
    await waitFor(() => expect(within(props).getByText("Max size")).toBeInTheDocument());
    expect(within(props).getByText("unlimited")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Counts" })).getByText("4.0 MiB")).toBeInTheDocument();
  });
});

describe("overview polling", () => {
  afterEach(() => vi.useRealTimers());

  it("reads the details again every 10 s while shown, and stops when it is gone", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { mock, unmount } = renderShell("default", seed("invoice-generate", "queue"));
    const reads = () => mock.calls.filter((c) => c.cmd === "entity_details").length;
    await screen.findByRole("region", { name: "Counts" });
    const first = reads();
    expect(first).toBeGreaterThan(0);
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    await waitFor(() => expect(reads()).toBe(first + 1));
    unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(40_000); });
    expect(reads()).toBe(first + 1);
  });

  it("reads a topic's subscriptions again every 10 s too, without logging the reads", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { mock, unmount } = renderShell("default", seed("shipment-events", "topic"));
    const reads = () => mock.calls.filter((c) => c.cmd === "list_subscriptions").length;
    await screen.findByRole("table", { name: "Subscriptions" });
    const first = reads();
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    await waitFor(() => expect(reads()).toBe(first + 1));
    expect(activityStore.all().some((e) => /subscription/i.test(e.action))).toBe(false);
    unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(40_000); });
    expect(reads()).toBe(first + 1);
  });

  it("logs a poll that fails, and keeps the counts it had", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { mock } = renderShell("default", seed("invoice-generate", "queue"));
    await screen.findByRole("region", { name: "Counts" });
    // The next read finds nothing there: the engine fails it and logs the failure.
    setTransport({
      listen: (e, cb) => mock.listen(e, cb),
      invoke: <T,>(cmd: string, args?: Record<string, unknown>) => mock.invoke<T>(cmd, cmd === "entity_details" ? { ...args, entity: "deleted-queue" } : args),
    });
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    await waitFor(() => expect(activityStore.all().some((e) => /Reading the details of deleted-queue failed/.test(e.action))).toBe(true));
    const counts = screen.getByRole("region", { name: "Counts" });
    expect(within(counts).getByText("1,284")).toBeInTheDocument();
    expect(within(counts).getByText("Couldn't refresh the counts")).toBeInTheDocument();
  });
});

describe("overview counts the service could not give exactly", () => {
  it("marks counts taken by peeking with ~, and capped ones with ~ and +", async () => {
    renderShell("counts-approx", seed("invoice-generate", "queue"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    expect(within(counts).getByText("~1,284")).toBeInTheDocument();
    expect(within(counts).getByText("~37").className).toContain("text-accent");
    expect(within(counts).getByText(/Counted by peeking\./)).toBeInTheDocument();
  });
  it("says when peeking stopped at its limit", async () => {
    renderShell("counts-capped", seed("invoice-generate", "queue"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    expect(within(counts).getByText("~1,284+")).toBeInTheDocument();
    expect(within(counts).getByText(/stopped at the limit/)).toBeInTheDocument();
  });
  it("shows ? for every count the service did not report", async () => {
    renderShell("counts-unknown", seed("invoice-generate", "queue"));
    const counts = await screen.findByRole("region", { name: "Counts" });
    expect(within(counts).getAllByText("?")).toHaveLength(5);
  });
});
