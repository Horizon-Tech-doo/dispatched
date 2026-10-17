import { describe, expect, it } from "vitest";
import { treeMinPercent, treeShownSize } from "./treeLayout";

describe("tree layout in a narrow window", () => {
  it("raises the minimum share so the pane keeps 200 px, within 12 to 45 percent", () => {
    expect(treeMinPercent(1440)).toBe(14);
    expect(treeMinPercent(2560)).toBe(12);
    expect(treeMinPercent(640)).toBe(32);
    expect(treeMinPercent(300)).toBe(45);
  });
  it("shows the saved size, or the minimum when that is larger", () => {
    expect(treeShownSize(18, 14)).toBe(18);
    expect(treeShownSize(18, 32)).toBe(32);
  });
});
