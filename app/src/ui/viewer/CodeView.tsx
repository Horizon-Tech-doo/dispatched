import { useMemo } from "react";
import { MAX_LINE, splitLines } from "./format";
import { VirtualLines } from "./VirtualLines";

/** Read-only text, one virtualised row per line, with a line-number gutter. No syntax colours. */
export function CodeView({ text, label, height }: { text: string; label: string; height?: number }) {
  const { lines, more } = useMemo(() => splitLines(text), [text]);
  const total = lines.length + more;
  const gutter = `calc(${String(total).length}ch + 24px)`;
  return (
    <VirtualLines
      label={label}
      height={height}
      resetKey={text}
      count={lines.length + (more > 0 ? 1 : 0)}
      renderRow={(i) => {
        if (i >= lines.length) {
          return (
            <>
              <span aria-hidden="true" className="sticky left-0 shrink-0 select-none border-r border-line bg-side" style={{ width: gutter }} />
              <span className="select-none px-3 text-muted">
                … {more.toLocaleString("en-US")} more {more === 1 ? "line" : "lines"} not shown; copy the body for all of it
              </span>
            </>
          );
        }
        const line = lines[i];
        const cut = line.length > MAX_LINE;
        return (
          <>
            <span aria-hidden="true" className="sticky left-0 shrink-0 select-none border-r border-line bg-side px-3 text-right tabular-nums text-faint" style={{ width: gutter }}>
              {i + 1}
            </span>
            <span data-line className="pl-3 pr-4 text-fg">{cut ? line.slice(0, MAX_LINE) : line}</span>
            {cut && <span className="select-none pr-4 text-muted">… line shortened; copy the body for all of it</span>}
          </>
        );
      }}
    />
  );
}
