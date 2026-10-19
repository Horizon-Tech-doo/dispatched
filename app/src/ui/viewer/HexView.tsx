import { Fragment, useLayoutEffect, useRef, useState } from "react";
import { HEX_OFFSET_DIGITS, hexHeader, hexRow, hexWidthFor, type HexWidth } from "./format";
import { NoticeRow, VirtualLines } from "./VirtualLines";

/** Zero bytes are dimmed so runs of padding recede. A byte is exactly two hex digits, so "00" only ever occurs as a whole byte. */
function HexBytes({ hex }: { hex: string }) {
  return (
    <>
      {hex.split(/(00)/).map((part, i) => (
        <Fragment key={i}>{part === "00" ? <span className="text-faint">00</span> : part}</Fragment>
      ))}
    </>
  );
}

/** How many bytes a row holds: 16 while the offset, hex and text columns fit the view, else 8. The view is measured
 *  (its width and the width of a mono character), and measured again when it is resized. */
function useHexWidth() {
  const box = useRef<HTMLDivElement>(null);
  const probe = useRef<HTMLSpanElement>(null);
  const [width, setWidth] = useState<HexWidth>(16);
  useLayoutEffect(() => {
    const el = box.current;
    const ch = probe.current;
    if (!el || !ch) return;
    const measure = () => setWidth(hexWidthFor(el.clientWidth, ch.getBoundingClientRect().width / 10));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return { box, probe, width };
}

/** Any bytes as `offset  hex  |text|` rows of 16 (8 in a narrow view), virtualised, under a sticky column header.
 *  `notice` is shown as a last row. Only a view narrower than one 8-byte row scrolls sideways. */
export function HexView({ bytes, label, notice }: { bytes: Uint8Array; label: string; notice?: string }) {
  const { box, probe, width } = useHexWidth();
  const rows = Math.ceil(bytes.length / width);
  return (
    <div ref={box} className="relative h-full min-w-0">
      <span ref={probe} aria-hidden="true" className="pointer-events-none invisible absolute left-0 top-0 whitespace-pre font-mono text-11">
        0000000000
      </span>
      <VirtualLines
        label={label}
        resetKey={bytes}
        count={rows + (notice ? 1 : 0)}
        header={
          <>
            <span className="px-3 text-muted">{"Offset".padEnd(HEX_OFFSET_DIGITS, " ")}</span>
            <span className="pr-3 text-muted">{hexHeader(width)}</span>
            <span className="pr-4 text-muted">Text</span>
          </>
        }
        renderRow={(i) => {
          if (i >= rows) return <NoticeRow>{notice}</NoticeRow>;
          const r = hexRow(bytes, i * width, bytes.length, width);
          return (
            <>
              <span className="px-3 text-muted">{r.offset}</span>
              <span className="pr-3 text-fg"><HexBytes hex={r.hex} /></span>
              <span className="pr-4 text-muted">{r.ascii}</span>
            </>
          );
        }}
      />
    </div>
  );
}
