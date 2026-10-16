import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError, setTransport } from "@/engine";
import { renderShell } from "../test-utils";

const seedTab = (id: string, kind: "queue" | "subscription", subTab: string) => ({
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id, kind, subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

const grid = () => screen.findByRole("grid", { name: /dead-letter|messages/i });

describe("message views", () => {
  it("lists dead letters with reasons, counts and the read-only note", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    expect(await within(g).findAllByText("MaxDeliveryCountExceeded")).not.toHaveLength(0);
    const status = screen.getByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("37 messages"));
    expect(status).toHaveTextContent("End of the dead-letter queue");
    expect(status).toHaveTextContent("Peek only, nothing locked");
  });

  it("peek more reads the next Top N", async () => {
    renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    await waitFor(() => expect(status).toHaveTextContent("200 messages"));
  });

  it("from the end shows the newest messages and no Peek more", async () => {
    renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    await grid();
    await userEvent.click(screen.getByRole("radio", { name: "End" }));
    const status = screen.getByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("Last 100 messages"));
    expect(within(status).queryByRole("button", { name: "Peek more" })).toBeNull();
    // The last 100 dead letters are 48114..48213, oldest first.
    expect(within(await grid()).getAllByRole("row")[1]).toHaveTextContent("48114");
  });

  it("sorts by sequence number descending", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    const seqHeader = within(g).getByRole("columnheader", { name: /Seq/ });
    await userEvent.click(within(seqHeader).getByRole("button"));
    await userEvent.click(within(seqHeader).getByRole("button"));
    const first = within(g).getAllByRole("row")[1];
    expect(first).toHaveTextContent("41036");
  });

  it("hides a column from the Columns menu and remembers it", async () => {
    const { mock } = renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    // In jsdom the resizable panels swallow the pointer-down Radix opens menus on: open it by keyboard.
    act(() => screen.getByRole("button", { name: "Columns" }).focus());
    await userEvent.keyboard("{Enter}");
    await userEvent.click(await screen.findByRole("menuitemcheckbox", { name: "Description" }));
    await waitFor(() => expect(within(g).queryByRole("columnheader", { name: "Description" })).toBeNull());
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "columns")).toBe(true));
  });

  it("shows a peek error with the fix", async () => {
    renderShell("peek-error", seedTab("invoice-generate", "queue", "dead-letter"));
    expect(await screen.findByRole("heading", { name: "Can't read the dead-letter queue" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("an empty view says so in plain words", async () => {
    renderShell("default", seedTab("label-print", "queue", "dead-letter"));
    expect(await screen.findByRole("heading", { name: "No dead letters" })).toBeInTheDocument();
  });

  it("peek again after Peek more reads Top N again, not the grown total", async () => {
    renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    await waitFor(() => expect(status).toHaveTextContent("200 messages"));
    await userEvent.click(screen.getByRole("button", { name: "Peek again" }));
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    await new Promise((r) => setTimeout(r, 50));
    expect(status).toHaveTextContent("100 messages");
  });

  it("starts each test with fresh view state", async () => {
    // Sorting set by an earlier test must not carry over (the view store is module state).
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    expect(within(g).getByRole("columnheader", { name: /Seq/ })).not.toHaveAttribute("aria-sort");
  });

  it("keeps the rows when a later page fails, and Try again continues the round", async () => {
    const { mock } = renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    let peeks = 0;
    let failNext = false;
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "peek_page") {
          peeks++;
          if (failNext) {
            failNext = false;
            throw new EngineError("network", "network error: connection reset by peer");
          }
        }
        return mock.invoke<T>(cmd, args);
      },
    });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    failNext = true;
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByRole("heading", { name: "Couldn't reach the namespace" })).toBeInTheDocument();
    expect(status).toHaveTextContent("100 messages · the next page failed");
    expect(within(await grid()).getAllByRole("row").length).toBeGreaterThan(1);
    const before = peeks;
    await userEvent.click(within(alert).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(status).toHaveTextContent("200 messages"));
    expect(peeks).toBe(before + 1);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
