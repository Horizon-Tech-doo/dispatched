import { describe, expect, it } from "vitest";
import { applyTheme, resolveTheme, rowHeight } from "./theme";
import { themes } from "./tokens";

describe("theme", () => {
  it("resolves system to the OS preference", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
  });

  it("writes role variables, theme and row height to the root", () => {
    const root = document.createElement("div");
    applyTheme("light", "comfortable", root);
    expect(root.style.getPropertyValue("--c-accent")).toBe(themes.light.accent);
    expect(root.style.getPropertyValue("--c-accent-ink")).toBe(themes.light.accentInk);
    expect(root.dataset.theme).toBe("light");
    expect(root.style.getPropertyValue("--row")).toBe(`${rowHeight.comfortable}px`);
  });
});
