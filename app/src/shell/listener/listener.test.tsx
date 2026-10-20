import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { ListenForm, SendForm } from "@/engine";
import { renderShell } from "../test-utils";
import { lastCall, typeInto } from "../operate/testing";

const onListener = (scenario: Parameters<typeof renderShell>[0], id = "label-print") =>
  renderShell(scenario, {
    last_profile: "p-logistics",
    "shell:p-logistics": { tabs: [{ id, kind: "queue", subTab: "listener" }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
  });

async function start(mode: "Watch" | "Receive and complete" | "Receive and delete") {
  const panel = await screen.findByRole("region", { name: "Listener" });
  await userEvent.click(within(panel).getByRole("radio", { name: mode }));
  await userEvent.click(within(panel).getByRole("button", { name: mode === "Watch" ? "Start watching" : "Start listening" }));
  return panel;
}

describe("listener tab", () => {
  it("watch_starts_without_confirmation", async () => {
    const { mock } = onListener("operate-ok");
    const panel = await start("Watch");
    await waitFor(() => expect(lastCall<{ form: ListenForm }>(mock, "op_listen")?.form).toMatchObject({ entity: "label-print", mode: "watch", concurrency: 1 }));
    expect(await within(panel).findByRole("grid", { name: "Messages received" })).toBeInTheDocument();
    await waitFor(() => expect(within(panel).getAllByRole("row").length).toBeGreaterThan(2), { timeout: 3000 });
    expect(within(panel).queryByRole("group", { name: "Confirmation" })).toBeNull();
  });

  it("delete_mode_confirms_inline_on_production", async () => {
    onListener("operate-prod");
    const panel = await start("Receive and delete");
    const field = await within(panel).findByRole("textbox", { name: /Type label-print to confirm/ });
    expect(within(panel).getByText(/Remove every message that arrives on label-print until stopped/)).toBeInTheDocument();
    await typeInto(field, "label-print");
    await userEvent.click(within(panel).getByRole("button", { name: "Start listening to label-print" }));
    await waitFor(() => expect(within(panel).getByRole("button", { name: "Stop" })).toBeInTheDocument());
  });

  it("stop_cancels_the_job_and_keeps_rows", async () => {
    const { mock } = onListener("operate-ok");
    const panel = await start("Watch");
    await waitFor(() => expect(within(panel).getAllByRole("row").length).toBeGreaterThan(2), { timeout: 3000 });
    await userEvent.click(within(panel).getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "job_cancel")).toBe(true));
    expect(await within(panel).findByRole("button", { name: "New listener" })).toBeInTheDocument();
    expect(within(panel).getAllByRole("row").length).toBeGreaterThan(2);
  });

  it("closing_the_tab_stops_its_listener", async () => {
    const { mock } = onListener("operate-ok");
    await start("Watch");
    await waitFor(() => expect(lastCall(mock, "op_listen")).toBeDefined());
    await userEvent.click(screen.getByRole("button", { name: /^Close label-print/ }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "job_cancel")).toBe(true));
  });

  it("delete_mode_is_not_remembered", async () => {
    onListener("operate-ok");
    const panel = await screen.findByRole("region", { name: "Listener" });
    await userEvent.click(within(panel).getByRole("radio", { name: "Receive and delete" }));
    // Away to another sub-tab and back.
    // In jsdom the resizable panels swallow pointer-downs: switch sub-tabs by keyboard.
    act(() => screen.getByRole("tab", { name: /^Overview/ }).focus());
    await userEvent.keyboard("{Enter}");
    act(() => screen.getByRole("tab", { name: /^Listener/ }).focus());
    await userEvent.keyboard("{Enter}");
    const again = await screen.findByRole("region", { name: "Listener" });
    expect(within(again).getByRole("radio", { name: "Watch" })).toBeChecked();
  });

  it("send_again_clones_the_held_message", async () => {
    const { mock } = onListener("operate-ok");
    const panel = await start("Receive and complete");
    await userEvent.click(await within(panel).findByRole("button", { name: "Start listening to label-print" }));
    await waitFor(() => expect(within(panel).getAllByRole("row").length).toBeGreaterThan(2), { timeout: 3000 });
    act(() => within(panel).getByRole("grid", { name: "Messages received" }).focus());
    await userEvent.keyboard("{ArrowDown}");
    const again = within(panel).getByRole("button", { name: "Send again…" });
    await waitFor(() => expect(again).toBeEnabled());
    await act(async () => again.click());
    const sheet = await screen.findByRole("dialog", { name: "Send a copy" });
    expect(within(sheet).getByDisplayValue("order-created")).toBeInTheDocument();
    expect(lastCall<{ form: SendForm }>(mock, "op_send")).toBeUndefined();
  });

  it("a running dead-letter watch keeps its sub-queue after a visit to another sub-tab", async () => {
    onListener("operate-ok");
    const panel = await screen.findByRole("region", { name: "Listener" });
    await userEvent.click(within(panel).getByRole("radio", { name: "Dead-letter" }));
    await userEvent.click(within(panel).getByRole("button", { name: "Start watching" }));
    await waitFor(() => expect(within(panel).getAllByRole("row").length).toBeGreaterThan(2), { timeout: 3000 });
    act(() => screen.getByRole("tab", { name: /^Overview/ }).focus());
    await userEvent.keyboard("{Enter}");
    act(() => screen.getByRole("tab", { name: /^Listener/ }).focus());
    await userEvent.keyboard("{Enter}");
    const again = await screen.findByRole("region", { name: "Listener" });
    act(() => within(again).getByRole("grid", { name: "Messages received" }).focus());
    await userEvent.keyboard("{ArrowDown}");
    expect(await within(again).findByText("dead-letter queue")).toBeInTheDocument();
  });
});
