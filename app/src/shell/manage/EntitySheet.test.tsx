import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { RuleForm, SettingsChange } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "../operate/testing";
import type { OperationSeed } from "../operate/seed";
import { changesOf } from "./fields";
import { manageCommands } from "../palette/commands";

const open = (seed: OperationSeed) => renderShell("operate-ok", { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", ...seed }} />);
type Create = { kind: string; entity: string; settings: SettingsChange; rule: RuleForm | null };

describe("entity sheets", () => {
  it("create_queue_sends_only_set_fields", async () => {
    const { mock } = open({ op: "entity-create", entityKind: "queue", topic: null });
    const sheet = await screen.findByRole("dialog", { name: "New queue" });
    await typeInto(within(sheet).getByRole("textbox", { name: "Name" }), "orders");
    await typeInto(within(sheet).getByRole("textbox", { name: "Lock duration" }), "45s");
    const create = within(sheet).getByRole("button", { name: "Create queue" });
    await waitFor(() => expect(create).toBeEnabled());
    await userEvent.click(create);
    await waitFor(() => expect(lastCall<Create>(mock, "op_entity_create")).toBeDefined());
    expect(lastCall<Create>(mock, "op_entity_create")).toMatchObject({ kind: "queue", entity: "orders", settings: { lock_duration: "45s" }, rule: null });
    expect(Object.keys(lastCall<Create>(mock, "op_entity_create")!.settings)).toEqual(["lock_duration"]);
  });

  it("untouched_duration_field_is_not_sent", async () => {
    const { mock } = open({ op: "entity-edit", entityKind: "queue", entity: "invoice-generate" });
    const sheet = await screen.findByRole("dialog", { name: "Settings of invoice-generate" });
    const ttlNever = await within(sheet).findByRole("checkbox", { name: /^Never expire/ });
    expect(ttlNever).toBeChecked();
    expect(within(sheet).getByRole("textbox", { name: "Lock duration" })).toHaveValue("1m");
    const max = within(sheet).getByRole("textbox", { name: "Max delivery count" });
    await act(async () => max.focus());
    await userEvent.keyboard("{Control>}a{/Control}{Backspace}3");
    const save = within(sheet).getByRole("button", { name: "Save 1 change" });
    await waitFor(() => expect(save).toBeEnabled());
    await userEvent.click(save);
    await waitFor(() => expect(lastCall<{ changes: SettingsChange }>(mock, "op_entity_update")?.changes).toEqual({ max_delivery_count: 3 }));
  });

  it("clearing_forward_sends_empty_string", () => {
    expect(changesOf({ forward_to: "archive", max_delivery_count: 10 }, { forward_to: null, max_delivery_count: 10 })).toEqual({ forward_to: "" });
    expect(changesOf({ user_metadata: "x" }, { user_metadata: "" })).toEqual({ user_metadata: "" });
    expect(changesOf({ lock_duration: "1m" }, { lock_duration: "1m" })).toEqual({});
  });

  it("creation_only_fields_are_read_only_on_edit", async () => {
    open({ op: "entity-edit", entityKind: "queue", entity: "invoice-generate" });
    const sheet = await screen.findByRole("dialog", { name: "Settings of invoice-generate" });
    expect(await within(sheet).findByRole("checkbox", { name: /^Sessions/ })).toBeDisabled();
    expect(within(sheet).getAllByText("Set when the entity was created.").length).toBeGreaterThan(0);
    expect(within(sheet).getByRole("button", { name: "Save 0 changes" })).toBeDisabled();
  });

  it("field_issue_shows_under_its_field_and_blocks_submit", async () => {
    open({ op: "entity-create", entityKind: "queue", topic: null });
    const sheet = await screen.findByRole("dialog", { name: "New queue" });
    await typeInto(within(sheet).getByRole("textbox", { name: "Name" }), "orders");
    await typeInto(within(sheet).getByRole("textbox", { name: "Lock duration" }), "soon");
    expect(await within(sheet).findByText(/'soon' is not a duration/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Create queue" })).toBeDisabled();
  });

  it("subscription_create_with_correlation_rule", async () => {
    const { mock } = open({ op: "entity-create", entityKind: "subscription", topic: "shipment-events" });
    const sheet = await screen.findByRole("dialog", { name: "New subscription on shipment-events" });
    await typeInto(within(sheet).getByRole("textbox", { name: "Name" }), "audit");
    await userEvent.click(within(sheet).getByRole("radio", { name: "Correlation" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Rule name" }), "orders");
    await typeInto(within(sheet).getByRole("textbox", { name: "Subject" }), "order");
    const create = within(sheet).getByRole("button", { name: "Create subscription" });
    await waitFor(() => expect(create).toBeEnabled());
    await userEvent.click(create);
    await waitFor(() => expect(lastCall<Create>(mock, "op_entity_create")).toBeDefined());
    const c = lastCall<Create>(mock, "op_entity_create")!;
    expect(c.entity).toBe("shipment-events/subscriptions/audit");
    expect(c.rule).toMatchObject({ name: "orders", filter: { kind: "correlation", subject: "order" } });
  });

  it("palette_offers_new_and_edit", () => {
    const opened: unknown[] = [];
    const cmds = manageCommands({ id: "shipment-events", kind: "topic" }, { openSheet: (s) => opened.push(s) });
    expect(cmds.map((c) => c.label)).toEqual(expect.arrayContaining(["New subscription on shipment-events…", "Edit settings of shipment-events…"]));
    const all = manageCommands(null, { openSheet: (s) => opened.push(s) });
    expect(all.map((c) => c.label)).toEqual(expect.arrayContaining(["New queue…", "New topic…"]));
  });
});
