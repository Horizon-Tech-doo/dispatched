import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { setTransport } from "@/engine";
import { renderShell } from "../test-utils";
import { countPlaceholderCh, countsKey, rememberCounts, subTabsFor } from "./subtabs";

// In jsdom the resizable panels swallow pointer-downs, so focus moves explicitly (as in the tree tests).
const focus = (el: HTMLElement) => act(() => el.focus());

describe("sub-tab definitions", () => {
  it("follow the spec per entity kind", () => {
    expect(subTabsFor("queue").map((t) => t.id)).toEqual(["overview", "messages", "dead-letter", "transfer-dlq", "scheduled", "deferred", "sessions", "listener"]);
    expect(subTabsFor("subscription").map((t) => t.id)).toContain("rules");
    expect(subTabsFor("topic").map((t) => t.id)).toEqual(["overview"]);
  });
});

describe("entity tabs", () => {
  it("open from the tree with Enter and show counts on sub-tabs", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const tree = await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(await within(tree).findByRole("treeitem", { name: "invoice-generate" }));
    await focus(tree);
    await userEvent.keyboard("{Enter}");
    const strip = await screen.findByRole("tablist", { name: "Open entities" });
    expect(within(strip).getByRole("tab", { name: /invoice-generate/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("heading", { level: 1, name: "invoice-generate" })).toBeInTheDocument();
    const views = screen.getByRole("tablist", { name: "Entity views" });
    const dl = within(views).getByRole("tab", { name: /Dead-letter/ });
    expect(within(dl).getByText("37").className).toContain("text-accent");
    await focus(dl); // Radix activates a focused tab; the panels swallow the pointer-down a click starts with
    expect(await screen.findByRole("grid", { name: "Dead-letter queue" })).toBeInTheDocument();
  });

  it("holds the place of sub-tab counts while they load", async () => {
    const sub = "shipment-events/subscriptions/tracking-sync";
    const { mock } = renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: sub, kind: "subscription", subTab: "overview" }], activeTab: sub } });
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "list_subscriptions") await gate;
        return mock.invoke<T>(cmd, args);
      },
    });
    const views = await screen.findByRole("tablist", { name: "Entity views" });
    const messages = within(views).getByRole("tab", { name: /Messages/ });
    await waitFor(() => expect(messages.querySelector("[data-count-placeholder]")).not.toBeNull());
    expect(messages.querySelector("[data-count-placeholder]")).toHaveStyle({ minWidth: "3ch" });
    // Tabs without a count reserve nothing.
    expect(within(views).getByRole("tab", { name: "Overview" }).querySelector("[data-count-placeholder]")).toBeNull();
    await act(async () => release());
    // Loaded, the count takes only its own width: no reserved slot to leave a gap before the next tab.
    await waitFor(() => expect(within(messages).getByText("2,906")).toBeInTheDocument());
    expect(within(messages).getByText("2,906").className).not.toMatch(/min-w-/);
    expect(messages.querySelector("[data-count-placeholder]")).toBeNull();
  });

  it("holds the room a tab's count took last time while it loads again", async () => {
    const sub = "shipment-events/subscriptions/tracking-sync";
    const { mock } = renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: sub, kind: "subscription", subTab: "overview" }], activeTab: sub } });
    rememberCounts(countsKey("p-logistics", sub), { active: 12_345, dead_letter: 5, scheduled: 0, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false });
    const gate = new Promise<void>(() => {});
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "list_subscriptions") await gate;
        return mock.invoke<T>(cmd, args);
      },
    });
    const views = await screen.findByRole("tablist", { name: "Entity views" });
    const messages = within(views).getByRole("tab", { name: /Messages/ });
    // "12,345" is six characters wide; Dead-letter's "5" one; a tab with no remembered count holds three.
    await waitFor(() => expect(messages.querySelector("[data-count-placeholder]")).toHaveStyle({ minWidth: "6ch" }));
    expect(within(views).getByRole("tab", { name: /Dead-letter/ }).querySelector("[data-count-placeholder]")).toHaveStyle({ minWidth: "1ch" });
  });

  it("sizes a placeholder from the last count, else three characters", () => {
    const messages = subTabsFor("queue").find((t) => t.id === "messages")!;
    expect(countPlaceholderCh(messages, null)).toBe(3);
    expect(countPlaceholderCh(messages, { active: 1_284, dead_letter: 0, scheduled: 0, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false })).toBe(5);
    expect(countPlaceholderCh(subTabsFor("queue").find((t) => t.id === "deferred")!, { active: 1, dead_letter: 0, scheduled: 0, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false })).toBe(3);
  });

  it("shows the kind word beside a queue or topic name, with no glyph, and the topic beside a subscription", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": {
        tabs: [
          { id: "label-print", kind: "queue", subTab: "overview" },
          { id: "shipment-events", kind: "topic", subTab: "overview" },
          { id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "overview" },
        ],
        activeTab: "label-print",
      },
    });
    const strip = await screen.findByRole("tablist", { name: "Open entities" });
    expect(within(within(strip).getByRole("tab", { name: /label-print/ })).getByText("queue").className).toContain("text-faint");
    expect(within(within(strip).getByRole("tab", { name: /^shipment-events topic/ })).getByText("topic").className).toContain("text-faint");
    expect(within(within(strip).getByRole("tab", { name: /tracking-sync/ })).getByText("shipment-events").className).toContain("text-faint");
    expect(strip.querySelector("svg:not([class*='lucide-x'])")).toBeNull();
  });

  it("draws the active marker on the whole tab, close button included", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }], activeTab: "label-print" } });
    const close = await screen.findByRole("button", { name: "Close label-print" });
    expect(close.parentElement?.className).toContain("border-t-fg");
    expect(close.parentElement?.className).toContain("bg-content");
    const trigger = screen.getByRole("tab", { name: /label-print/ }).className;
    expect(trigger).toContain("data-[state=active]:bg-transparent");
    expect(trigger).not.toContain("data-[state=active]:bg-content");
    expect(trigger).toContain("border-t-0");
  });

  it("close with the close button and fall back to the empty workspace", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }], activeTab: "label-print" } });
    await screen.findByRole("tab", { name: /label-print/ });
    await userEvent.click(screen.getByRole("button", { name: "Close label-print" }));
    expect(await screen.findByRole("heading", { name: "Open an entity" })).toBeInTheDocument();
  });

  it("tab_for_missing_entity_shows_gone_state", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "deleted-queue", kind: "queue", subTab: "overview" }], activeTab: "deleted-queue" } });
    expect(await screen.findByRole("heading", { name: "This entity wasn't found" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Close tab" }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: /deleted-queue/ })).not.toBeInTheDocument());
  });

  it("names the topic in a subscription's close button", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "overview" }], activeTab: "shipment-events/subscriptions/tracking-sync" } });
    expect(await screen.findByRole("button", { name: "Close tracking-sync on shipment-events" })).toBeInTheDocument();
  });

  it("moves focus to the newly active tab after closing with the close button", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": {
        tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }, { id: "customs-docs", kind: "queue", subTab: "overview" }],
        activeTab: "label-print",
      },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Close label-print" }));
    await waitFor(() => expect(screen.getByRole("tab", { name: /customs-docs/ })).toHaveFocus());
    expect(screen.getByRole("tab", { name: /customs-docs/ })).toHaveAttribute("aria-selected", "true");
  });

  it("moves focus to the tree after the last tab is closed with the close button", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }], activeTab: "label-print" } });
    await userEvent.click(await screen.findByRole("button", { name: "Close label-print" }));
    expect(await screen.findByRole("heading", { name: "Open an entity" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("tree", { name: "Namespace" })).toHaveFocus());
  });

  it("subscription_of_missing_topic_shows_gone_state", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "deleted-topic/subscriptions/x", kind: "subscription", subTab: "overview" }], activeTab: "deleted-topic/subscriptions/x" } });
    expect(await screen.findByRole("heading", { name: "This entity wasn't found" })).toBeInTheDocument();
  });

  it("falls_back_to_overview_for_a_saved_sub_tab_the_kind_does_not_have", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "shipment-events", kind: "topic", subTab: "dead-letter" }], activeTab: "shipment-events" } });
    const views = await screen.findByRole("tablist", { name: "Entity views" });
    expect(within(views).getByRole("tab", { name: "Overview" })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByRole("heading", { name: "Overview isn't available yet" })).toBeInTheDocument();
  });

  it("shows_the_header_and_a_note_when_the_namespace_cannot_be_listed", async () => {
    renderShell("network", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }], activeTab: "label-print" } });
    expect(await screen.findByRole("heading", { level: 1, name: "label-print" })).toBeInTheDocument();
    expect(await screen.findByText("Counts are unavailable while the namespace can't be listed.", {}, { timeout: 5000 })).toBeInTheDocument();
    expect(screen.queryByRole("status", { name: "Loading" })).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Messages" })).toBeInTheDocument();
  }, 10000);

  it("restores the saved sub-tab", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "dead-letter" }], activeTab: "shipment-events/subscriptions/tracking-sync" } });
    const views = await screen.findByRole("tablist", { name: "Entity views" });
    await waitFor(() => expect(within(views).getByRole("tab", { name: /Dead-letter/ })).toHaveAttribute("aria-selected", "true"));
    expect(await within(views).findByText("3,417")).toBeInTheDocument();
  });
});
