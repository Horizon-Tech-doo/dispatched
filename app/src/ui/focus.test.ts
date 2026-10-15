import { describe, expect, it } from "vitest";

// Tailwind 4: `outline-none` sets the outline style variable to none, so a `focus-visible:outline-1` on the
// same element draws nothing. A component that wants a focus ring must not also carry `outline-none`.
const sources = import.meta.glob("./**/*.tsx", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

/** The argument text of every `cn(...)` call, and every className string, in a source file. */
function classExpressions(text: string): string[] {
  const out: string[] = [];
  for (let at = text.indexOf("cn("); at !== -1; at = text.indexOf("cn(", at + 1)) {
    let depth = 0;
    let quote = "";
    let i = at + 2;
    for (; i < text.length; i++) {
      const ch = text[i];
      if (quote) {
        if (ch === quote) quote = "";
      } else if (ch === '"' || ch === "'" || ch === "`") quote = ch;
      else if (ch === "(") depth++;
      else if (ch === ")" && --depth === 0) break;
    }
    out.push(text.slice(at, i + 1));
  }
  out.push(...(text.match(/className="[^"\n]*"/g) ?? []));
  return out;
}

describe("focus rings", () => {
  it("no class expression combines outline-none with a focus-visible outline", () => {
    const offenders: string[] = [];
    for (const [file, text] of Object.entries(sources)) {
      for (const expr of classExpressions(text)) {
        if (/(^|[\s"])outline-none(\s|")/.test(expr) && /focus-visible:outline(?!-offset)/.test(expr)) offenders.push(`${file}: ${expr.slice(0, 80)}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it("segmented options and tab triggers carry a focus-visible outline and no outline-none", () => {
    for (const file of ["./components/segmented.tsx", "./components/tabs.tsx"]) {
      expect(sources[file], file).toMatch(/focus-visible:outline focus-visible:outline-1/);
      expect(sources[file], file).not.toMatch(/\boutline-none\b/);
    }
  });
});
