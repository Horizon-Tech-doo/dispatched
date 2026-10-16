import { useMemo } from "react";
import { MAX_LINE, MAX_LINES, splitLines } from "./format";
import { NoticeRow, VirtualLines } from "./VirtualLines";

/**
 * Read-only text, one virtualised row per line, with a line-number gutter. No syntax colours.
 * Lines past `MAX_LINE` characters are cut, with a notice pinned to the visible edge of that row.
 * Past `maxLines` lines the rest is left out and a last row says how many. `notice` replaces that text
 * and is also shown, as a last row, when nothing was left out (for example a body truncated upstream).
 */
export function CodeView({ text, label, height, notice, maxLines = MAX_LINES }: {
  text: string;
  label: string;
  height?: number;
  notice?: string;
  maxLines?: number;
}) {
  const { lines, more } = useMemo(() => splitLines(text, maxLines), [text, maxLines]);
  const note = notice ?? (more > 0 ? `${more.toLocaleString("en-US")} more ${more === 1 ? "line" : "lines"} not shown. Copy the body for all of it.` : null);
  const gutter = `calc(${String(lines.length + more).length}ch + 24px)`;
  return (
    <VirtualLines
      label={label}
      height={height}
      resetKey={text}
      count={lines.length + (note ? 1 : 0)}
      renderRow={(i) => {
        if (i >= lines.length) return <NoticeRow>{note}</NoticeRow>;
        const line = lines[i];
        const cut = line.length > MAX_LINE;
        return (
          <>
            <span aria-hidden="true" className="sticky left-0 shrink-0 select-none border-r border-line bg-side px-3 text-right tabular-nums text-faint" style={{ width: gutter }}>
              {i + 1}
            </span>
            <span data-line className="pl-3 pr-4 text-fg">{cut ? line.slice(0, MAX_LINE) : line}</span>
            {cut && (
              <span className="sticky right-0 shrink-0 select-none border-l border-line bg-content pl-2 pr-3 text-muted">
                Line shortened. Copy the body for all of it.
              </span>
            )}
          </>
        );
      }}
    />
  );
}
