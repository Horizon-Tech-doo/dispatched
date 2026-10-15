import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export function IconButton({ label, className, type = "button", ...props }: ComponentProps<"button"> & { label: string }) {
  return (
    <button
      type={type}
      aria-label={label}
      className={cn(
        "inline-grid size-6 shrink-0 place-items-center rounded-ctl text-icon hover:bg-hover hover:text-fg active:bg-selection disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5",
        className,
      )}
      {...props}
    />
  );
}
