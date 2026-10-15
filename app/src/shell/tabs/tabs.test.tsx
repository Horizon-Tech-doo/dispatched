import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";
import { subTabsFor } from "./subtabs";

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
    expect(await screen.findByRole("heading", { name: "Dead-letter isn't available yet" })).toBeInTheDocument();
  });

  it("close with the close button and fall back to the empty workspace", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "label-print", kind: "queue", subTab: "overview" }], activeTab: "label-print" } });
    await screen.findByRole("tab", { name: /label-print/ });
    await userEvent.click(screen.getByRole("button", { name: "Close label-print" }));
    expect(await screen.findByRole("heading", { name: "Open an entity" })).toBeInTheDocument();
  });

  it("tab_for_missing_entity_shows_gone_state", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "deleted-queue", kind: "queue", subTab: "overview" }], activeTab: "deleted-queue" } });
    expect(await screen.findByRole("heading", { name: "This entity no longer exists" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Close tab" }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: /deleted-queue/ })).not.toBeInTheDocument());
  });

  it("restores the saved sub-tab", async () => {
    renderShell("default", { last_profile: "p-logistics", "shell:p-logistics": { tabs: [{ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "dead-letter" }], activeTab: "shipment-events/subscriptions/tracking-sync" } });
    const views = await screen.findByRole("tablist", { name: "Entity views" });
    await waitFor(() => expect(within(views).getByRole("tab", { name: /Dead-letter/ })).toHaveAttribute("aria-selected", "true"));
    expect(await within(views).findByText("3,417")).toBeInTheDocument();
  });
});
