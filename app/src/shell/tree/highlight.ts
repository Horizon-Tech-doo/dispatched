import type { EntityCounts } from "@/engine";
import type { LabelColor } from "@/ui";
import type { CountKind, HighlightRule } from "../state";
import { compilePattern } from "./safeRegex";

export type Compiled = { color: LabelColor; /** The rule in words, for a tooltip and a screen reader: "dead-letter at least 200". */ description: string; test(name: string, c: EntityCounts | null | undefined): boolean };
export type Highlight = { color: LabelColor; description: string };

const COUNT_WORDS: Record<CountKind, string> = { active: "active", "dead-letter": "dead-letter", scheduled: "scheduled", "transfer-dlq": "transfer DLQ" };

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
 *  that cannot backtrack (see `safeRegex`) and has a compile budget, so no pattern, not even a hand-edited setting,
 *  can stall the tree. A name's answer is remembered, so scrolling does not run the matcher again. */
export function compileRules(rules: HighlightRule[]): Compiled[] {
  const out: Compiled[] = [];
  for (const r of rules) {
    if (r.match.kind === "name") {
      const compiled = compilePattern(r.match.pattern);
      if ("error" in compiled) continue;
      const known = new Map<string, boolean>();
      out.push({
        color: r.color,
        description: `name matches ${r.match.pattern}`,
        test: (name) => {
          let hit = known.get(name);
          if (hit === undefined) {
            hit = compiled.test(name);
            if (known.size >= 5000) known.clear();
            known.set(name, hit);
          }
          return hit;
        },
      });
    } else {
      const { count, atLeast } = r.match;
      if (Number.isFinite(atLeast)) out.push({ color: r.color, description: `${COUNT_WORDS[count]} at least ${atLeast}`, test: (_n, c) => !!c && value(c, count) >= atLeast });
    }
  }
  return out;
}

/** The first rule that matches the row, if any. */
export function highlightOf(name: string, counts: EntityCounts | null | undefined, rules: Compiled[]): Highlight | null {
  const hit = rules.find((r) => r.test(name, counts));
  return hit ? { color: hit.color, description: hit.description } : null;
}
