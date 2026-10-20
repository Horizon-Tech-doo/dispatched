import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { EntityRefForm } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "../operate/testing";
import type { OperationSeed } from "../operate/seed";
import { manageCommands } from "../palette/commands";

const open = (seed: OperationSeed, scenario: Parameters<typeof renderShell>[0] = "operate-ok") =>
  renderShell(scenario, { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", ...seed }} />);

describe("status, copy and delete sheets", () => {
  it("status_offers_kind_statuses", async () => {
    open({ op: "entity-status", target: { kind: "topic", entity: "shipment-events" } });
    const sheet = await screen.findByRole("dialog", { name: "Status of shipment-events" });
    expect(within(sheet).getByRole("radio", { name: /^Send disabled/ })).toBeInTheDocument();
    expect(within(sheet).queryByRole("radio", { name: /^Receive disabled/ })).toBeNull();
    // The current status is chosen, so there is nothing to save yet.
    expect(await within(sheet).findByRole("radio", { name: /^Active/ })).toBeChecked();
    expect(within(sheet).getByRole("button", { name: "Set status" })).toBeDisabled();
  });

  it("rename_refusal_shows_core_text", async () => {
    open({ op: "entity-rename", target: { kind: "queue", entity: "invoice-generate" } });
    const sheet = await screen.findByRole("dialog", { name: "Rename invoice-generate" });
    const name = within(sheet).getByRole("textbox", { name: "New name" });
    expect(name).toHaveValue("invoice-generate-copy");
    await userEvent.click(within(sheet).getByRole("button", { name: "Rename queue" }));
    expect(await within(sheet).findByText(/holds about 1,284 messages/)).toBeInTheDocument();
    expect(within(sheet).getByRole("textbox", { name: "New name" })).toBeInTheDocument();
  });

  it("bulk_delete_from_dashboard", async () => {
    const { mock } = open({ op: "entity-delete", entities: [{ kind: "queue", entity: "label-print" }, { kind: "queue", entity: "customs-docs" }] }, "operate-prod");
    const sheet = await screen.findByRole("dialog", { name: "Delete 2 entities" });
    expect(within(sheet).getByText("label-print")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 entities" }));
    for (const target of ["label-print", "customs-docs"]) {
      const field = await within(sheet).findByRole("textbox", { name: new RegExp(`Type ${target} to confirm`) });
      await typeInto(field, target);
      await userEvent.click(within(sheet).getByRole("button", { name: /^(Delete|Confirm)/ }));
    }
    await waitFor(() => expect(lastCall<{ entities: EntityRefForm[] }>(mock, "op_entity_delete")?.entities).toHaveLength(2));
    expect(await within(sheet).findByText("Deleted 2 entities")).toBeInTheDocument();
    expect(within(sheet).getByRole("columnheader", { name: "Entity" })).toBeInTheDocument();
  });

  it("duplicate_not_offered_for_topics", () => {
    const ids = (kind: "queue" | "topic" | "subscription") => manageCommands({ id: "x", kind }, { openSheet: () => {} }).map((c) => c.id);
    expect(ids("topic")).not.toContain("manage:duplicate");
    expect(ids("topic")).toContain("manage:rename");
    expect(ids("queue")).toEqual(expect.arrayContaining(["manage:duplicate", "manage:rename", "manage:status", "manage:delete"]));
    expect(ids("subscription")).toContain("manage:duplicate");
    expect(ids("subscription")).not.toContain("manage:rename");
  });

  it("the dashboard offers deleting the selected entities", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "::dashboard", kind: "dashboard", subTab: "overview" }], activeTab: "::dashboard", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } } });
    expect(await screen.findByRole("button", { name: "Delete…" })).toBeDisabled();
  });
});
