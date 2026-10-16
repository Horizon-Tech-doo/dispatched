import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, type ReactNode } from "react";
import { observeRectWithFloor } from "../lib/virtual";

export const LINE_HEIGHT = 16;

/** A one-line note in the last row of a body view (a cut, a limit). Pinned to the left edge so it shows without scrolling sideways. */
export function NoticeRow({ children }: { children: ReactNode }) {
  return <span className="sticky left-0 select-none px-3 text-muted">{children}</span>;
}

/**
 * The shared shell of the body views: a focusable, scrolling region with one absolutely placed row per visible line.
 * Rows are all `LINE_HEIGHT` tall, so a long body costs the same as a short one.
 * `height` is the viewport height assumed before the region is measured (first paint, tests); the region itself fills its parent.
 * `resetKey` returns the scroll position to the top when the content changes.
 * `header` is one sticky `LINE_HEIGHT` row above the lines (column titles).
 */
export function VirtualLines({ count, label, height, resetKey, header, renderRow }: {
  count: number;
  label: string;
  height?: number;
  resetKey: unknown;
  header?: ReactNode;
  renderRow(index: number): ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const rect = { width: 600, height: height ?? 400 };
  const v = useVirtualizer({
    count,
    getScrollElement: () => ref.current,
    estimateSize: () => LINE_HEIGHT,
    overscan: 20,
    // The header takes the first row of the list, so line rows start below it.
    paddingStart: header ? LINE_HEIGHT : 0,
    initialRect: rect,
    observeElementRect: observeRectWithFloor(rect),
  });
  useEffect(() => {
    const el = ref.current;
    if (el) {
      el.scrollTop = 0;
      el.scrollLeft = 0;
    }
  }, [resetKey]);
  return (
    <div
      ref={ref}
      role="region"
      aria-label={label}
      tabIndex={0}
      data-selectable
      className="h-full overflow-auto font-mono text-11 focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg"
    >
      <div className="relative w-max min-w-full" style={{ height: v.getTotalSize() }}>
        {header && (
          <div
            data-header
            className="sticky top-0 z-[1] flex w-max min-w-full whitespace-pre border-b border-line bg-content"
            style={{ height: LINE_HEIGHT, lineHeight: `${LINE_HEIGHT - 1}px` }}
          >
            {header}
          </div>
        )}
        {v.getVirtualItems().map((row) => (
          <div
            key={row.index}
            data-row
            className="absolute left-0 top-0 flex min-w-full whitespace-pre"
            style={{ height: LINE_HEIGHT, lineHeight: `${LINE_HEIGHT}px`, transform: `translateY(${row.start}px)`, tabSize: 4 }}
          >
            {renderRow(row.index)}
          </div>
        ))}
      </div>
    </div>
  );
}
