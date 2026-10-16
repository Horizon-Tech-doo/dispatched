import type { CountKind } from "../state";

/** Width of `1ch` in the 12px UI text (7.6px in WebKit), rounded up: a row is never planned tighter than it is. */
export const CH_PX = 8;
const NAME_MIN_CH = 6;
/** Left of a row's name: padding, 14px of indent per level, the chevron column, and the glyph with its gaps. */
const nameStart = (depth: number) => 6 + depth * 14 + 14 + 4 + 14 + 6;
/** Right of the name: the marks' left padding, and the row's right padding. */
const EDGES = 4 + 10;
const GAP_PX = 8;
const SWATCH_PX = 8;

/** The widest a row of this depth can need to show every one of `kinds` in a fixed slot (`widths`, in characters) with
 *  the name still holding its minimum, and the swatch slot when the connection has a rule. */
export function rowNeed(depth: number, kinds: readonly CountKind[], widths: Record<CountKind, number>, swatch: boolean): number {
  const slots = kinds.reduce((sum, k) => sum + widths[k] * CH_PX, 0);
  const parts = kinds.length + (swatch ? 1 : 0);
  return Math.ceil(nameStart(depth) + NAME_MIN_CH * CH_PX + slots + Math.max(0, parts - 1) * GAP_PX + (swatch ? SWATCH_PX : 0) + EDGES);
}

export interface MarksTier {
  /** Every chosen count shows. Otherwise scheduled and transfer DLQ give way first. */
  optional: boolean;
  /** The counts that show have fixed-width slots, so a count's place in the row tells which count it is. Otherwise each
   *  takes only the room its value needs and the name gives way down to a couple of characters. */
  fixed: boolean;
}

const OPTIONAL: readonly CountKind[] = ["scheduled", "transfer-dlq"];

/** What fits in a pane of `paneWidth` px for the deepest row shown. An unmeasured pane (0) is taken as wide. */
export function marksTier(paneWidth: number, depth: number, chosen: readonly CountKind[], widths: Record<CountKind, number>, swatch: boolean): MarksTier {
  if (paneWidth <= 0) return { optional: true, fixed: true };
  const optional = rowNeed(depth, chosen, widths, swatch) <= paneWidth;
  const base = chosen.filter((k) => !OPTIONAL.includes(k));
  return { optional, fixed: optional || rowNeed(depth, base, widths, swatch) <= paneWidth };
}
