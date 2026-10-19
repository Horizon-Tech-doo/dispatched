import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";

const seedTab = (id: string, subTab: string) => ({
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id, kind: "queue", subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

describe("DlqSummary", () => {
  it("dlq summary group click filters the view", async () => {
    renderShell("default", seedTab("invoice-generate", "dead-letter"));
    await screen.findByRole("grid", { name: /Dead-letter queue/ });
    const bar = screen.getByRole("toolbar", { name: "Peek options" });
    const toggle = within(bar).getByRole("button", { name: "Summary" });
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    const summary = await screen.findByRole("region", { name: "Dead-letter reasons" });
    await waitFor(() => expect(within(summary).getAllByRole("button").length).toBeGreaterThan(1));
    expect(within(summary).getByText(/37 messages examined/)).toBeInTheDocument();
    const group = within(summary).getByRole("button", { name: /MaxDeliveryCountExceeded/ });
    const count = Number(within(group).getByText(/^\d[\d,]*$/).textContent!.replace(/,/g, ""));
    await userEvent.click(group);
    expect(await screen.findByRole("button", { name: "Edit filter: DeadLetterReason = 'MaxDeliveryCountExceeded'" })).toBeInTheDocument();
    const status = screen.getByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent(`${count} matching messages`));
  });
});
