import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useShell } from "../context";
import { renderShell } from "../test-utils";

const tree = () => screen.findByRole("tree", { name: "Namespace" });

// In jsdom every element is a 0x0 box at the origin, so the resizable panels treat any pointer-down as a drag on
// their handle and swallow it: clicks never move focus. Focus explicitly where a test types or presses keys.
const focus = (el: HTMLElement) => act(() => el.focus());

/** A sentence that may be split across elements (a host kept on one line): match the element that holds all of it. */
const sentence = (text: string) =>
  screen.getByText((_, el) => el?.textContent === text && ![...(el?.children ?? [])].some((c) => c.textContent === text));

describe("NamespaceTree", () => {
  afterEach(() => vi.restoreAllMocks());

  it("lists queues and topics with counts, amber only on dead letters", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^invoice-generate(,| \(|$)/ });
    expect(within(row).getByText("1,284").className).not.toContain("text-accent");
    expect(within(row).getByText("37").className).toContain("text-accent");
  });

  it("renders a dead-letter count only when it is above zero, and keeps a zero active count", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const none = await within(t).findByRole("treeitem", { name: /^customs-docs(,| \(|$)/ });
    expect(within(none).getByText("112")).toBeInTheDocument();
    expect(within(none).queryByText("0")).not.toBeInTheDocument();
    const dead = within(t).getByRole("treeitem", { name: /^returns-intake(,| \(|$)/ });
    expect(within(dead).getByText("211").className).toContain("text-accent");
    // A zero active count stays (billing: 0 active, 0 dead letters).
    await userEvent.click(within(t).getByRole("treeitem", { name: /^shipment-events(,| \(|$)/ }));
    await focus(t);
    await userEvent.keyboard("{ArrowRight}");
    const empty = await within(t).findByRole("treeitem", { name: /^billing(,| \(|$)/ });
    expect(within(empty).getByText("0").className).toContain("text-muted");
  });

  it("counts are 12px and flush right, group rows are semibold muted", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^invoice-generate(,| \(|$)/ });
    expect(within(row).getByText("1,284").className).toContain("text-12");
    expect(within(row).getByText("37").className).not.toContain("min-w");
    const group = within(t).getByRole("treeitem", { name: /^Queues(,| \(|$)/ });
    const label = within(group).getByText("Queues");
    expect(label.className).toContain("font-semibold");
    expect(label.className).toContain("text-muted");
  });

  it("marks a production namespace with plain amber text, not the filled badge", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const ns = await within(t).findByRole("treeitem", { name: /^sb-logistics-prod(,| \(|$)/ });
    const prod = within(ns).getByText("PROD");
    expect(prod.className).toContain("text-accent");
    expect(prod.className).not.toContain("bg-accent");
  });

  it("has no PROD mark on a non-production namespace", async () => {
    renderShell("default", { last_profile: "p-logistics-dev" });
    const t = await tree();
    const ns = await within(t).findByRole("treeitem", { name: /^sb-logistics-dev(,| \(|$)/ });
    expect(within(ns).queryByText("PROD")).not.toBeInTheDocument();
  });

  it("loads subscriptions when a topic expands", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^shipment-events(,| \(|$)/ });
    await userEvent.click(row);
    await focus(t);
    await userEvent.keyboard("{ArrowRight}");
    expect(await within(t).findByRole("treeitem", { name: /^tracking-sync(,| \(|$)/ })).toBeInTheDocument();
    expect(mock.calls).toContainEqual({ cmd: "list_subscriptions", args: { profileId: "p-logistics", topic: "shipment-events" } });
  });

  it("does not load subscriptions of collapsed topics", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await within(await tree()).findByRole("treeitem", { name: /^shipment-events(,| \(|$)/ });
    expect(mock.calls.filter((c) => c.cmd === "list_subscriptions")).toEqual([]);
  });

  it("selects a row on click", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const row = await within(await tree()).findByRole("treeitem", { name: /^label-print(,| \(|$)/ });
    await userEvent.click(row);
    expect(row).toHaveAttribute("aria-selected", "true");
  });

  it("filters across loaded entities", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await within(t).findByRole("treeitem", { name: /^label-print(,| \(|$)/ });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await userEvent.keyboard("LABEL");
    await waitFor(() => expect(within(t).queryByRole("treeitem", { name: /^invoice-generate(,| \(|$)/ })).not.toBeInTheDocument());
    expect(within(t).getByRole("treeitem", { name: /^label-print(,| \(|$)/ })).toBeInTheDocument();
  });

  it("says so when nothing matches the filter", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await within(await tree()).findByRole("treeitem", { name: /^label-print(,| \(|$)/ });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await userEvent.keyboard("zzz");
    expect(await screen.findByRole("heading", { name: "No matches" })).toBeInTheDocument();
  });

  it("explains a limited listing and shows pinned entities", async () => {
    renderShell("forbidden", { last_profile: "p-logistics" });
    expect(await screen.findByRole("heading", { name: "Showing pinned entities only" })).toBeInTheDocument();
    expect(
      sentence("This identity can't list entities in sb-logistics-prod. Grant it Manage rights to see everything, or edit this connection to pin more."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/needs Manage,EntityRead/)).not.toBeInTheDocument(); // the engine's wording is not echoed
    expect(within(await tree()).getByRole("treeitem", { name: /^orders(,| \(|$)/ })).toBeInTheDocument();
  });

  it("shows pinned subscriptions by name with their topic beside it", async () => {
    renderShell("forbidden", { last_profile: "p-logistics" });
    const row = await within(await tree()).findByRole("treeitem", { name: /^tracking-sync(,| \(|$)/ });
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
    expect(sentence("The connection to sb-logistics-prod was interrupted. Check your network or VPN, then try again.")).toBeInTheDocument();
    // The host never breaks across lines.
    expect(screen.getByText("sb-logistics-prod", { selector: "span.whitespace-nowrap" })).toBeInTheDocument();
    // The drawer logs the same failure; the tree's own line is the one outside it, in mono on its own line.
    const raw = screen.getAllByText("network error: connection reset by peer").find((el) => !el.closest("[aria-label='Jobs and activity']"))!;
    expect(raw.tagName).toBe("P");
    expect(raw).toHaveClass("font-mono", "text-11", "text-muted");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("sends an expired sign-in to the connection sheet, in the tree, the status bar and the selector", async () => {
    const user = userEvent.setup();
    function SheetProbe() {
      const { sheet } = useShell();
      return <output aria-label="Open sheet">{sheet ? JSON.stringify(sheet) : "none"}</output>;
    }
    renderShell("sign-in-expired", { last_profile: "p-logistics-dev" }, <SheetProbe />);
    expect(await screen.findByRole("heading", { name: "Can't list this namespace" })).toBeInTheDocument();
    expect(sentence("Your sign-in for Logistics dev has expired. Edit the connection to sign in again.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).not.toBeInTheDocument();
    const bar = screen.getByRole("contentinfo");
    expect(within(bar).getAllByRole("status")[0]).toHaveTextContent("Sign-in expired"); // the first status is the connection state
    expect(within(bar).queryByRole("button", { name: "Reconnect" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Connection: Logistics dev, sb-logistics-dev, sign-in expired$/ })).toBeInTheDocument();
    await user.click(within(bar).getByRole("button", { name: "Edit connection" }));
    expect(screen.getByLabelText("Open sheet")).toHaveTextContent('{"kind":"profile","profileId":"p-logistics-dev"}');
  });

  it("marks a lost connection on the status bar and the selector", async () => {
    renderShell("network", { last_profile: "p-logistics" });
    expect(await screen.findByRole("heading", { name: "Can't list this namespace" }, { timeout: 4000 })).toBeInTheDocument();
    const bar = screen.getByRole("contentinfo");
    expect(within(bar).getAllByRole("status")[0]).toHaveTextContent("Connection lost");
    expect(within(bar).getByRole("button", { name: "Reconnect" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connection: Logistics, sb-logistics-prod, production, connection lost" })).toBeInTheDocument();
  });

  it("opens the context menu from the keyboard on the selected row", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await user.click(await within(t).findByRole("treeitem", { name: /^label-print(,| \(|$)/ }));
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
    await user.click(await within(t).findByRole("treeitem", { name: /^label-print(,| \(|$)/ }));
    fireEvent.contextMenu(t, { button: 2 });
    expect(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Open" })).toHaveAttribute("aria-disabled", "true");
  });

  it("keeps loaded subscriptions in the tree when a topic is filtered by its name", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    await user.click(await within(t).findByRole("treeitem", { name: /^shipment-events(,| \(|$)/ }));
    await focus(t);
    await user.keyboard("{ArrowRight}");
    await within(t).findByRole("treeitem", { name: /^tracking-sync(,| \(|$)/ });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await user.keyboard("shipment");
    expect(within(t).getByRole("treeitem", { name: /^billing(,| \(|$)/ })).toBeInTheDocument();
    expect(within(t).getByRole("treeitem", { name: /^warehouse-notify(,| \(|$)/ })).toBeInTheDocument();
  });

  it("offers open, copy and refresh in the context menu", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const row = await within(await tree()).findByRole("treeitem", { name: /^invoice-generate(,| \(|$)/ });
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
    const group = await within(await tree()).findByRole("treeitem", { name: /^Queues(,| \(|$)/ });
    fireEvent.contextMenu(group);
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Open" })).toHaveAttribute("aria-disabled", "true");
    expect(within(menu).getByRole("menuitem", { name: "Refresh" })).not.toHaveAttribute("aria-disabled", "true");
  });

  it("shows the chosen counts and a highlight swatch from the connection's rules", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": {
        tabs: [], activeTab: null, expanded: ["group:queues"], selected: null,
        layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" },
        explorer: { counts: ["active", "scheduled", "dead-letter"], rules: [{ id: "r", color: "rose", match: { kind: "count", count: "dead-letter", atLeast: 100 } }], openOn: "dead-letter" },
      },
    });
    const t = await tree();
    const returns = await within(t).findByRole("treeitem", { name: /returns-intake/ });
    expect(returns.querySelector('[style*="--label-rose"]')).not.toBeNull();
    expect(within(returns).getByText("highlighted")).toBeInTheDocument();
    expect(within(t).getByRole("treeitem", { name: /invoice-generate/ }).querySelector('[style*="--label-"]')).toBeNull();
    await userEvent.dblClick(within(t).getByRole("treeitem", { name: /label-print/ }));
    expect(await screen.findByRole("tab", { name: /Dead-letter/, selected: true })).toBeInTheDocument();
  });

  it("shows only the counts that are chosen, dead letters last and scheduled only above zero", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": {
        tabs: [], activeTab: null, expanded: ["group:queues"], selected: null,
        layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" },
        explorer: { counts: ["scheduled", "dead-letter"], rules: [], openOn: "overview" },
      },
    });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^invoice-generate(,| \(|$)/ });
    expect(within(row).queryByText("1,284")).not.toBeInTheDocument();
    expect(within(row).getByText("37").className).toContain("text-accent");
    const quiet = within(t).getByRole("treeitem", { name: /^label-print(,| \(|$)/ });
    expect(quiet.textContent).toBe("label-print");
  });

  it("highlights by name for topics and for pinned entities, whose counts are unknown", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": {
        tabs: [], activeTab: null, expanded: ["group:topics"], selected: null,
        layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" },
        explorer: { counts: ["active"], rules: [{ id: "r", color: "violet", match: { kind: "name", pattern: "^shipment" } }], openOn: "overview" },
      },
    });
    const t = await tree();
    const topic = await within(t).findByRole("treeitem", { name: /^shipment-events(,| \(|$)/ });
    expect(topic.querySelector('[style*="--label-violet"]')).not.toBeNull();
  });

  it("copies the URL and asks the engine for the connection string", async () => {
    const user = userEvent.setup(); // installs the clipboard stub
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const menuOn = async (name: string | RegExp) => fireEvent.contextMenu(await within(t).findByRole("treeitem", { name }));
    await menuOn(/invoice-generate/);
    await user.click(await screen.findByRole("menuitem", { name: "Copy URL" }));
    expect(await navigator.clipboard.readText()).toBe("sb://sb-logistics-prod.servicebus.windows.net/invoice-generate");
    await menuOn(/invoice-generate/);
    await user.click(await screen.findByRole("menuitem", { name: "Copy dead-letter URL" }));
    expect(await navigator.clipboard.readText()).toBe("sb://sb-logistics-prod.servicebus.windows.net/invoice-generate/$DeadLetterQueue");
    await menuOn(/invoice-generate/);
    await user.click(await screen.findByRole("menuitem", { name: "Copy connection string" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "copy_entity_connection_string")).toBe(true));
    // The string never reaches the webview: only the outcome is shown, in the status bar.
    const bar = screen.getByRole("contentinfo");
    expect(await within(bar).findByText("Copied the connection string for invoice-generate. It is scoped to that entity and was never shown.")).toBeInTheDocument();
    expect(screen.queryByText(/SharedAccessKey/)).toBeNull();
    expect(within(t).queryByRole("status")).toBeNull();
  });

  it("says in the status bar which URL was copied, without moving the tree", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const before = within(t).getByRole("treeitem", { name: /invoice-generate/ }).parentElement!.style.height;
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /invoice-generate/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Copy dead-letter URL" }));
    expect(await within(screen.getByRole("contentinfo")).findByText("Copied the dead-letter URL for invoice-generate")).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Namespace" }).querySelector("[role=alert], [role=status]")).toBeNull();
    expect(within(t).getByRole("treeitem", { name: /invoice-generate/ }).parentElement!.style.height).toBe(before);
  });

  it("says in the status bar that the connection string could not be copied, with a way to the details", async () => {
    const user = userEvent.setup();
    renderShell("copy-scoped", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [], activeTab: null, expanded: ["group:queues"], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "jobs" } },
    });
    const t = await tree();
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /invoice-generate/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Copy connection string" }));
    const bar = screen.getByRole("contentinfo");
    expect(await within(bar).findByText(/Couldn't copy the connection string/)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Jobs and activity" })).toBeNull();
    await user.click(within(bar).getByRole("button", { name: "Details" }));
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    expect(within(drawer).getByRole("tab", { name: /Activity/, selected: true })).toBeInTheDocument();
    expect(within(drawer).getByText(/Copying the connection string for invoice-generate failed/)).toBeInTheDocument();
  });

  it("offers no dead-letter URL for a topic", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /^shipment-events(,| \(|$)/ }));
    expect(await screen.findByRole("menuitem", { name: "Copy dead-letter URL" })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("menuitem", { name: "Copy URL" })).not.toHaveAttribute("aria-disabled", "true");
  });

  it("disables the connection string on an Entra connection and says why", async () => {
    renderShell("default", { last_profile: "p-logistics-dev" });
    const t = await tree();
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /invoice-generate/ }));
    const item = await screen.findByRole("menuitem", { name: "Copy connection string" });
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveAttribute("title", expect.stringMatching(/Entra ID connections have no connection string/));
  });

  it("remembers the tree filter for the connection", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    const box = await screen.findByRole("searchbox", { name: "Filter entities" });
    await focus(box);
    await userEvent.keyboard("invoice");
    await waitFor(() => {
      const saved = mock.calls.filter((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "shell:p-logistics").at(-1);
      expect((saved?.args as { value: { treeFilter: string } } | undefined)?.value.treeFilter).toBe("invoice");
    });
  });

  const seeded = (explorer: Record<string, unknown>, extra: Record<string, unknown> = {}, scenario = "default") =>
    renderShell(scenario as "default", {
      last_profile: "p-logistics",
      "shell:p-logistics": {
        tabs: [], activeTab: null, expanded: ["group:queues", "group:topics", "t:shipment-events"], selected: null,
        layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" },
        explorer, ...extra,
      },
    });
  const COUNTS = { counts: ["active", "scheduled", "dead-letter"], rules: [{ id: "r", color: "rose", match: { kind: "count", count: "dead-letter", atLeast: 100 } }], openOn: "overview" };

  it("names a row with its counts and its highlight, for a screen reader", async () => {
    seeded(COUNTS);
    const t = await tree();
    const returns = await within(t).findByRole("treeitem", { name: "returns-intake, 48 active, 211 dead-letter, highlighted" });
    expect(returns).toHaveAccessibleDescription("rose: dead-letter at least 100");
    expect(within(t).getByRole("treeitem", { name: "invoice-generate, 1,284 active, 3 scheduled, 37 dead-letter" })).toHaveAccessibleDescription("");
    // A subscription is named with its topic.
    expect(within(t).getByRole("treeitem", { name: "tracking-sync (shipment-events), 2,906 active, 3,417 dead-letter, highlighted" })).toBeInTheDocument();
    expect(within(t).getAllByRole("treeitem", { name: /highlighted/ })).toHaveLength(2); // returns-intake and tracking-sync (3,417)
    expect(within(t).getByRole("treeitem", { name: "label-print, 6 active" })).toBeInTheDocument();
    expect(within(t).getByRole("treeitem", { name: /^shipment-events(,| \(|$)/ })).toBeInTheDocument();
  });

  it("shows the scheduled count of a queue when scheduled counts are on", async () => {
    seeded(COUNTS);
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    expect(within(row).getByText("3")).toBeInTheDocument();
  });

  it("gives each chosen count a fixed slot as wide as its widest value, so position tells the kind", async () => {
    seeded(COUNTS);
    const t = await tree();
    await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    const widths = (kind: string) => [...new Set([...t.querySelectorAll<HTMLElement>(`[data-count-slot="${kind}"]`)].map((e) => e.style.minWidth))];
    expect(widths("active")).toEqual(["5ch"]); // 1,284
    expect(widths("scheduled")).toEqual(["1ch"]);
    expect(widths("dead-letter")).toEqual(["5ch"]); // 3,417 (a subscription below)
    expect(t.querySelector('[data-count-slot="transfer-dlq"]')).toBeNull();
    // A zero is a blank slot, not a missing one.
    const quiet = within(t).getByRole("treeitem", { name: /^label-print/ });
    expect(quiet.querySelector('[data-count-slot="dead-letter"]')?.textContent).toBe("");
    expect(quiet.querySelectorAll("[data-count-slot]")).toHaveLength(3);
  });

  /** The tree is laid out at 0x0 in jsdom: give its pane a width, as the browser would. */
  const withPane = (width: number) =>
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const w = this.matches('nav[aria-label="Namespace"]') ? width : 0;
      return { x: 0, y: 0, top: 0, left: 0, right: w, bottom: 0, width: w, height: 0, toJSON: () => ({}) };
    });

  it("lets scheduled and transfer DLQ give way first in a pane that cannot hold every count, then the fixed slots", async () => {
    const explorer = { ...COUNTS, counts: ["active", "scheduled", "transfer-dlq", "dead-letter"] };
    const spy = withPane(600);
    const wide = seeded(explorer);
    let t = await tree();
    let row = await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    expect(row.querySelector('[data-count-slot="scheduled"]')).not.toBeNull();
    expect(row.querySelector('[data-count-slot="transfer-dlq"]')).not.toBeNull();
    expect(row.querySelector<HTMLElement>('[data-count-slot="active"]')!.style.minWidth).toBe("5ch");
    // A name keeps six characters while every count has its slot.
    expect(within(row).getByText("invoice-generate").className).toContain("min-w-[6ch]");
    wide.unmount();
    spy.mockRestore();

    // 260px holds active and dead-letter in their slots, not scheduled and transfer DLQ as well.
    const mid = withPane(260);
    const midRender = seeded(explorer);
    t = await tree();
    row = await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    expect(row.querySelector('[data-count-slot="scheduled"]')).toBeNull();
    expect(row.querySelector('[data-count-slot="transfer-dlq"]')).toBeNull();
    expect(row.querySelector<HTMLElement>('[data-count-slot="active"]')!.style.minWidth).toBe("5ch");
    expect(row.querySelector('[data-count-slot="dead-letter"]')).not.toBeNull();
    midRender.unmount();
    mid.mockRestore();

    // At the tree's minimum each count takes only its own width and the name gives way to two characters.
    const narrow = withPane(200);
    seeded(explorer);
    t = await tree();
    row = await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    expect(row.querySelector<HTMLElement>('[data-count-slot="active"]')!.style.minWidth).toBe("");
    expect(within(row).getByText("invoice-generate").className).toContain("min-w-[2ch]");
    expect(row.querySelector('[data-count-slot="scheduled"]')).toBeNull();
    narrow.mockRestore();
  });

  it("lets a pinned subscription's name shrink, and its topic first", async () => {
    renderShell("forbidden", { last_profile: "p-logistics" });
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^tracking-sync/ });
    expect(within(row).getByText("tracking-sync").className).not.toContain("shrink-0");
    expect(within(row).getByText("tracking-sync").className).toContain("truncate");
    expect(within(row).getByText("shipment-events").className).toContain("[flex-shrink:3]");
  });

  it("speaks an approximate count as about, not as a tilde", async () => {
    seeded(COUNTS, {}, "counts-approx");
    const t = await tree();
    expect(await within(t).findByRole("treeitem", { name: /^invoice-generate, about 1,284 active/ })).toBeInTheDocument();
  });

  it("explains a count in a tooltip", async () => {
    seeded(COUNTS);
    const t = await tree();
    const row = await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    expect(row.querySelector("[data-count-slot] [title], [data-count-slot][title]")).toBeNull(); // no native titles on counts
    fireEvent.pointerMove(within(row).getByText("3").parentElement!);
    expect(await screen.findByRole("tooltip", {}, { timeout: 2000 })).toHaveTextContent("Scheduled");
  });

  it("reserves a swatch slot at the right edge of every row when the connection has a rule", async () => {
    seeded(COUNTS);
    const t = await tree();
    await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    const rows = within(t).getAllByRole("treeitem").filter((r) => !/^(sb-|Queues|Topics)/.test(r.getAttribute("aria-label") ?? ""));
    expect(rows.length).toBeGreaterThan(6);
    for (const r of rows) expect(r.querySelector("[data-swatch-slot]"), r.getAttribute("aria-label") ?? "").not.toBeNull();
    const returns = within(t).getByRole("treeitem", { name: /^returns-intake/ });
    const slot = returns.querySelector("[data-swatch-slot]")!;
    expect(slot.querySelector('[style*="--label-rose"]')).not.toBeNull();
    // The swatch is the last thing in the row and sits clear of the dead-letter count.
    expect(slot.parentElement!.className).toContain("gap-2"); // 8px between slots, so adjacent counts do not read as one number
    expect(slot.parentElement!.lastElementChild).toBe(slot);
    expect(slot.previousElementSibling?.getAttribute("data-count-slot")).toBe("dead-letter");
    expect(within(t).getByRole("treeitem", { name: /^label-print/ }).querySelector("[data-swatch-slot]")?.children).toHaveLength(0);
    fireEvent.pointerMove(slot);
    expect(await screen.findByRole("tooltip", {}, { timeout: 2000 })).toHaveTextContent("Rose: dead-letter at least 100");
  });

  it("reserves no swatch slot without rules", async () => {
    seeded({ ...COUNTS, rules: [] });
    const t = await tree();
    await within(t).findByRole("treeitem", { name: /^invoice-generate/ });
    expect(t.querySelector("[data-swatch-slot]")).toBeNull();
  });

  it("says nothing about a connection string copy that finishes after the connection was switched", async () => {
    function Probe() {
      const sh = useShell();
      return (
        <>
          <button onClick={() => void sh.copyConnectionString("invoice-generate")}>probe copy</button>
          <button onClick={() => void sh.conn.connect("p-partner")}>probe switch</button>
        </>
      );
    }
    const { mock } = renderShell("default", { last_profile: "p-logistics" }, <Probe />);
    await tree();
    let release: () => void = () => {};
    const real = mock.invoke.bind(mock);
    mock.invoke = ((cmd: string, args?: Record<string, unknown>) =>
      cmd === "copy_entity_connection_string" ? new Promise((res) => (release = () => res(real(cmd, args)))) : real(cmd, args)) as typeof mock.invoke;
    await userEvent.click(screen.getByRole("button", { name: "probe copy" }));
    await userEvent.click(screen.getByRole("button", { name: "probe switch" }));
    await waitFor(() => expect(screen.getByRole("banner")).toHaveTextContent("Partner gateway"));
    release();
    await act(async () => { await new Promise((r) => setTimeout(r, 50)); });
    expect(within(screen.getByRole("contentinfo")).queryByText(/connection string/)).toBeNull();
  });

  it("keeps the status bar's notice line in the page, empty until there is something to say", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    const t = await tree();
    const bar = screen.getByRole("contentinfo");
    const notice = within(bar).getAllByRole("status")[1];
    expect(notice).toHaveTextContent("");
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /^tracking-sync|^invoice-generate/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Copy URL" }));
    await waitFor(() => expect(notice).toHaveTextContent(/^Copied the URL for /));
    expect(within(bar).getAllByRole("status")[1]).toBe(notice); // the same element, filled, so it is announced
  });

  it("names a subscription with its topic in a copy message", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [], activeTab: null, expanded: ["group:topics", "t:shipment-events"], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } } });
    const t = await tree();
    fireEvent.contextMenu(await within(t).findByRole("treeitem", { name: /^tracking-sync/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Copy URL" }));
    expect(await within(screen.getByRole("contentinfo")).findByText("Copied the URL for tracking-sync (shipment-events)")).toBeInTheDocument();
  });
});
