import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { setTransport } from "@/engine";
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

  it("a topic has no counts, but shows its size, and only its own address", async () => {
    renderShell("default", seed("shipment-events", "topic"));
    const addresses = await screen.findByRole("region", { name: "Addresses" });
    expect(screen.queryByRole("region", { name: "Counts" })).toBeNull();
    expect(within(addresses).queryByRole("button", { name: "Copy dead-letter URL" })).toBeNull();
    expect(within(addresses).queryByRole("button", { name: "Copy connection string" })).toBeNull();
    const props = screen.getByRole("region", { name: "Properties" });
    expect(within(props).getByText("Size")).toBeInTheDocument();
    expect(within(props).getByText("4.0 MiB")).toBeInTheDocument();
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
});
