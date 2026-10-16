import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { setTransport, type DashboardRow, type DashboardView } from "@/engine";
import { activityStore } from "../drawer/activity";
import { renderShell } from "../test-utils";
import { toTsv } from "./tsv";

const seed = {
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id: "::dashboard", kind: "dashboard", subTab: "overview" }], activeTab: "::dashboard", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
};
const focus = (el: HTMLElement) => act(() => el.focus());

describe("dashboard", () => {
  it("sorts by dead letters first and marks them in amber", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await waitFor(() => expect(within(grid).getAllByRole("row")[1]).toHaveTextContent("tracking-sync"));
    expect(within(within(grid).getAllByRole("row")[1]).getByText("3,417").className).toContain("text-accent");
  });

  it("shows counts as data: nonzero in the primary colour, zero muted, no row tint", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    const row = await within(grid).findByRole("row", { name: /invoice-generate/ });
    expect(within(row).getByText("1,284").className).toContain("text-fg");
    expect(within(row).getByText("37").className).toContain("text-accent");
    // Zero counts stay quiet, and a row with dead letters is not tinted.
    expect(within(row).getAllByText("0")[0].className).toContain("text-muted");
    expect(row.className).not.toMatch(/accent/);
  });

  it("numbers the rows", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("invoice-generate");
    expect(within(grid).getAllByRole("columnheader")[0]).toHaveTextContent("#");
  });

  it("keeps only rows with dead letters when asked", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("customs-docs");
    await userEvent.click(screen.getByRole("button", { name: "Only with dead letters" }));
    expect(within(grid).queryByText("customs-docs")).toBeNull();
    expect(within(grid).getByText("invoice-generate")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Only with dead letters" })).toHaveAttribute("aria-pressed", "true");
  });

  it("filters by name or topic", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("customs-docs");
    await focus(screen.getByRole("searchbox", { name: "Filter dashboard" })); // the panels swallow the pointer-down a click starts with
    await userEvent.keyboard("shipment");
    expect(within(grid).queryByText("customs-docs")).toBeNull();
    expect(within(grid).getByText("tracking-sync")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}zzzz");
    expect(await screen.findByText("No match")).toBeInTheDocument();
  });

  it("says in the status line what is shown and when it was read", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("customs-docs");
    const status = screen.getByRole("status", { name: "Dashboard status" });
    expect(status).toHaveTextContent(/\d+ entities/);
    expect(status).toHaveTextContent(/\d+ with dead letters/);
    expect(status).toHaveTextContent(/Refreshed \d{2}:\d{2}:\d{2}/);
    await userEvent.click(screen.getByRole("button", { name: "Only with dead letters" }));
    expect(status).toHaveTextContent(/^\d+ of \d+ entities/);
  });

  it("copies the rows as TSV", async () => {
    const user = userEvent.setup(); // installs the clipboard stub
    renderShell("default", seed);
    await screen.findByRole("grid", { name: "Dashboard" });
    await user.click(await screen.findByRole("button", { name: "Copy as TSV" }));
    const text = await navigator.clipboard.readText();
    expect(await screen.findByText("Copied")).toBeInTheDocument();
    expect(text.split("\n")[0]).toBe("Entity\tTopic\tKind\tStatus\tActive\tDead-letter\tScheduled\tTransfer\tTransfer DLQ");
    expect(text).toContain("invoice-generate\t\tqueue\tActive\t1284\t37\t0\t0\t0");
  });

  it("opens an entity from its row", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await userEvent.dblClick(await within(grid).findByText("label-print"));
    expect(await screen.findByRole("heading", { level: 1, name: "label-print" })).toBeInTheDocument();
  });

  it("opens the focused row with Enter", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await userEvent.click(await within(grid).findByText("label-print"));
    await focus(grid);
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { level: 1, name: "label-print" })).toBeInTheDocument();
  });

  it("opens from the palette", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(screen.getByPlaceholderText(/Go to entity or command/), "dashboard{Enter}");
    expect(await screen.findByRole("grid", { name: "Dashboard" })).toBeInTheDocument();
    // The dashboard is not an entity: the tree selects nothing for it.
    expect(screen.getAllByRole("treeitem").some((t) => t.getAttribute("aria-selected") === "true")).toBe(false);
  });

  it("opens with mod+shift+d", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}{Shift>}d{/Shift}{/Control}");
    expect(await screen.findByRole("grid", { name: "Dashboard" })).toBeInTheDocument();
  });

  it("opens from the namespace row, with Enter and from its menu", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const tree = await screen.findByRole("tree", { name: "Namespace" });
    const ns = within(tree).getAllByRole("treeitem")[0];
    await userEvent.click(ns);
    await focus(tree);
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("grid", { name: "Dashboard" })).toBeInTheDocument();
    await userEvent.keyboard("{Control>}w{/Control}");
    await waitFor(() => expect(screen.queryByRole("grid", { name: "Dashboard" })).toBeNull());
    fireEvent.contextMenu(within(tree).getAllByRole("treeitem")[0]);
    await userEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Open dashboard" }));
    expect(await screen.findByRole("grid", { name: "Dashboard" })).toBeInTheDocument();
  });

  it("shows the tab as Dashboard, without a kind word", async () => {
    renderShell("default", seed);
    const strip = await screen.findByRole("tablist", { name: "Open entities" });
    const tab = within(strip).getByRole("tab", { name: "Dashboard" });
    expect(tab).toHaveAttribute("aria-selected", "true");
    expect(within(strip).getByRole("button", { name: "Close Dashboard" })).toBeInTheDocument();
  });

  it("refreshes with the button and F5 without logging anything unless a read fails", async () => {
    // Auto-refresh calls `dashboard` on a timer: Rust and the mock log it only on failure.
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("invoice-generate");
    await userEvent.click(screen.getByRole("button", { name: "Refresh the dashboard" }));
    await userEvent.keyboard("{F5}");
    expect(activityStore.all().some((e) => /dashboard/i.test(e.action))).toBe(false);
  });

  it("picks the auto-refresh interval and remembers it", async () => {
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid", { name: "Dashboard" });
    const group = screen.getByRole("radiogroup", { name: "Auto-refresh" });
    expect(within(group).getByRole("radio", { name: "Off" })).toBeChecked();
    await userEvent.click(within(group).getByRole("radio", { name: "1 min" }));
    expect(within(group).getByRole("radio", { name: "1 min" })).toBeChecked();
    const saved = await mock.invoke<{ dashboardRefresh?: number }>("settings_get", { key: "ui" });
    expect(saved.dashboardRefresh).toBe(60);
  });

  it("shows unknown counts as ?", async () => {
    renderShell("counts-unknown", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    const row = await within(grid).findByRole("row", { name: /invoice-generate/ });
    expect(within(row).getAllByText("?").length).toBeGreaterThanOrEqual(5);
  });

  it("explains a limited view and does not name the kind of an entity it only knows", async () => {
    renderShell("forbidden", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    const main = within(screen.getByRole("main"));
    expect(await main.findByText("Showing pinned entities only")).toBeInTheDocument();
    expect(main.getByText(/This identity can't list the namespace/)).toBeInTheDocument();
    // Rows for entities only known by name carry no queue glyph: one of them can be a topic.
    const row = await within(grid).findByRole("row", { name: /orders/ });
    expect(row.querySelector("svg")).toBeNull();
    expect(within(row).getAllByText("?").length).toBeGreaterThan(0);
  });

  it("lists the topics it could not read", async () => {
    const { mock } = renderShell("default", seed);
    const failing: DashboardView = {
      rows: [{ kind: "queue", id: "orders", name: "orders", topic: null, status: "Active", counts: null }],
      failures: [{ topic: "shipment-events", kind: "forbidden", message: "forbidden: 401 on shipment-events" }, { topic: "audit", kind: "network", message: "network error" }],
      limited: null,
    };
    setTransport({ listen: mock.listen, invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => (cmd === "dashboard" ? (failing as T) : mock.invoke<T>(cmd, args)) });
    await userEvent.click(await screen.findByRole("button", { name: "Refresh the dashboard" }));
    expect(await screen.findByText("Couldn't list the subscriptions of 2 topics")).toBeInTheDocument();
    expect(screen.getByText("shipment-events, audit")).toBeInTheDocument();
  });

  it("says so when the dashboard can't be loaded", async () => {
    renderShell("network", seed);
    expect(await screen.findByText("Couldn't load the dashboard", {}, { timeout: 4000 })).toBeInTheDocument();
    expect(within(screen.getByRole("main")).getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });
});

describe("toTsv", () => {
  it("leaves unknown counts empty and flattens tabs and newlines", () => {
    const rows: DashboardRow[] = [{ kind: "subscription", id: "t/subscriptions/s", name: "s\tx", topic: "t", status: null, counts: null }];
    expect(toTsv(rows).split("\n")[1]).toBe("s x\tt\tsubscription\t\t\t\t\t\t");
  });

  it("leaves the kind empty for entities known only by name", () => {
    const rows: DashboardRow[] = [{ kind: "queue", id: "orders", name: "orders", topic: null, status: null, counts: null }];
    expect(toTsv(rows, { kindsKnown: false }).split("\n")[1]).toBe("orders\t\t\t\t\t\t\t\t");
  });
});
