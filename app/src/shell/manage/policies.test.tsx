import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { PolicyChange } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "../operate/testing";
import type { OperationSeed } from "../operate/seed";

const open = (seed: OperationSeed) => renderShell("operate-ok", { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", ...seed }} />);
const overview = (id: string) =>
  renderShell("default", {
    last_profile: "p-logistics",
    "shell:p-logistics": { tabs: [{ id, kind: "queue", subTab: "overview" }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
  });

describe("shared access policies", () => {
  it("policies_never_render_keys", async () => {
    overview("invoice-generate");
    const section = await screen.findByRole("region", { name: "Shared access policies" });
    const row = await within(section).findByRole("row", { name: /billing-app/ });
    expect(within(row).getByText("Listen, Send")).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(/[A-Za-z0-9+/]{43}=/);
    expect(within(section).getByRole("button", { name: "Add policy…" })).toBeInTheDocument();
  });

  it("manage_right_locks_listen_and_send", async () => {
    const { mock } = open({ op: "policy-add", entityKind: "queue", entity: "invoice-generate" });
    const sheet = await screen.findByRole("dialog", { name: "Add policy" });
    await typeInto(within(sheet).getByRole("textbox", { name: "Name" }), "ops-2");
    await userEvent.click(within(sheet).getByRole("checkbox", { name: /^Manage/ }));
    expect(within(sheet).getByRole("checkbox", { name: /^Listen/ })).toBeChecked();
    expect(within(sheet).getByRole("checkbox", { name: /^Listen/ })).toBeDisabled();
    expect(within(sheet).getByRole("checkbox", { name: /^Send/ })).toBeChecked();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add policy" }));
    await waitFor(() => expect(lastCall<{ change: PolicyChange }>(mock, "op_policy_change")?.change).toEqual({ type: "add", rights: ["listen", "send", "manage"] }));
  });

  it("regenerate_goes_through_confirmation", async () => {
    const { mock } = open({ op: "policy-change", entityKind: "queue", entity: "invoice-generate", name: "billing-app", change: { type: "renew", slot: "primary" } });
    const sheet = await screen.findByRole("dialog", { name: "Regenerate primary key of billing-app" });
    expect(within(sheet).getByText(/Clients using it stop working/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Regenerate primary key" }));
    await waitFor(() => expect(lastCall<{ change: PolicyChange }>(mock, "op_policy_change")?.change).toEqual({ type: "renew", slot: "primary" }));
    expect(await within(sheet).findByRole("button", { name: /Regenerate|Confirm/ })).toBeInTheDocument();
  });
});
