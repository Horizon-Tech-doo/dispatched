import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { keys, type SelectForm } from "@/engine";
import { jobStore } from "../drawer/activity";
import { finished, itemEvent, report, started } from "../drawer/jobEvents";
import { lastCall } from "../operate/testing";
import { renderShell } from "../test-utils";

const seedTab = (id: string, kind: "queue" | "subscription", subTab: string) => ({
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id, kind, subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

/** The view's grid once its rows have arrived. */
async function loaded(name: RegExp) {
  const g = await screen.findByRole("grid", { name });
  await waitFor(() => expect(within(g).getAllByRole("row").length).toBeGreaterThan(1));
  return g;
}

const toolbar = () => screen.getByRole("toolbar", { name: "Peek options" });

describe("message operations in the views", () => {
  it("delete key opens the delete sheet and deletes nothing", async () => {
    const { mock } = renderShell("operate-ok", seedTab("customs-docs", "queue", "messages"));
    const g = await loaded(/Active messages/);
    act(() => g.focus());
    await userEvent.keyboard("{ArrowDown}{Shift>}{ArrowDown}{/Shift}");
    await userEvent.keyboard("{Delete}");
    const sheet = await screen.findByRole("dialog", { name: "Delete messages" });
    expect(within(sheet).getByText(/2 selected messages/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Delete 2 messages" })).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "op_delete")).toBe(false);
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 messages" }));
    await waitFor(() => expect(lastCall(mock, "op_delete")).toBeDefined());
    expect(lastCall<{ form: SelectForm }>(mock, "op_delete")!.form).toEqual({ entity: "customs-docs", view: "active", selection: { type: "sequences", sequences: ["1", "2"] } });
  });

  it("with nothing selected, Delete names the filter and a limit", async () => {
    const { mock } = renderShell("operate-ok", seedTab("customs-docs", "queue", "messages"));
    await loaded(/Active messages/);
    await userEvent.click(within(toolbar()).getByRole("button", { name: "Delete" }));
    const sheet = await screen.findByRole("dialog", { name: "Delete messages" });
    expect(within(sheet).getByText(/Every message in the active messages/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete matching messages" }));
    await waitFor(() => expect(lastCall(mock, "op_delete")).toBeDefined());
    expect(lastCall<{ form: SelectForm }>(mock, "op_delete")!.form.selection).toEqual({ type: "matching", filter: {}, limit: 112 });
  });

  it("scheduled view shows Cancel scheduled, not Receive", async () => {
    renderShell("operate-ok", seedTab("invoice-generate", "queue", "scheduled"));
    await loaded(/Scheduled messages/);
    expect(within(toolbar()).getByRole("button", { name: "Cancel scheduled" })).toBeInTheDocument();
    expect(within(toolbar()).queryByRole("button", { name: "Receive" })).toBeNull();
  });

  it("active and dead-letter views offer Receive, Resubmit, Move and Delete; deferred offers Settle", async () => {
    renderShell("operate-ok", seedTab("invoice-generate", "queue", "dead-letter"));
    await loaded(/Dead-letter queue/);
    for (const name of ["Receive", "Resubmit", "Move", "Delete", "Summary"]) expect(within(toolbar()).getByRole("button", { name })).toBeInTheDocument();
    // jsdom panels swallow pointer-downs; Radix tabs activate on focus.
    act(() => screen.getByRole("tab", { name: /Deferred/ }).focus());
    await loaded(/Deferred messages/);
    expect(within(toolbar()).getByRole("button", { name: "Settle" })).toBeInTheDocument();
    expect(within(toolbar()).queryByRole("button", { name: "Receive" })).toBeNull();
  });

  it("finished job invalidates source and target views", async () => {
    const { client } = renderShell("operate-ok", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    const source = keys.peek("p-logistics", "invoice-generate", "dead-letter", { kind: "start" }, {}, null);
    const target = keys.peek("p-logistics", "label-print", "active", { kind: "start" }, {}, null);
    const other = keys.peek("p-logistics", "customs-docs", "active", { kind: "start" }, {}, null);
    for (const k of [source, target, other]) client.setQueryData(k, { pages: [], pageParams: [] });
    act(() => jobStore.apply(started("j1", "Move 2 from invoice-generate/$DeadLetterQueue to label-print", null, null, "move")));
    act(() => jobStore.apply(itemEvent("j1", { entity: "invoice-generate", outcome: "succeeded" })));
    expect(client.getQueryData(source)).toBeDefined();
    act(() => jobStore.apply(finished("j1", { report: report({ kind: "move", entity: "invoice-generate", target: "label-print", items: 1, succeeded: 1 }) })));
    await waitFor(() => expect(client.getQueryData(source)).toBeUndefined());
    expect(client.getQueryData(target)).toBeUndefined();
    expect(client.getQueryData(other)).toBeDefined();
  });
});
