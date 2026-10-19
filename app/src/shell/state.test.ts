import { describe, expect, it } from "vitest";
import { emptyShell, parsePrefs, parseProfileShell, shellReducer, type ProfileShell } from "./state";

const open = (s: ProfileShell, id: string, kind: "queue" | "subscription" = "queue") => shellReducer(s, { type: "open", tab: { id, kind } });

describe("shell reducer", () => {
  it("opens new tabs after the active one and focuses existing ones", () => {
    let s = open(emptyShell, "orders");
    s = open(s, "shipment-events/subscriptions/tracking-sync", "subscription");
    expect(s.tabs.map((t) => t.id)).toEqual(["orders", "shipment-events/subscriptions/tracking-sync"]);
    expect(s.activeTab).toBe("shipment-events/subscriptions/tracking-sync");
    s = open(s, "orders");
    expect(s.tabs).toHaveLength(2);
    expect(s.activeTab).toBe("orders");
    expect(s.tabs[0].subTab).toBe("overview");
  });

  it("closing the active tab activates its right neighbour, else the left", () => {
    let s = ["a", "b", "c"].reduce((acc, id) => open(acc, id), emptyShell);
    s = shellReducer(s, { type: "activate", id: "b" });
    s = shellReducer(s, { type: "close", id: "b" });
    expect(s.activeTab).toBe("c");
    s = shellReducer(s, { type: "close", id: "c" });
    expect(s.activeTab).toBe("a");
    s = shellReducer(s, { type: "close", id: "a" });
    expect(s.activeTab).toBeNull();
  });

  it("switches by index and remembers the sub-tab per tab", () => {
    let s = ["a", "b"].reduce((acc, id) => open(acc, id), emptyShell);
    s = shellReducer(s, { type: "subTab", id: "a", subTab: "dead-letter" });
    s = shellReducer(s, { type: "activateIndex", index: 0 });
    expect(s.activeTab).toBe("a");
    expect(s.tabs[0].subTab).toBe("dead-letter");
    expect(shellReducer(s, { type: "activateIndex", index: 7 })).toBe(s);
  });

  it("tracks expanded nodes and the tree selection", () => {
    let s = shellReducer(emptyShell, { type: "toggleExpanded", nodeId: "t:shipment-events", expanded: true });
    expect(s.expanded).toContain("t:shipment-events");
    s = shellReducer(s, { type: "toggleExpanded", nodeId: "t:shipment-events", expanded: false });
    expect(s.expanded).not.toContain("t:shipment-events");
    expect(shellReducer(s, { type: "select", nodeId: "q:orders" }).selected).toBe("q:orders");
  });
});

describe("persisted state", () => {
  it("bad_persisted_state_falls_back_to_defaults", () => {
    expect(parseProfileShell(null)).toEqual(emptyShell);
    expect(parseProfileShell("garbage")).toEqual(emptyShell);
    const s = parseProfileShell({
      tabs: [{ id: "orders", kind: "queue", subTab: "dead-letter" }, { id: 3 }, { id: "x", kind: "nope", subTab: "overview" }, { id: "y", kind: "queue", subTab: "bogus" }],
      activeTab: "missing",
      expanded: ["t:a", 4],
      selected: 9,
      layout: { treeSize: 900, drawerOpen: "yes", drawerSize: 25, drawerTab: "jobs", detailSize: 5 },
    });
    expect(s.tabs).toEqual([
      { id: "orders", kind: "queue", subTab: "dead-letter" },
      { id: "y", kind: "queue", subTab: "overview" },
    ]);
    expect(s.activeTab).toBe("orders");
    expect(s.expanded).toEqual(["t:a"]);
    expect(s.selected).toBeNull();
    expect(s.layout).toEqual({ treeSize: emptyShell.layout.treeSize, drawerOpen: true, drawerSize: 25, drawerTab: "jobs", detailSize: 38 });
    expect(parseProfileShell({ layout: { detailSize: 55 } }).layout.detailSize).toBe(55);
  });

  it("keeps a dashboard tab only under its own id", () => {
    const tabs = parseProfileShell({ tabs: [{ id: "::dashboard", kind: "dashboard" }, { id: "orders", kind: "dashboard" }] }).tabs;
    expect(tabs).toEqual([{ id: "::dashboard", kind: "dashboard", subTab: "overview" }]);
  });

  it("prefs fall back per field", () => {
    expect(parsePrefs({ theme: "light", density: "huge", autoRefresh: 30, peekSize: 500, dashboardRefresh: 60 })).toEqual({ theme: "light", density: "compact", autoRefresh: 30, peekSize: 500, dashboardRefresh: 60 });
    expect(parsePrefs(undefined)).toEqual({ theme: "system", density: "compact", autoRefresh: 0, peekSize: 100, dashboardRefresh: 0 });
    expect(parsePrefs({ peekSize: 5000 }).peekSize).toBe(100);
    expect(parsePrefs({ dashboardRefresh: 45 }).dashboardRefresh).toBe(0);
  });
});

