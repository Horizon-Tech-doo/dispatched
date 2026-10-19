import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { setTransport, type PurgePlans } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "./testing";

const purge = (entities: string[]) => <OpenSheet sheet={{ kind: "operation", op: "purge", entities }} />;

describe("PurgeSheet", () => {
  it("the Dashboard purges the rows picked with their checkboxes", async () => {
    renderShell("operate-ok", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [{ id: "::dashboard", kind: "dashboard", subTab: "overview" }], activeTab: "::dashboard", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
    });
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    const bar = screen.getByRole("toolbar", { name: "Dashboard options" });
    expect(within(bar).getByRole("button", { name: "Purge…" })).toBeDisabled();
    await userEvent.click(await within(grid).findByRole("checkbox", { name: "Select label-print" }));
    await userEvent.click(within(grid).getByRole("checkbox", { name: "Select customs-docs" }));
    await userEvent.click(within(bar).getByRole("button", { name: "Purge 2 selected…" }));
    const sheet = await screen.findByRole("dialog", { name: "Purge messages" });
    expect(within(sheet).getByText("label-print")).toBeInTheDocument();
    expect(within(sheet).getByText("customs-docs")).toBeInTheDocument();
  });

  it("purge of two entities shows two plans and asks twice on production", async () => {
    const { mock } = renderShell("operate-prod", { last_profile: "p-logistics" }, purge(["label-print", "customs-docs"]));
    const sheet = await screen.findByRole("dialog", { name: "Purge messages" });
    // Never one click: counting comes first.
    expect(within(sheet).queryByRole("button", { name: /^Purge/ })).toBeNull();
    await userEvent.click(within(sheet).getByRole("button", { name: "Count messages" }));
    const plans = await within(sheet).findAllByRole("region", { name: /^Plan for / });
    expect(plans.map((p) => p.getAttribute("aria-label"))).toEqual(["Plan for label-print", "Plan for customs-docs"]);
    // The active line and the total.
    expect(within(plans[0]).getAllByText("6 messages")).toHaveLength(2);
    expect(within(sheet).getByText(/Scheduled messages are not purged/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Purge 2 entities" }));
    await waitFor(() => expect(lastCall(mock, "op_purge")).toBeDefined());
    const first = await within(sheet).findByRole("textbox", { name: "Type label-print to confirm" });
    await typeInto(first, "label-print");
    await userEvent.click(within(sheet).getByRole("button", { name: "Purge label-print" }));
    expect(await within(sheet).findByText("Confirmation 2")).toBeInTheDocument();
    const second = within(sheet).getByRole("textbox", { name: "Type customs-docs to confirm" });
    await typeInto(second, "customs-docs");
    await userEvent.click(within(sheet).getByRole("button", { name: "Purge customs-docs" }));
    expect(await within(sheet).findByRole("grid", { name: "Items" })).toBeInTheDocument();
  });

  it("purge plan with an unknown count is refused before confirm", async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, purge(["stock-levels/subscriptions/replenish"]));
    const sheet = await screen.findByRole("dialog", { name: "Purge messages" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Count messages" }));
    expect(await within(sheet).findByText(/count of stock-levels\/subscriptions\/replenish is unknown/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: /^Purge/ })).toBeDisabled();
    expect(mock.calls.some((c) => c.cmd === "op_purge")).toBe(false);
  });

  it('AtLeast(0) deferred reads "an unknown number of deferred messages"', async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, purge(["label-print"]));
    setTransport({
      listen: mock.listen,
      invoke: <T,>(cmd: string, args?: Record<string, unknown>) =>
        (cmd === "purge_plan"
          ? Promise.resolve({
              plan_id: "plan-x",
              plans: [{ entity: "label-print", scope: "all", lines: [{ sub_queue: "active", count: "1,284 messages" }], deferred: "an unknown number of deferred messages", total: "at least 1,284 messages", unknown: false, requires_session: false }],
            } satisfies PurgePlans)
          : mock.invoke(cmd, args)) as Promise<T>,
    });
    const sheet = await screen.findByRole("dialog", { name: "Purge messages" });
    await userEvent.click(within(sheet).getByRole("radio", { name: "All" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Count messages" }));
    const plan = await within(sheet).findByRole("region", { name: "Plan for label-print" });
    expect(within(plan).getByText("an unknown number of deferred messages")).toBeInTheDocument();
    expect(within(plan).getByText("at least 1,284 messages")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Purge label-print" })).toBeEnabled();
  });

  it("changing the scope drops the counts", async () => {
    renderShell("operate-ok", { last_profile: "p-logistics" }, purge(["label-print"]));
    const sheet = await screen.findByRole("dialog", { name: "Purge messages" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Count messages" }));
    await within(sheet).findByRole("region", { name: "Plan for label-print" });
    await userEvent.click(within(sheet).getByRole("radio", { name: "Dead-letter" }));
    expect(within(sheet).queryByRole("region", { name: "Plan for label-print" })).toBeNull();
    expect(within(sheet).getByRole("button", { name: "Count messages" })).toBeInTheDocument();
  });
});
