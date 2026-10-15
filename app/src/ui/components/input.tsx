import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export function Input({ className, mono, ...props }: ComponentProps<"input"> & { mono?: boolean }) {
  return (
    <input
      spellCheck={false}
      autoComplete="off"
      className={cn(
        "h-6 w-full min-w-0 rounded-ctl border border-line bg-field px-2 text-12 text-fg placeholder:text-muted",
        "focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg focus-visible:-outline-offset-1 aria-invalid:border-accent disabled:opacity-45",
        mono && "font-mono text-11",
        className,
      )}
      {...props}
    />
  );
}

export function Textarea({ className, mono, ...props }: ComponentProps<"textarea"> & { mono?: boolean }) {
  return (
    <textarea
      spellCheck={false}
      className={cn(
        "min-h-16 w-full resize-y rounded-ctl border border-line bg-field px-2 py-1 text-12 text-fg placeholder:text-muted",
        "focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg focus-visible:-outline-offset-1 aria-invalid:border-accent disabled:opacity-45",
        mono && "font-mono text-11",
        className,
      )}
      {...props}
    />
  );
}
