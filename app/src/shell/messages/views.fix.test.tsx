import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { setTransport } from "@/engine";
import { renderShell } from "../test-utils";
import { setViewState, viewKey, type ViewState } from "./viewStore";

const seedTab = (id: string, kind: "queue" | "subscription", subTab: string) => ({
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id, kind, subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});
const toolbar = () => screen.getByRole("toolbar", { name: "Peek options" });
async function loaded(name: RegExp) {
  const g = await screen.findByRole("grid", { name });
  await waitFor(() => expect(within(g).getAllByRole("row").length).toBeGreaterThan(1));
  return g;
}

describe("message views after review", () => {
  it("names the selection on Delete, and needs a selection to unschedule or settle", async () => {
    renderShell("operate-ok", seedTab("invoice-generate", "queue", "scheduled"));
    const g = await loaded(/Scheduled messages/);
    const unschedule = within(toolbar()).getByRole("button", { name: "Unschedule" });
    expect(unschedule).toBeDisabled();
    act(() => g.focus());
    await userEvent.keyboard("{ArrowDown}");
    expect(within(toolbar()).getByRole("button", { name: "Unschedule 1" })).toBeEnabled();
  });

  it("Delete carries the selection count in its label", async () => {
    renderShell("operate-ok", seedTab("customs-docs", "queue", "messages"));
    const g = await loaded(/Active messages/);
    act(() => g.focus());
    await userEvent.keyboard("{ArrowDown}{Shift>}{ArrowDown}{/Shift}");
    expect(within(toolbar()).getByRole("button", { name: "Delete 2" })).toBeInTheDocument();
  });

  it("the dead-letter reasons toggle is called Reasons, groups are one line, and a pick joins an existing filter", async () => {
    const VIEW: ViewState = { from: { kind: "start" }, topN: 100, filter: { property: "attempt >= 1" }, filterOpen: false, sorting: null, detailOpen: false };
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    setViewState(viewKey("p-logistics", "invoice-generate", "dead-letter"), VIEW, {});
    await screen.findByRole("grid", { name: /Dead-letter queue/ });
    await userEvent.click(within(toolbar()).getByRole("button", { name: "Reasons" }));
    const panel = await screen.findByRole("region", { name: "Dead-letter reasons" });
    const group = await within(panel).findByRole("button", { name: /MaxDeliveryCountExceeded/ });
    expect(group.querySelectorAll("[data-line]")).toHaveLength(1);
    expect(group).not.toHaveAttribute("title");
    await userEvent.click(group);
    expect(await screen.findByRole("button", { name: "Edit filter: attempt >= 1 AND DeadLetterReason = 'MaxDeliveryCountExceeded'" })).toBeInTheDocument();
    expect(within(panel).getByText(/Added to the filter already applied/)).toBeInTheDocument();
  });

  it("Read again says it is reading", async () => {
    const { mock } = renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    let hold = false;
    setTransport({
      listen: mock.listen,
      invoke: <T,>(cmd: string, args?: Record<string, unknown>) => (cmd === "dlq_summary" && hold ? new Promise<T>(() => {}) : mock.invoke<T>(cmd, args)),
    });
    await screen.findByRole("grid", { name: /Dead-letter queue/ });
    await userEvent.click(within(toolbar()).getByRole("button", { name: "Reasons" }));
    const panel = await screen.findByRole("region", { name: "Dead-letter reasons" });
    await within(panel).findByRole("button", { name: /MaxDeliveryCountExceeded/ });
    hold = true;
    fireEvent.click(within(panel).getByRole("button", { name: "Read again" }));
    expect(await within(panel).findByRole("button", { name: "Reading…" })).toBeDisabled();
  });
});

describe("dashboard and tree after review", () => {
  it("row checkboxes are not tab stops, and Purge keeps its width", async () => {
    renderShell("operate-ok", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [{ id: "::dashboard", kind: "dashboard", subTab: "overview" }], activeTab: "::dashboard", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
    });
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    expect(await within(grid).findByRole("checkbox", { name: "Select label-print" })).toHaveAttribute("tabindex", "-1");
    expect(within(screen.getByRole("toolbar", { name: "Dashboard options" })).getByRole("button", { name: "Purge…" }).className).toMatch(/min-w-/);
  });

  it("the tree menu offers operations only on entity rows, grouped, with Session state only for session entities", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await screen.findByRole("tree", { name: "Namespace" });
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /^Queues(,| \(|$)/ }));
    let menu = await screen.findByRole("menu");
    expect(within(menu).queryByRole("menuitem", { name: /Send messages/ })).toBeNull();
    await userEvent.keyboard("{Escape}");
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /^label-print(,| \(|$)/ }));
    menu = await screen.findByRole("menu");
    const items = within(menu).getAllByRole("menuitem").map((m) => m.textContent);
    expect(items).toContain("Unschedule messages in label-print…");
    expect(items.some((i) => i?.startsWith("Session state"))).toBe(false);
    expect(items.indexOf("Purge label-print…")).toBeGreaterThan(items.indexOf("Move messages from label-print…"));
    await userEvent.keyboard("{Escape}");
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /^returns-intake(,| \(|$)/ }));
    menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Session state of returns-intake…" })).toBeInTheDocument();
  });
});
