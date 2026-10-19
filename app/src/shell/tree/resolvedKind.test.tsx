import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";

describe("resolved kinds", () => {
  it("pinned entity shows topic glyph after resolve", async () => {
    const { mock } = renderShell("forbidden", { last_profile: "p-logistics" });
    const tree = await screen.findByRole("tree", { name: "Namespace" });
    const row = await within(tree).findByRole("treeitem", { name: /^orders/ });
    await waitFor(() => expect(row.querySelector('svg[data-kind="topic"]')).not.toBeNull());
    expect(mock.calls.filter((c) => c.cmd === "resolve_kind" && (c.args as { entity: string }).entity === "orders")).toHaveLength(1);
    // A subscription's kind is in its id: it is not asked for.
    expect(mock.calls.some((c) => c.cmd === "resolve_kind" && String((c.args as { entity: string }).entity).includes("/subscriptions/"))).toBe(false);
  });
});
