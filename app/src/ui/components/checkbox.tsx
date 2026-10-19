import { Check } from "lucide-react";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

/** A 14px check box in the control style: `field` fill, 1px `line` border, 3px radius; on it takes the `selection` fill
 *  and a check. A native input underneath, so it keeps its role, keyboard and form behaviour. */
export function Checkbox({ className, ...props }: Omit<ComponentProps<"input">, "type">) {
  return (
    <span className={cn("relative inline-flex size-3.5 shrink-0", className)}>
      <input
        type="checkbox"
        className={cn(
          "peer size-3.5 appearance-none rounded-ctl border border-line bg-field checked:border-fg checked:bg-selection disabled:opacity-45",
          "focus-visible:outline focus-visible:outline-1 focus-visible:outline-offset-1 focus-visible:outline-fg",
        )}
        {...props}
      />
      <Check aria-hidden="true" strokeWidth={3} className="pointer-events-none absolute inset-0 m-auto hidden size-2.5 text-fg peer-checked:block" />
    </span>
  );
}
