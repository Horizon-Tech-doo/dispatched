import { labelColors, type LabelColor } from "../tokens";

/** An 8px identification swatch for a connection's label; renders nothing for none or an unknown colour. */
export function LabelSwatch({ color }: { color: string | null | undefined }) {
  if (!color || !(color in labelColors)) return null;
  return <span aria-hidden="true" className="inline-block size-2 shrink-0 rounded-[2px]" style={{ background: `var(--label-${color as LabelColor})` }} />;
}
