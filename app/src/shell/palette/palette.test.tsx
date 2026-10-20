import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import * as fx from "@/engine/mock/fixtures";
import { renderShell } from "../test-utils";
import { buildCommands } from "./commands";

const ctx = () => ({
  listing: { queues: fx.queues, topics: fx.topics, limited: null, known: [] },
  subs: { "shipment-events": fx.subscriptions["shipment-events"] },
  profiles: fx.profiles.map((p) => ({ ...p, bound_sign_in: null })),
  activeProfileId: "p-logistics",
  activeTab: null as { id: string; kind: "queue" | "topic" | "subscription" } | null,
  host: "sb-logistics-prod.servicebus.windows.net",
  entra: false,
  copy: vi.fn(), copyConnectionString: vi.fn(),
  open: vi.fn(), connect: vi.fn(), openSheet: vi.fn(), refresh: vi.fn(), toggleDrawer: vi.fn(), setPrefs: vi.fn(),
});

describe("buildCommands", () => {
  it("palette lists every operate command for a queue, and no Rules commands", () => {
    const c = { ...ctx(), activeTab: { id: "invoice-generate", kind: "queue" as const } };
    const labels = buildCommands(c).filter((x) => x.group === "This entity").map((x) => x.label);
    for (const l of [
      "Send messages to invoice-generate…", "Receive messages from invoice-generate…", "Delete messages in invoice-generate…", "Move messages from invoice-generate…",
      "Resubmit messages from invoice-generate…", "Purge invoice-generate…", "Unschedule messages in invoice-generate…",
      "Import messages into invoice-generate…",
    ])
      expect(labels).toContain(l);
    expect(buildCommands(c).map((x) => x.label)).toContain("Show activity");
    expect(labels.some((l) => /rule/i.test(l))).toBe(false);
    // Session state only where sessions are used.
    expect(labels.some((l) => l.startsWith("Session state"))).toBe(false);
    const session = buildCommands({ ...ctx(), activeTab: { id: "returns-intake", kind: "queue" } }).map((x) => x.label);
    expect(session).toContain("Session state of returns-intake…");
    buildCommands(c).find((x) => x.label === "Purge invoice-generate…")!.run();
    expect(c.openSheet).toHaveBeenCalledWith({ kind: "operation", op: "purge", entities: ["invoice-generate"] });
  });

  it("a topic offers only sending and importing among message operations", () => {
    const labels = buildCommands({ ...ctx(), activeTab: { id: "shipment-events", kind: "topic" } }).filter((x) => x.group === "This entity" && x.id.startsWith("op:")).map((x) => x.label);
    expect(labels.filter((l) => l.endsWith("…"))).toEqual(["Send messages to shipment-events…", "Import messages into shipment-events…"]);
  });

  it("offers entities, connections and app commands with shortcuts", () => {
    const cmds = buildCommands(ctx());
    const labels = cmds.map((c) => `${c.group}: ${c.label}`);
    expect(labels).toContain("Go to: invoice-generate");
    expect(labels).toContain("Go to: tracking-sync");
    expect(cmds.find((c) => c.label === "tracking-sync")?.detail).toBe("shipment-events");
    expect(cmds.find((c) => c.label === "invoice-generate")?.detail).toBe("queue");
    expect(cmds.find((c) => c.label === "shipment-events")?.detail).toBe("topic");
    expect(cmds.find((c) => c.label === "Switch to Logistics dev")?.production).toBe(false);
    expect(labels).toContain("Connections: Switch to Logistics dev");
    expect(labels).not.toContain("Connections: Switch to Logistics");
    expect(cmds.find((c) => c.label === "Toggle Jobs and Activity")?.shortcut).toBe("mod+j");
  });

  it("offers the dashboard while connected, and not otherwise", () => {
    const c = ctx();
    const cmd = buildCommands(c).find((x) => x.label === "Open dashboard");
    expect(cmd?.shortcut).toBe("mod+shift+d");
    cmd!.run();
    expect(c.open).toHaveBeenCalledWith({ id: "::dashboard", kind: "dashboard" });
    expect(buildCommands({ ...c, activeProfileId: null }).some((x) => x.label === "Open dashboard")).toBe(false);
  });

  it("lists a pinned entity once even when the namespace lists it too", () => {
    const c = ctx();
    const cmds = buildCommands({ ...c, listing: { ...c.listing, known: [{ entity: "invoice-generate", subscription: null }] } });
    expect(cmds.filter((x) => x.label === "invoice-generate")).toHaveLength(1);
    expect(new Set(cmds.map((x) => x.id)).size).toBe(cmds.length);
  });

  it("opens the explorer options while connected only", () => {
    const c = ctx();
    buildCommands(c).find((x) => x.label === "Explorer options")!.run();
    expect(c.openSheet).toHaveBeenCalledWith({ kind: "explorer-options" });
    expect(buildCommands({ ...c, activeProfileId: null }).some((x) => x.label === "Explorer options")).toBe(false);
  });

  it("copies the URL and the connection string of the entity in the active tab", () => {
    const c = { ...ctx(), activeTab: { id: "shipment-events/subscriptions/tracking-sync", kind: "subscription" as const } };
    const cmds = buildCommands(c);
    cmds.find((x) => x.label === "Copy URL of tracking-sync (shipment-events)")!.run();
    expect(c.copy).toHaveBeenCalledWith("sb://sb-logistics-prod.servicebus.windows.net/shipment-events/Subscriptions/tracking-sync", "Copied the URL for tracking-sync (shipment-events)");
    cmds.find((x) => x.label === "Copy connection string of tracking-sync (shipment-events)")!.run();
    expect(c.copyConnectionString).toHaveBeenCalledWith("shipment-events/subscriptions/tracking-sync");
  });

  it("offers no entity copy commands without an entity tab, and no connection string on Entra", () => {
    const c = ctx();
    expect(buildCommands(c).some((x) => x.group === "This entity")).toBe(false);
    const entra = buildCommands({ ...c, entra: true, activeTab: { id: "orders", kind: "queue" } });
    expect(entra.map((x) => x.label)).toContain("Copy URL of orders");
    expect(entra.some((x) => /connection string/.test(x.label))).toBe(false);
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
    buildCommands(c).find((x) => x.label === "tracking-sync")!.run();
    expect(c.open).toHaveBeenCalledWith({ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription" });
  });
});

describe("palette", () => {
  it("opens from the small search button that stands in for the field in a narrow window", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(screen.getByRole("button", { name: "Open the command palette" }));
    expect(await screen.findByPlaceholderText("Go to entity or command")).toBeInTheDocument();
  });

  it("opens with mod+k and opens an entity", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const input = await screen.findByPlaceholderText("Go to entity or command");
    await userEvent.type(input, "label-print");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true"));
    expect(screen.queryByPlaceholderText("Go to entity or command")).not.toBeInTheDocument();
    // The tree follows: the opened entity is the selected row.
    expect(screen.getByRole("treeitem", { name: /^label-print(,| \(|$)/ })).toHaveAttribute("aria-selected", "true");
  });

  it("groups commands, shows shortcuts and says so when nothing matches", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const input = await screen.findByPlaceholderText("Go to entity or command");
    expect(screen.getByText("Go to")).toBeInTheDocument();
    expect(screen.getByText("Connections")).toBeInTheDocument();
    expect(screen.getByText("App")).toBeInTheDocument();
    expect(screen.getByText("Ctrl+J")).toBeInTheDocument();
    await userEvent.type(input, "zzzzqq");
    expect(await screen.findByText('No results for "zzzzqq"')).toBeInTheDocument();
  });

  it("closes with Escape and mod+k, and mod+k works while typing in the filter", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await act(async () => screen.getByRole("searchbox", { name: "Filter entities" }).focus());
    await userEvent.keyboard("{Control>}k{/Control}");
    await screen.findByPlaceholderText("Go to entity or command");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByPlaceholderText("Go to entity or command")).not.toBeInTheDocument());
    await userEvent.keyboard("{Control>}k{/Control}");
    await screen.findByPlaceholderText("Go to entity or command");
    await userEvent.keyboard("{Control>}k{/Control}");
    await waitFor(() => expect(screen.queryByPlaceholderText("Go to entity or command")).not.toBeInTheDocument());
  });

  it("marks production connections and aligns every label to one left edge", async () => {
    renderShell("default", { last_profile: "p-logistics-dev" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(await screen.findByPlaceholderText("Go to entity or command"), "Switch to");
    const prod = await screen.findByRole("option", { name: /Switch to Logistics\b(?! dev)/ });
    expect(within(prod).getByText("PROD").className).toContain("text-accent");
    const dev = screen.getByRole("option", { name: /Switch to Partner gateway/ });
    expect(within(dev).queryByText("PROD")).not.toBeInTheDocument();
    // A command with no glyph keeps a spacer where the glyph would be.
    expect(prod.firstElementChild?.className).toContain("size-3.5");
    expect(prod.firstElementChild?.tagName).toBe("SPAN");
    expect(prod.firstElementChild?.textContent).toBe("");
  });

  it("searches subscriptions of collapsed topics, by name and by topic, without a notice", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const input = await screen.findByPlaceholderText("Go to entity or command");
    await userEvent.type(input, "tracking");
    const row = await screen.findByRole("option", { name: /tracking-sync/ });
    expect(within(row).getByText("shipment-events").className).toContain("text-muted");
    expect(screen.queryByText(/aren't searched/)).not.toBeInTheDocument();
    await userEvent.clear(input);
    await userEvent.type(input, "shipment-events");
    expect(await screen.findByRole("option", { name: /tracking-sync/ })).toBeInTheDocument();
    await userEvent.clear(input);
    await userEvent.type(input, "tracking");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByRole("tab", { name: /tracking-sync/ })).toHaveAttribute("aria-selected", "true"));
    expect(screen.getByRole("treeitem", { name: /^tracking-sync(,| \(|$)/ })).toHaveAttribute("aria-selected", "true");
  });

  it("with more than 50 topics, loads no subscriptions and says collapsed topics are not searched", async () => {
    const { mock } = renderShell("many-topics", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    expect(await screen.findByText("Subscriptions of collapsed topics aren't searched. Expand a topic to include them.")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "list_subscriptions")).toBe(false);
  });

  it("echoes the query when nothing matches", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(await screen.findByPlaceholderText("Go to entity or command"), "zzzzqq");
    expect(await screen.findByText('No results for "zzzzqq"')).toBeInTheDocument();
  });

  it("says only commands run when not connected", async () => {
    renderShell("default");
    expect(await screen.findByRole("button", { name: /Run a command/ })).toBeInTheDocument();
    await userEvent.keyboard("{Control>}k{/Control}");
    expect(await screen.findByPlaceholderText("Run a command")).toBeInTheDocument();
  });

  it("has density-following rows", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    const row = await screen.findByRole("option", { name: /Open settings/ });
    expect(row.className).toContain("h-row");
    expect(row.className).not.toContain("h-7");
  });

  it("toggles the drawer with mod+j", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    expect(await screen.findByRole("region", { name: "Jobs and activity" })).toBeInTheDocument();
    await userEvent.keyboard("{Control>}j{/Control}");
    await waitFor(() => expect(screen.queryByRole("region", { name: "Jobs and activity" })).not.toBeInTheDocument());
    await userEvent.keyboard("{Control>}j{/Control}");
    expect(await screen.findByRole("region", { name: "Jobs and activity" })).toBeInTheDocument();
  });
});
