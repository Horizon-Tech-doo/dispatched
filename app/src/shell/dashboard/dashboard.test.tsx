import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { setTransport, type DashboardRow, type DashboardView, type EntityCounts } from "@/engine";
import { activityStore } from "../drawer/activity";
import { renderShell } from "../test-utils";
import { sortRows } from "./sort";
import { toTsv } from "./tsv";

const seed = {
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id: "::dashboard", kind: "dashboard", subTab: "overview" }], activeTab: "::dashboard", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
};
const focus = (el: HTMLElement) => act(() => el.focus());

const counts = (active: number, dead_letter: number, extra: Partial<EntityCounts> = {}): EntityCounts => ({
  active, dead_letter, scheduled: 0, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false, ...extra,
});
const queue = (name: string, c: EntityCounts | null): DashboardRow => ({ kind: "queue", id: name, name, topic: null, status: "Active", counts: c });
const view = (rows: DashboardRow[], extra: Partial<DashboardView> = {}): DashboardView => ({ rows, failures: [], limited: null, ...extra });
/** Answer the dashboard with `v` from now on; F5 then reads it. */
async function serve(mock: { listen: Parameters<typeof setTransport>[0]["listen"]; invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> }, v: DashboardView) {
  setTransport({ listen: mock.listen, invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => (cmd === "dashboard" ? (v as T) : mock.invoke<T>(cmd, args)) });
  await userEvent.keyboard("{F5}");
}
const dashboardCalls = (mock: { calls: { cmd: string }[] }) => mock.calls.filter((c) => c.cmd === "dashboard").length;

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
    expect(screen.getByRole("radio", { name: "All 10" })).toBeChecked();
    await userEvent.click(screen.getByRole("radio", { name: "With dead letters 5" }));
    expect(within(grid).queryByText("customs-docs")).toBeNull();
    expect(within(grid).getByText("invoice-generate")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "With dead letters 5" })).toBeChecked();
  });

  it("counts a transfer dead letter as a dead letter, and says what unknown counts mean when nothing else is left", async () => {
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid", { name: "Dashboard" });
    await serve(mock, view([queue("only-transfer", counts(5, 0, { transfer_dead_letter: 2 })), queue("clean", counts(1, 0)), queue("blind", null)]));
    await userEvent.click(await screen.findByRole("radio", { name: "With dead letters 1" }));
    const grid = screen.getByRole("grid", { name: "Dashboard" });
    expect(within(grid).getByText("only-transfer")).toBeInTheDocument();
    expect(within(grid).queryByText("clean")).toBeNull();
    expect(within(grid).queryByText("blind")).toBeNull();
    // The unknown row is not hidden quietly: the status line counts it.
    expect(screen.getByText(/1 entity with unknown counts/)).toBeInTheDocument();
    await serve(mock, view([queue("clean", counts(1, 0)), queue("blind", null)]));
    expect(await screen.findByText("No dead letters found")).toBeInTheDocument();
    expect(screen.getByText(/1 entity has unknown counts/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(await within(screen.getByRole("grid", { name: "Dashboard" })).findByText("blind")).toBeInTheDocument();
  });

  it("says no dead letters anywhere when every count is known", async () => {
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid", { name: "Dashboard" });
    await serve(mock, view([queue("clean", counts(1, 0))]));
    await userEvent.click(await screen.findByRole("radio", { name: /^With dead letters/ }));
    expect(await screen.findByText("No dead letters anywhere")).toBeInTheDocument();
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

  it("says in the status line what is shown, and the clock only while auto-refresh is on", async () => {
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("customs-docs");
    const status = screen.getByText(/^\d+ entities/).parentElement!;
    expect(status).toHaveTextContent(/^10 entities · 1 entity with unknown counts$/);
    expect(status).not.toHaveAttribute("role"); // not a live region: only the copy feedback announces
    await userEvent.click(screen.getByRole("radio", { name: /^With dead letters/ }));
    expect(status).toHaveTextContent(/^5 of 10 entities · 1 entity with unknown counts$/);
    await userEvent.click(screen.getByRole("radio", { name: "30 s" }));
    expect(status).toHaveTextContent(/^5 of 10 entities · 1 entity with unknown counts · Auto-refresh 30 s · Counts read \d{2}:\d{2}:\d{2}$/);
    await userEvent.click(screen.getByRole("radio", { name: "Off" }));
    expect(status).toHaveTextContent(/^5 of 10 entities · 1 entity with unknown counts$/);
  });

  it("copies the rows as TSV", async () => {
    const user = userEvent.setup(); // installs the clipboard stub
    renderShell("default", seed);
    await screen.findByRole("grid", { name: "Dashboard" });
    await user.click(await screen.findByRole("button", { name: "Copy as TSV" }));
    const text = await navigator.clipboard.readText();
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    expect(text.split("\n")[0]).toBe("Entity\tTopic\tKind\tStatus\tActive\tDead-letter\tScheduled\tTransfer\tTransfer DLQ");
    expect(text).toContain("invoice-generate\t\tqueue\tActive\t1284\t37\t3\t0\t0");
  });

  it("copies the rows in the order the grid shows them", async () => {
    const user = userEvent.setup();
    renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("tracking-sync");
    await user.click(screen.getByRole("button", { name: "Copy as TSV" }));
    // Sorted by Dead-letter, descending (the default).
    expect((await navigator.clipboard.readText()).split("\n")[1]).toMatch(/^tracking-sync\tshipment-events\tsubscription\tActive\t2906\t3417\t/);
    // By Active, ascending: the entity whose counts are unknown comes first, and its cells are empty.
    await user.click(within(grid).getByRole("button", { name: "Active" }));
    await waitFor(() => expect(within(grid).getAllByRole("row")[1]).toHaveTextContent("replenish"));
    await user.click(screen.getByRole("button", { name: /^Cop/ }));
    await waitFor(async () => expect((await navigator.clipboard.readText()).split("\n")[1]).toBe("replenish\tstock-levels\tsubscription\tActive\t\t\t\t\t"));
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
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

  it("refreshes with F5, has no refresh button of its own, and logs nothing unless a read fails", async () => {
    // Auto-refresh calls `dashboard` on a timer: Rust and the mock log it only on failure.
    const { mock } = renderShell("default", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    await within(grid).findByText("invoice-generate");
    expect(screen.queryByRole("button", { name: "Refresh the dashboard" })).toBeNull();
    const before = dashboardCalls(mock);
    await userEvent.keyboard("{F5}");
    await waitFor(() => expect(dashboardCalls(mock)).toBe(before + 1));
    expect(activityStore.all().some((e) => /dashboard/i.test(e.action))).toBe(false);
  });

  it("reads once per interval while auto-refresh is on", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { mock, unmount } = renderShell("default", { ...seed, ui: { dashboardRefresh: 30 } });
      await within(await screen.findByRole("grid", { name: "Dashboard" })).findByText("invoice-generate");
      expect(dashboardCalls(mock)).toBe(1);
      await act(async () => void vi.advanceTimersByTime(29_000));
      expect(dashboardCalls(mock)).toBe(1);
      await act(async () => void vi.advanceTimersByTime(2_000));
      await waitFor(() => expect(dashboardCalls(mock)).toBe(2));
      await act(async () => void vi.advanceTimersByTime(30_000));
      await waitFor(() => expect(dashboardCalls(mock)).toBe(3));
      unmount();
      await act(async () => void vi.advanceTimersByTime(300_000));
      expect(dashboardCalls(mock)).toBe(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not read on its own while auto-refresh is off", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { mock } = renderShell("default", seed);
      await within(await screen.findByRole("grid", { name: "Dashboard" })).findByText("invoice-generate");
      await act(async () => void vi.advanceTimersByTime(600_000));
      expect(dashboardCalls(mock)).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows nothing of the old connection after switching to another", async () => {
    const tab = seed["shell:p-logistics"];
    const { mock } = renderShell("default", { last_profile: "p-logistics-dev", "shell:p-logistics-dev": tab, "shell:p-logistics": tab });
    const first = await screen.findByRole("grid", { name: "Dashboard" });
    await within(first).findByText("invoice-generate");
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "dashboard" && args?.profileId === "p-logistics") {
          await gate;
          return view([queue("prod-only-queue", counts(3, 0))]) as T;
        }
        return mock.invoke<T>(cmd, args);
      },
    });
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(screen.getByPlaceholderText(/Go to entity or command/), "Switch to Logistics{Enter}");
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    // The new connection's read has not come back: the grid holds nothing from the old one.
    expect(within(grid).queryByText("invoice-generate")).toBeNull();
    await act(async () => release());
    expect(await within(screen.getByRole("grid", { name: "Dashboard" })).findByText("prod-only-queue")).toBeInTheDocument();
    expect(within(screen.getByRole("grid", { name: "Dashboard" })).queryByText("invoice-generate")).toBeNull();
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

  it("shows unknown counts as ? with the reason, and counts them in the status line", async () => {
    renderShell("counts-unknown", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    const row = await within(grid).findByRole("row", { name: /invoice-generate/ });
    expect(within(row).getAllByText("?").length).toBeGreaterThanOrEqual(5);
    expect(within(row).getAllByTitle("The service didn't report counts for this entity.").length).toBeGreaterThanOrEqual(5);
    expect(screen.getByText(/\d+ entities with unknown counts/)).toBeInTheDocument();
  });

  it("explains a limited view in the status line, and does not name the kind of an entity it only knows", async () => {
    renderShell("forbidden", seed);
    const grid = await screen.findByRole("grid", { name: "Dashboard" });
    const row = await within(grid).findByRole("row", { name: /orders/ });
    expect(screen.getByText("2 pinned entities · Counts are unavailable while the namespace can't be listed.")).toBeInTheDocument();
    // No banner of its own: the tree carries the explanation and the way to fix it.
    expect(within(screen.getByRole("main")).queryByText("Showing pinned entities only")).toBeNull();
    // `orders` may be a topic: one "Kind unknown" glyph, in the grid and in the tree's pinned group; a subscription is known.
    expect(within(row).getByTitle("Kind unknown")).toBeInTheDocument();
    expect(within(within(grid).getByRole("row", { name: /tracking-sync/ })).queryByTitle("Kind unknown")).toBeNull();
    expect(within(within(screen.getByRole("tree", { name: "Namespace" })).getByRole("treeitem", { name: /^orders/ })).getByTitle("Kind unknown")).toBeInTheDocument();
    expect(within(row).getAllByText("?").length).toBeGreaterThan(0);
    // Nothing to filter by dead letters when no count is known.
    expect(screen.queryByRole("radio", { name: /With dead letters/ })).toBeNull();
  });

  it("lists the topics it could not read, with the distinct reasons, and caps the names", async () => {
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid", { name: "Dashboard" });
    const failures = ["a", "b", "c", "d", "e", "f", "g"].map((t, i) => ({ topic: `topic-${t}`, kind: i % 2 ? "network" : "forbidden", message: `${i % 2 ? "network error" : "forbidden: 401"} on topic-${t}` }));
    await serve(mock, view([queue("orders", counts(1, 0))], { failures }));
    expect(await screen.findByText("Couldn't list the subscriptions of 7 topics")).toBeInTheDocument();
    expect(screen.getByText(/topic-a, topic-b, topic-c, topic-d, topic-e and 2 more\. Reasons: forbidden, network\./)).toBeInTheDocument();
    expect(screen.queryByText(/topic-g/)).toBeNull();
    expect(screen.getByText("forbidden: 401 on topic-a | network error on topic-b")).toBeInTheDocument();
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

  it("marks approximate counts with ~ and capped ones with ~N+", () => {
    const row = (c: EntityCounts): DashboardRow => ({ kind: "queue", id: "q", name: "q", topic: null, status: "Active", counts: c });
    expect(toTsv([row(counts(1284, 3, { approximate: true }))]).split("\n")[1]).toBe("q\t\tqueue\tActive\t~1284\t~3\t~0\t~0\t~0");
    expect(toTsv([row(counts(1284, 3, { approximate: true, truncated: true }))]).split("\n")[1]).toBe("q\t\tqueue\tActive\t~1284+\t~3+\t~0+\t~0+\t~0+");
  });

  it("leaves the kind empty for entities known only by name", () => {
    const rows: DashboardRow[] = [{ kind: "queue", id: "orders", name: "orders", topic: null, status: null, counts: null }];
    expect(toTsv(rows, { kindsKnown: false }).split("\n")[1]).toBe("orders\t\t\t\t\t\t\t\t");
  });
});

describe("sortRows", () => {
  const rows = [queue("b", counts(5, 1)), queue("a", counts(5, 1)), queue("c", null), queue("d", counts(9, 0))];
  it("sorts by the column's value, unknown below zero, ties in incoming order", () => {
    expect(sortRows(rows, { id: "dlq", desc: true }).map((r) => r.name)).toEqual(["b", "a", "d", "c"]);
    expect(sortRows(rows, { id: "dlq", desc: false }).map((r) => r.name)).toEqual(["c", "d", "b", "a"]);
    expect(sortRows(rows, { id: "entity", desc: false }).map((r) => r.name)).toEqual(["a", "b", "c", "d"]);
  });
  it("keeps the incoming order without a sort", () => {
    expect(sortRows(rows, null)).toBe(rows);
  });
});
