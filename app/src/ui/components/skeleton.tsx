import { cn } from "../lib/cn";

/** A static placeholder bar (no animation: only sheets, drawer and popovers move). */
export function Skeleton({ className }: { className?: string }) {
  return <span aria-hidden="true" className={cn("block h-2.5 rounded-[2px] bg-hover", className)} />;
}

const WIDTHS = ["w-[62%]", "w-[48%]", "w-[71%]", "w-[39%]", "w-[56%]", "w-[66%]", "w-[44%]", "w-[58%]"];

/** Rows shaped like the content that is loading. */
export function SkeletonRows({ rows = 8, columns = 1 }: { rows?: number; columns?: number }) {
  return (
    <div role="status" aria-label="Loading" className="flex flex-col">
      {Array.from({ length: rows }, (_, r) => (
        <div key={r} data-skeleton-row className="flex h-row items-center gap-4 px-3">
          {Array.from({ length: columns }, (_, c) => (
            <Skeleton key={c} className={cn(c === 0 ? WIDTHS[r % WIDTHS.length] : "w-10", c === 0 && "flex-1")} />
          ))}
        </div>
      ))}
    </div>
  );
}
