import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { RuleForm } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "../operate/testing";
import type { OperationSeed } from "../operate/seed";
import { ruleKind } from "../rules/RulesView";

const SUB = "shipment-events/subscriptions/billing";
const open = (seed: OperationSeed) => renderShell("operate-ok", { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", ...seed }} />);

describe("rule editing", () => {
  it("true_filter_renders_as_default", () => {
    expect(ruleKind({ kind: "true" })).toBe("Matches every message (default rule)");
  });

  it("add_correlation_rule_payload", async () => {
    const { mock } = open({ op: "rule-save", entity: SUB, rule: null });
    const sheet = await screen.findByRole("dialog", { name: "Add rule" });
    await typeInto(within(sheet).getByRole("textbox", { name: "Name" }), "eu");
    await userEvent.click(within(sheet).getByRole("radio", { name: "Correlation" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Subject" }), "order");
    await userEvent.click(within(sheet).getByRole("button", { name: "Add property" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Name of property 1" }), "region");
    await typeInto(within(sheet).getByRole("textbox", { name: "Value of property 1" }), "eu");
    const add = within(sheet).getByRole("button", { name: "Add rule" });
    await waitFor(() => expect(add).toBeEnabled());
    await userEvent.click(add);
    await waitFor(() => expect(lastCall<{ rule: RuleForm; update: boolean }>(mock, "op_rule_save")).toBeDefined());
    const call = lastCall<{ rule: RuleForm; update: boolean }>(mock, "op_rule_save")!;
    expect(call.update).toBe(false);
    expect(call.rule).toMatchObject({ name: "eu", filter: { kind: "correlation", subject: "order", properties: [{ name: "region", value: "eu" }] } });
  });

  it("a pristine rule form shows no error, only a disabled Add rule", async () => {
    open({ op: "rule-save", entity: SUB, rule: null });
    const sheet = await screen.findByRole("dialog", { name: "Add rule" });
    await new Promise((r) => setTimeout(r, 400));
    expect(within(sheet).queryByText(/needs a name|needs an expression/)).toBeNull();
    expect(within(sheet).getByRole("button", { name: "Add rule" })).toBeDisabled();
  });

  it("empty_correlation_shows_794_and_blocks", async () => {
    open({ op: "rule-save", entity: SUB, rule: null });
    const sheet = await screen.findByRole("dialog", { name: "Add rule" });
    await typeInto(within(sheet).getByRole("textbox", { name: "Name" }), "c");
    await userEvent.click(within(sheet).getByRole("radio", { name: "Correlation" }));
    expect(await within(sheet).findByText(/matches no message/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Add rule" })).toBeDisabled();
  });

  it("edit_rule_prefills_and_sends_update", async () => {
    const { mock } = open({
      op: "rule-save",
      entity: "shipment-events/subscriptions/tracking-sync",
      rule: { name: "eu-only", filter: { kind: "sql", expression: "region = 'eu'" }, action: "SET priority = 'high'", created_at: null },
    });
    const sheet = await screen.findByRole("dialog", { name: "Edit rule eu-only" });
    const sql = within(sheet).getByRole("textbox", { name: "SQL filter" });
    expect(sql).toHaveValue("region = 'eu'");
    expect(within(sheet).getByRole("textbox", { name: "Name" })).toHaveAttribute("readonly");
    await typeInto(sql, "{End} AND tier = 2");
    const save = within(sheet).getByRole("button", { name: "Save rule" });
    await waitFor(() => expect(save).toBeEnabled());
    await userEvent.click(save);
    await waitFor(() => expect(lastCall<{ rule: RuleForm; update: boolean }>(mock, "op_rule_save")?.update).toBe(true));
    expect(lastCall<{ rule: RuleForm }>(mock, "op_rule_save")!.rule.filter).toEqual({ kind: "sql", expression: "region = 'eu' AND tier = 2" });
  });

  it("remove_last_rule_warns", async () => {
    open({ op: "rule-remove", entity: SUB, name: "$Default", last: true });
    const sheet = await screen.findByRole("dialog", { name: "Remove rule $Default" });
    expect(within(sheet).getByText("Without a rule this subscription receives no messages.")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Remove rule" })).toBeEnabled();
  });

  it("the rules tab offers add, edit and remove", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [{ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "rules" }], activeTab: "shipment-events/subscriptions/tracking-sync", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
    });
    await screen.findByRole("list", { name: "Rules" });
    expect(screen.getByRole("button", { name: "Add rule…" })).toBeInTheDocument();
    // In jsdom the resizable panels swallow the pointer-down Radix opens menus on: open it by keyboard.
    act(() => screen.getByRole("button", { name: "Actions for rule eu-only" }).focus());
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("menuitem", { name: "Edit…" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Remove…" })).toBeInTheDocument();
  });
});
