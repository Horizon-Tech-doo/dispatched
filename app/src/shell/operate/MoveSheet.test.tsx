import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall } from "./testing";

describe("MoveSheet", () => {
  it("move target picker excludes the source and subscriptions", async () => {
    const { mock } = renderShell(
      "operate-ok",
      { last_profile: "p-logistics" },
      <OpenSheet sheet={{ kind: "operation", op: "move", entity: "customs-docs", view: "active", sequences: [1, 2], filter: {} }} />,
    );
    const sheet = await screen.findByRole("dialog", { name: "Move messages" });
    const picker = within(sheet).getByRole("listbox", { name: "Move to" });
    await waitFor(() => expect(within(picker).getAllByRole("option").length).toBeGreaterThan(0));
    const names = within(picker).getAllByRole("option").map((o) => o.textContent);
    expect(names).toEqual(["invoice-generate", "label-print", "returns-intake", "archive-events", "carrier-updates", "shipment-events", "stock-levels"]);
    expect(names).not.toContain("customs-docs");
    expect(within(sheet).getByRole("button", { name: "Move 2 messages" })).toBeDisabled();
    await userEvent.click(within(picker).getByRole("option", { name: "label-print" }));
    expect(within(picker).getByRole("option", { name: "label-print" })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(within(sheet).getByRole("button", { name: "Move 2 messages" }));
    await waitFor(() => expect(lastCall(mock, "op_move")).toBeDefined());
    expect(lastCall(mock, "op_move")).toMatchObject({ target: "label-print", dropUncopyable: false, form: { entity: "customs-docs", selection: { type: "sequences", sequences: ["1", "2"] } } });
  });

  it("a subscription's dead letters can be resubmitted to their topic by default", async () => {
    const { mock } = renderShell(
      "operate-ok",
      { last_profile: "p-logistics" },
      <OpenSheet sheet={{ kind: "operation", op: "resubmit", entity: "carrier-updates/subscriptions/eta-recalc", view: "dead-letter", sequences: [650], filter: {} }} />,
    );
    const sheet = await screen.findByRole("dialog", { name: "Resubmit messages" });
    expect(within(sheet).getByText(/Back to carrier-updates/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Resubmit 1 message" }));
    await waitFor(() => expect(lastCall(mock, "op_resubmit")).toBeDefined());
    expect(lastCall(mock, "op_resubmit")).toMatchObject({
      form: { source: { entity: "carrier-updates/subscriptions/eta-recalc", view: "dead-letter", selection: { type: "sequences", sequences: ["650"] } }, remove_original: true, new_message_id: false },
    });
    expect((lastCall<{ form: { target?: string } }>(mock, "op_resubmit")!.form).target).toBeUndefined();
  });
});
