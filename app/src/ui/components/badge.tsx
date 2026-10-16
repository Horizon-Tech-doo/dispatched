import type { ComponentProps } from "react";
import { cn } from "../lib/cn";
import { formatCount, type CountQuality } from "../lib/format";

const tones = {
  neutral: "border border-line text-muted",
  accent: "bg-accent text-accent-ink",
  attention: "bg-accent-tint text-accent",
} as const;

export function Badge({ tone = "neutral", className, ...props }: ComponentProps<"span"> & { tone?: keyof typeof tones }) {
  return (
    <span
      className={cn("inline-flex h-4 items-center rounded-[2px] px-1 text-11 font-semibold leading-4 tracking-[0.04em]", tones[tone], className)}
      {...props}
    />
  );
}

export function ProdBadge({ className }: { className?: string }) {
  return (
    <Badge tone="accent" title="Production profile" className={className}>
      PROD
    </Badge>
  );
}

/** A count in a tree row or tab: `?` unknown, `~N` approximate, amber when `attention` and above zero. */
export function CountBadge({
  value,
  quality,
  attention,
  className,
  title,
}: {
  value: number | null | undefined;
  quality?: CountQuality;
  attention?: boolean;
  className?: string;
  /** What is counted, as a tooltip, when several counts stand side by side. */
  title?: string;
}) {
  const hot = attention && value != null && value > 0;
  return (
    <span title={title} className={cn("text-11 tabular-nums", hot ? "text-accent" : "text-muted", className)}>
      {formatCount(value, quality)}
    </span>
  );
}
