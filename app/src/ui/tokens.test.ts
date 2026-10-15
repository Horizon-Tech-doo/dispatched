import { describe, expect, it } from "vitest";
import { contrastPairs, labelColors, roles, themes } from "./tokens";
import { contrastRatio } from "./contrast";

describe("tokens", () => {
  it("define every role in both themes as a 6-digit hex", () => {
    for (const name of ["dark", "light"] as const) {
      for (const role of roles) expect(themes[name][role], `${name}.${role}`).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("meet WCAG contrast for every text-on-surface pair", () => {
    const failures: string[] = [];
    for (const name of ["dark", "light"] as const) {
      for (const { text, on, min } of contrastPairs) {
        const r = contrastRatio(themes[name][text], themes[name][on]);
        if (r < min) failures.push(`${name}: ${text} on ${on} = ${r.toFixed(2)} < ${min}`);
      }
    }
    expect(failures).toEqual([]);
  });

  it("computes known ratios", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(contrastRatio("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
  });
});

describe("label colours", () => {
  it("are distinguishable (3:1) on the side, field, content and chrome surfaces in both themes", () => {
    for (const [name, c] of Object.entries(labelColors)) {
      for (const theme of ["dark", "light"] as const) {
        for (const surface of ["side", "field", "content", "chrome"] as const) {
          expect(contrastRatio(c[theme], themes[theme][surface]), `${name} ${theme} on ${surface}`).toBeGreaterThanOrEqual(3);
        }
      }
    }
  });
});
