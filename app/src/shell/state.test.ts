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

  it("prefs fall back per field", () => {
    expect(parsePrefs({ theme: "light", density: "huge", autoRefresh: 30, peekSize: 500 })).toEqual({ theme: "light", density: "compact", autoRefresh: 30, peekSize: 500 });
    expect(parsePrefs(undefined)).toEqual({ theme: "system", density: "compact", autoRefresh: 0, peekSize: 100 });
    expect(parsePrefs({ peekSize: 5000 }).peekSize).toBe(100);
  });
});

describe("layout action", () => {
  it("returns the same state when the sizes did not change", () => {
    expect(shellReducer(emptyShell, { type: "layout", patch: { treeSize: emptyShell.layout.treeSize } })).toBe(emptyShell);
    expect(shellReducer(emptyShell, { type: "layout", patch: { treeSize: 30 } }).layout.treeSize).toBe(30);
  });
});