describe("layout action", () => {
  it("returns the same state when the sizes did not change", () => {
    expect(shellReducer(emptyShell, { type: "layout", patch: { treeSize: emptyShell.layout.treeSize } })).toBe(emptyShell);
    expect(shellReducer(emptyShell, { type: "layout", patch: { treeSize: 30 } }).layout.treeSize).toBe(30);
  });
});

describe("explorer options", () => {
  const rule = (over: Record<string, unknown> = {}) => ({ id: "r", color: "rose", match: { kind: "count", count: "dead-letter", atLeast: 100 }, ...over });

  it("defaults the explorer options and the tree filter", () => {
    expect(emptyShell.explorer).toEqual({ counts: ["active", "dead-letter"], rules: [], openOn: "overview" });
    expect(emptyShell.treeFilter).toBe("");
    const s = parseProfileShell({ tabs: [] });
    expect(s.explorer).toEqual(emptyShell.explorer);
    expect(s.treeFilter).toBe("");
  });

  it("drops rules with unknown colours or kinds and caps them at 20", () => {
    const s = parseProfileShell({
      explorer: {
        counts: ["active"],
        rules: [
          rule(),
          rule({ id: "bad-colour", color: "amber" }),
          rule({ id: "bad-kind", match: { kind: "regex", pattern: "x" } }),
          rule({ id: "bad-count", match: { kind: "count", count: "transfer", atLeast: 1 } }),
          rule({ id: "negative", match: { kind: "count", count: "active", atLeast: -1 } }),
          rule({ id: "nan", match: { kind: "count", count: "active", atLeast: "5" } }),
          rule({ id: "long", match: { kind: "name", pattern: "x".repeat(201) } }),
          rule({ id: "name", color: "blue", match: { kind: "name", pattern: "^orders" } }),
          { color: "rose", match: { kind: "name", pattern: "no id" } },
        ],
      },
    });
    expect(s.explorer.rules.map((r) => r.id)).toEqual(["r", "name"]);
    const many = parseProfileShell({ explorer: { rules: Array.from({ length: 30 }, (_, i) => rule({ id: `r${i}` })) } });
    expect(many.explorer.rules).toHaveLength(20);
  });

  it("drops a count rule with a fractional threshold and repeated rule ids", () => {
    const s = parseProfileShell({
      explorer: {
        rules: [
          rule({ id: "frac", match: { kind: "count", count: "active", atLeast: 1.5 } }),
          rule({ id: "dup" }),
          rule({ id: "dup", color: "blue", match: { kind: "name", pattern: "x" } }),
          rule({ id: "ok", match: { kind: "count", count: "active", atLeast: 0 } }),
        ],
      },
    });
    expect(s.explorer.rules.map((r) => r.id)).toEqual(["dup", "ok"]);
    expect(s.explorer.rules[0].color).toBe("rose");
  });

  it("keeps the counts that are known, and openOn only from its three values", () => {
    expect(parseProfileShell({ explorer: { counts: ["scheduled", "transfer", "dead-letter", 4] } }).explorer.counts).toEqual(["scheduled", "dead-letter"]);
    expect(parseProfileShell({ explorer: { counts: ["bogus"] } }).explorer.counts).toEqual(["active", "dead-letter"]);
    expect(parseProfileShell({ explorer: { counts: ["active", "active"] } }).explorer.counts).toEqual(["active"]);
    expect(parseProfileShell({ explorer: { openOn: "messages" } }).explorer.openOn).toBe("messages");
    expect(parseProfileShell({ explorer: { openOn: "dead-letter" } }).explorer.openOn).toBe("dead-letter");
    expect(parseProfileShell({ explorer: { openOn: "transfer-dlq" } }).explorer.openOn).toBe("overview");
  });

  it("keeps a tree filter up to 200 characters", () => {
    expect(parseProfileShell({ treeFilter: "invoice" }).treeFilter).toBe("invoice");
    expect(parseProfileShell({ treeFilter: "x".repeat(201) }).treeFilter).toBe("");
    expect(parseProfileShell({ treeFilter: 4 }).treeFilter).toBe("");
  });

  it("opens entities on the connection's sub-tab, and topics and the dashboard on Overview", () => {
    let s = shellReducer(emptyShell, { type: "explorer", options: { counts: ["active"], rules: [], openOn: "dead-letter" } });
    s = shellReducer(s, { type: "open", tab: { id: "orders", kind: "queue" } });
    s = shellReducer(s, { type: "open", tab: { id: "events", kind: "topic" } });
    s = shellReducer(s, { type: "open", tab: { id: "::dashboard", kind: "dashboard" } });
    s = shellReducer(s, { type: "open", tab: { id: "billing", kind: "queue", subTab: "messages" } });
    const sub = (id: string) => s.tabs.find((t) => t.id === id)?.subTab;
    expect([sub("orders"), sub("events"), sub("::dashboard"), sub("billing")]).toEqual(["dead-letter", "overview", "overview", "messages"]);
  });

  it("sets the tree filter", () => {
    expect(shellReducer(emptyShell, { type: "treeFilter", value: "inv" }).treeFilter).toBe("inv");
  });
});
