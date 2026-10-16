import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";
import { ruleKind } from "./RulesView";

describe("rules", () => {
  it("shows SQL, default and correlation rules read-only", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [{ id: "shipment-events/subscriptions/tracking-sync", kind: "subscription", subTab: "rules" }], activeTab: "shipment-events/subscriptions/tracking-sync", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
    });
    const list = await screen.findByRole("list", { name: "Rules" });
    const def = within(list).getByRole("listitem", { name: "$Default" });
    expect(within(def).getByText("Matches every message")).toBeInTheDocument();
    const eu = within(list).getByRole("listitem", { name: "eu-only" });
    expect(within(eu).getByText("region = 'eu' AND sys.Label LIKE 'shipment%'")).toBeInTheDocument();
    expect(within(eu).getByText("SET priority = 'high'")).toBeInTheDocument();
    const prio = within(list).getByRole("listitem", { name: "priority" });
    expect(within(prio).getByText("batch-7")).toBeInTheDocument();
    expect(within(prio).getByText("tier")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Edit|Delete|Add rule/ })).toBeNull();
  });

  it("flags a correlation filter that sets nothing and an unknown filter", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [{ id: "carrier-updates/subscriptions/eta-recalc", kind: "subscription", subTab: "rules" }], activeTab: "carrier-updates/subscriptions/eta-recalc", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
    });
    const list = await screen.findByRole("list", { name: "Rules" });
    const unset = within(list).getByRole("listitem", { name: "unset" });
    expect(within(unset).getByRole("heading", { name: "This rule matches nothing" })).toBeInTheDocument();
    const legacy = within(list).getByRole("listitem", { name: "legacy-route" });
    expect(within(legacy).getByText("Unknown filter")).toBeInTheDocument();
  });

  it("names filter kinds", () => {
    expect(ruleKind({ kind: "sql", expression: "1=0" })).toBe("Matches no message");
    expect(ruleKind({ kind: "sql", expression: " 1 = 1 " })).toBe("Matches every message");
    expect(ruleKind({ kind: "sql", expression: "a = 1" })).toBe("SQL filter");
    expect(ruleKind({ kind: "unknown" })).toBe("Unknown filter");
  });
});
