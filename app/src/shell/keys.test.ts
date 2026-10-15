import { describe, expect, it } from "vitest";
import { matches } from "./keys";

const ev = (key: string, mods: Partial<KeyboardEvent> = {}) => new KeyboardEvent("keydown", { key, ...mods });

describe("matches", () => {
  it("maps mod to Cmd on macOS and Ctrl elsewhere", () => {
    expect(matches(ev("k", { metaKey: true }), "mod+k", true)).toBe(true);
    expect(matches(ev("k", { ctrlKey: true }), "mod+k", true)).toBe(false);
    expect(matches(ev("k", { ctrlKey: true }), "mod+k", false)).toBe(true);
  });

  it("requires exact modifiers", () => {
    expect(matches(ev("k", { metaKey: true, shiftKey: true }), "mod+k", true)).toBe(false);
    expect(matches(ev("P", { metaKey: true, shiftKey: true }), "mod+shift+p", true)).toBe(true);
    expect(matches(ev("F5"), "f5")).toBe(true);
    expect(matches(ev("3", { metaKey: true }), "mod+3", true)).toBe(true);
    expect(matches(ev(",", { metaKey: true }), "mod+,", true)).toBe(true);
  });
});
