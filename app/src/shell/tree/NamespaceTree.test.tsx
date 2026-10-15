import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { useShell } from "../context";
import { renderShell } from "../test-utils";

const tree = () => screen.findByRole("tree", { name: "Namespace" });

// In jsdom every element is a 0x0 box at the origin, so the resizable panels treat any pointer-down as a drag on
// their handle and swallow it: clicks never move focus. Focus explicitly where a test types or presses keys.
const focus = (el: HTMLElement) => act(() => el.focus());

describe("NamespaceTree", () => {
  it("lists queues and topics with counts, amber only on dead letters", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: "invoice-generate" });
    expect(within(row).getByText("1,284").className).not.toContain("text-accent");
    expect(within(row).getByText("37").className).toContain("text-accent");
  });

  it("renders a dead-letter count only when it is above zero, and keeps a zero active count", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const none = await within(t).findByRole("treeitem", { name: "customs-docs" });
    expect(within(none).getByText("112")).toBeInTheDocument();
    expect(within(none).queryByText("0")).not.toBeInTheDocument();
    const dead = within(t).getByRole("treeitem", { name: "returns-intake" });
    expect(within(dead).getByText("0").className).toContain("text-muted");
    expect(within(dead).getByText("211").className).toContain("text-accent");
  });

  it("counts are 12px and flush right, group rows are semibold muted", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: "invoice-generate" });
    expect(within(row).getByText("1,284").className).toContain("text-12");
    expect(within(row).getByText("37").className).not.toContain("min-w");
    const group = within(t).getByRole("treeitem", { name: "Queues" });
    const label = within(group).getByText("Queues");
    expect(label.className).toContain("font-semibold");
    expect(label.className).toContain("text-muted");
  });

  it("marks a production namespace with plain amber text, not the filled badge", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const ns = await within(t).findByRole("treeitem", { name: "sb-logistics-prod" });
    const prod = within(ns).getByText("PROD");
    expect(prod.className).toContain("text-accent");
    expect(prod.className).not.toContain("bg-accent");
  });

  it("has no PROD mark on a non-production namespace", async () => {
    renderShell("default", { last_profile: "p-logistics-dev" });
    const t = await tree();
    const ns = await within(t).findByRole("treeitem", { name: "sb-logistics-dev" });
    expect(within(ns).queryByText("PROD")).not.toBeInTheDocument();
  });

  it("loads subscriptions when a topic expands", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: "shipment-events" });
    await userEvent.click(row);
    await focus(t);
    await userEvent.keyboard("{ArrowRight}");
    expect(await within(t).findByRole("treeitem", { name: "tracking-sync" })).toBeInTheDocument();
    expect(mock.calls).toContainEqual({ cmd: "list_subscriptions", args: { topic: "shipment-events" } });
  });

  it("does not load subscriptions of collapsed topics", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await within(await tree()).findByRole("treeitem", { name: "shipment-events" });
    expect(mock.calls.filter((c) => c.cmd === "list_subscriptions")).toEqual([]);
  });

  it("selects a row on click", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const row = await within(await tree()).findByRole("treeitem", { name: "label-print" });
    await userEvent.click(row);
    expect(row).toHaveAttribute("aria-selected", "true");
  });

  it("filters across loaded entities", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await within(t).findByRole("treeitem", { name: "label-print" });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await userEvent.keyboard("LABEL");
    await waitFor(() => expect(within(t).queryByRole("treeitem", { name: "invoice-generate" })).not.toBeInTheDocument());
    expect(within(t).getByRole("treeitem", { name: "label-print" })).toBeInTheDocument();
  });

  it("says so when nothing matches the filter", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await within(await tree()).findByRole("treeitem", { name: "label-print" });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await userEvent.keyboard("zzz");
    expect(await screen.findByRole("heading", { name: "No matches" })).toBeInTheDocument();
  });

  it("explains a limited listing and shows pinned entities", async () => {
    renderShell("forbidden", { last_profile: "p-logistics" });
    expect(await screen.findByText(/This identity can't list entities in sb-logistics-prod, so only pinned entities are shown\./)).toBeInTheDocument();
    expect(screen.getByText(/Grant it Manage rights to see everything, or edit this connection to pin more\./)).toBeInTheDocument();
    expect(screen.queryByText(/needs Manage,EntityRead/)).not.toBeInTheDocument(); // the engine's wording is not echoed
    expect(within(await tree()).getByRole("treeitem", { name: "orders" })).toBeInTheDocument();
  });

  it("shows pinned subscriptions by name with their topic beside it", async () => {
    renderShell("forbidden", { last_profile: "p-logistics" });
    const row = await within(await tree()).findByRole("treeitem", { name: "tracking-sync" });
    expect(within(row).getByText("shipment-events").className).toContain("text-muted");
  });

  it("offers to edit the connection from the limited-listing notice", async () => {
    const user = userEvent.setup();
    function SheetProbe() {
      const { sheet } = useShell();
      return <output aria-label="Open sheet">{sheet ? JSON.stringify(sheet) : "none"}</output>;
    }
    renderShell("forbidden", { last_profile: "p-logistics" }, <SheetProbe />);
    await user.click(await screen.findByRole("button", { name: "Edit connection" }));
    expect(screen.getByLabelText("Open sheet")).toHaveTextContent('{"kind":"profile","profileId":"p-logistics"}');
  });

  it("shows a retryable error when listing fails", async () => {
    renderShell("network", { last_profile: "p-logistics" });
    // One automatic retry for network errors, after the query client's default delay.
    expect(await screen.findByRole("heading", { name: "Can't list this namespace" }, { timeout: 4000 })).toBeInTheDocument();
    expect(screen.getByText("The connection to sb-logistics-prod was interrupted. Check your network or VPN, then retry.")).toBeInTheDocument();
    // The drawer logs the same failure; the tree's own line is the one outside it.
    const raw = screen.getAllByText("network error: connection reset by peer").find((el) => !el.closest("[aria-label='Jobs and activity']"))!;
    expect(raw.className).toContain("text-11");
    expect(raw.className).toContain("text-muted");
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("opens the context menu from the keyboard on the selected row", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await user.click(await within(t).findByRole("treeitem", { name: "label-print" }));
    fireEvent.contextMenu(t, { button: 0 });
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Copy entity path" })).not.toHaveAttribute("aria-disabled", "true");
    await user.click(within(menu).getByRole("menuitem", { name: "Copy entity path" }));
    expect(await navigator.clipboard.readText()).toBe("label-print");
  });

  it("lets a right-click on empty space act on nothing", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await user.click(await within(t).findByRole("treeitem", { name: "label-print" }));
    fireEvent.contextMenu(t, { button: 2 });
    expect(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Open" })).toHaveAttribute("aria-disabled", "true");
  });

  it("keeps loaded subscriptions in the tree when a topic is filtered by its name", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await user.click(await within(t).findByRole("treeitem", { name: "shipment-events" }));
    await focus(t);
    await user.keyboard("{ArrowRight}");
    await within(t).findByRole("treeitem", { name: "tracking-sync" });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await user.keyboard("shipment");
    expect(within(t).getByRole("treeitem", { name: "billing" })).toBeInTheDocument();
    expect(within(t).getByRole("treeitem", { name: "warehouse-notify" })).toBeInTheDocument();
  });

  it("offers open, copy and refresh in the context menu", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const row = await within(await tree()).findByRole("treeitem", { name: "invoice-generate" });
    fireEvent.contextMenu(row);
    const menu = await screen.findByRole("menu");
    expect(row).toHaveAttribute("aria-selected", "true");
    expect(within(menu).getByRole("menuitem", { name: "Open" })).toBeEnabled();
    await user.click(within(menu).getByRole("menuitem", { name: "Copy entity path" }));
    expect(await navigator.clipboard.readText()).toBe("invoice-generate");
    fireEvent.contextMenu(row);
    const before = mock.calls.filter((c) => c.cmd === "list_entities").length;
    await user.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Refresh" }));
    await waitFor(() => expect(mock.calls.filter((c) => c.cmd === "list_entities").length).toBe(before + 1));
  });

  it("disables entity actions on a group row", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const group = await within(await tree()).findByRole("treeitem", { name: "Queues" });
    fireEvent.contextMenu(group);
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Open" })).toHaveAttribute("aria-disabled", "true");
    expect(within(menu).getByRole("menuitem", { name: "Refresh" })).not.toHaveAttribute("aria-disabled", "true");
  });
});
