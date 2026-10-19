import { Tooltip as TooltipPrimitive } from "radix-ui";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../lib/cn";
import { Kbd } from "./kbd";

export function TooltipProvider(props: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={450} skipDelayDuration={150} {...props} />;
}
export const Tooltip = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export function TooltipContent({ className, sideOffset = 4, ...props }: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        sideOffset={sideOffset}
        className={cn("z-50 flex items-center gap-2 rounded-ctl border border-line bg-side px-2 py-1 text-12 text-fg shadow-float", className)}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
}

/** A trigger with a tooltip naming it, and its shortcut if it has one. */
export function Hint({ label, shortcut, off, children }: { label: string; shortcut?: string; off?: boolean; children: ReactNode }) {
  // `off`: no tooltip for now, but the trigger stays where it is in the tree (its button is not remounted).
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      {!off && (
        <TooltipContent>
          <span>{label}</span>
          {shortcut && <Kbd shortcut={shortcut} />}
        </TooltipContent>
      )}
    </Tooltip>
  );
}
