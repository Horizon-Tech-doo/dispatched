import type { FilterField, FilterSpec } from "@/engine";

/** What to say about one engine problem: a plain sentence, and the engine's own words (without its `filter:`
 *  prefix and backticks) for the mono detail line. */
export function describeIssue(field: FilterField, message: string, typed: string): { sentence: string; detail: string } {
  const detail = message.replace(/^filter: /, "").replace(/`/g, "");
  const column = message.match(/at column (\d+)/)?.[1];
  switch (field) {
    case "property":
      if (typed.includes("==") && /unexpected `=`/.test(message)) return { sentence: "Use = to compare.", detail };
      return { sentence: column ? `This expression can't be read. Check it at column ${column}, then apply.` : "This expression can't be read. Check it, then apply.", detail };
    case "body_regex":
      return { sentence: "This regular expression can't be read. Check it, then apply.", detail };
    case "body_json_path":
      return { sentence: "This JSONPath can't be read. It starts with $, for example $.order.status.", detail };
    case "body_json_equals":
      return { sentence: "Add a JSONPath to compare this value with, or clear it.", detail };
  }
}

/** The applied conditions on one line, for the collapsed filter: `attempt >= 2 · body contains "held"`. */
export function filterSummary(f: FilterSpec): string {
  const set = (v: string | null | undefined): v is string => typeof v === "string" && v.trim() !== "";
  const parts: string[] = [];
  if (set(f.property)) parts.push(f.property.trim());
  if (set(f.body_contains)) parts.push(`body contains "${f.body_contains}"`);
  else if (set(f.body_regex)) parts.push(`body regex ${f.body_regex}`);
  else if (set(f.body_json_path)) parts.push(set(f.body_json_equals) ? `${f.body_json_path.trim()} = ${f.body_json_equals.trim()}` : `${f.body_json_path.trim()} exists`);
  if (set(f.session_id)) parts.push(`session ${f.session_id}`);
  return parts.join(" · ");
}
