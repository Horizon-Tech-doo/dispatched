import type { EntityCounts } from "@/engine";
import type { LabelColor } from "@/ui";
import type { CountKind, HighlightRule } from "../state";
import { compilePattern } from "./safeRegex";

export type Compiled = { color: LabelColor; test(name: string, c: EntityCounts | null | undefined): boolean };

const value = (c: EntityCounts, k: CountKind) =>
  k === "active" ? c.active : k === "dead-letter" ? c.dead_letter : k === "scheduled" ? c.scheduled : c.transfer_dead_letter;

/** What is wrong with a rule, in plain words; undefined when it can run. The same check the options sheet shows. */
export function ruleProblem(r: HighlightRule): string | undefined {
  if (r.match.kind === "count") return Number.isInteger(r.match.atLeast) && r.match.atLeast >= 0 ? undefined : "Enter a whole number, 0 or more.";
  if (!r.match.pattern.trim()) return "Enter a pattern, such as ^orders- or -dlq$.";
  const compiled = compilePattern(r.match.pattern);
  return "error" in compiled ? `Not a valid pattern: ${compiled.error}.` : undefined;
}

/** The rules that can run, in order. A rule with a bad pattern highlights nothing. Name patterns run on a matcher
 *  that cannot backtrack (see `safeRegex`), so no pattern can stall the tree. */
export function compileRules(rules: HighlightRule[]): Compiled[] {
  const out: Compiled[] = [];
  for (const r of rules) {
    if (r.match.kind === "name") {
      const compiled = compilePattern(r.match.pattern);
      if (!("error" in compiled)) out.push({ color: r.color, test: (name) => compiled.test(name) });
    } else {
      const { count, atLeast } = r.match;
      if (Number.isFinite(atLeast)) out.push({ color: r.color, test: (_n, c) => !!c && value(c, count) >= atLeast });
    }
  }
  return out;
}

/** The colour of the first rule that matches the row, if any. */
export function highlightOf(name: string, counts: EntityCounts | null | undefined, rules: Compiled[]): LabelColor | null {
  return rules.find((r) => r.test(name, counts))?.color ?? null;
}
