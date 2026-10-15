import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import * as fx from "@/engine/mock/fixtures";
import { renderShell } from "../test-utils";
import { buildCommands } from "./commands";

const ctx = () => ({
  listing: { queues: fx.queues, topics: fx.topics, limited: null, known: [] },
  subs: { "shipment-events": fx.subscriptions["shipment-events"] },
  profiles: fx.profiles,
  activeProfileId: "p-logistics",
  open: vi.fn(), connect: vi.fn(), openSheet: vi.fn(), refresh: vi.fn(), toggleDrawer: vi.fn(), setPrefs: vi.fn(),
});

describe("buildCommands", () => {
  it("offers entities, connections and app commands with shortcuts", () => {
    const cmds = buildCommands(ctx());
    const labels = cmds.map((c) => `${c.group}: ${c.label}`);
    expect(labels).toContain("Go to: invoice-generate");
    expect(labels).toContain("Go to: shipment-events/tracking-sync");
    expect(labels).toContain("Connections: Switch to Logistics dev");
    expect(labels).not.toContain("Connections: Switch to Logistics");
    expect(cmds.find((c) => c.label === "Toggle drawer")?.shortcut).toBe("mod+j");
  });

  it("lists a pinned entity once even when the namespace lists it too", () => {
    const c = ctx();
    const cmds = buildCommands({ ...c, listing: { ...c.listing, known: [{ entity: "invoice-generate", subscription: null }] } });
    expect(cmds.filter((x) => x.label === "invoice-generate")).toHaveLength(1);
    expect(new Set(cmds.map((x) => x.id)).size).toBe(cmds.length);
  });

  it("sets preferences from the theme and density commands", () => {
    const c = ctx();
    const cmds = buildCommands(c);
    cmds.find((x) => x.label === "Theme: light")!.run();
    cmds.find((x) => x.label === "Density: comfortable")!.run();
    expect(c.setPrefs).toHaveBeenNthCalledWith(1, { theme: "light" });
    expect(c.setPrefs).toHaveBeenNthCalledWith(2, { density: "comfortable" });
  });

  it("opens a subscription by its tab id", () => {
    const c = ctx();
    buildCommands(c).find((x) => x.label === "shipment-events/tracking-sync")!.run();
    expect(c.open).toHaveBeenCalledWith({ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription" });
  });
});

describe("palette", () => {
  it("opens with mod+k and opens an entity", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const input = await screen.findByPlaceholderText("Go to an entity or run a command");
    await userEvent.type(input, "label-print");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true"));
    expect(screen.queryByPlaceholderText("Go to an entity or run a command")).not.toBeInTheDocument();
    // The tree follows: the opened entity is the selected row.
    expect(screen.getByRole("treeitem", { name: "label-print" })).toHaveAttribute("aria-selected", "true");
  });

  it("groups commands, shows shortcuts and says so when nothing matches", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const input = await screen.findByPlaceholderText("Go to an entity or run a command");
    expect(screen.getByText("Go to")).toBeInTheDocument();
    expect(screen.getByText("Connections")).toBeInTheDocument();
    expect(screen.getByText("App")).toBeInTheDocument();
    expect(screen.getByText("Ctrl+J")).toBeInTheDocument();
    await userEvent.type(input, "zzzzqq");
    expect(await screen.findByText("No matching entity or command.")).toBeInTheDocument();
  });

  it("closes with Escape and mod+k, and mod+k works while typing in the filter", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await act(async () => screen.getByRole("searchbox", { name: "Filter entities" }).focus());
    await userEvent.keyboard("{Control>}k{/Control}");
    await screen.findByPlaceholderText("Go to an entity or run a command");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByPlaceholderText("Go to an entity or run a command")).not.toBeInTheDocument());
    await userEvent.keyboard("{Control>}k{/Control}");
    await screen.findByPlaceholderText("Go to an entity or run a command");
    await userEvent.keyboard("{Control>}k{/Control}");
    await waitFor(() => expect(screen.queryByPlaceholderText("Go to an entity or run a command")).not.toBeInTheDocument());
  });

  // Needs Task 17's drawer; Task 17 replaces this with the real test.
  it.todo("toggles the drawer with mod+j");
});
