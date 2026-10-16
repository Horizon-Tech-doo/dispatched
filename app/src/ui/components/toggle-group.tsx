import { Check } from "lucide-react";
import { cn } from "../lib/cn";

/** Several independent on/off buttons in one frame (which counts to show, say). An item that is on carries a check and a
 *  neutral fill, so it reads as switched on and not as one choice of many. For one-of-many choices use `Segmented`. */
export function ToggleGroup<T extends string>({ label, values, options, onChange }: {
  label: string;
  values: T[];
  options: { value: T; label: string }[];
  onChange(values: T[]): void;
}) {
  return (
    <div role="group" aria-label={label} className="inline-flex h-6 items-stretch overflow-hidden rounded-ctl border border-line bg-field">
      {options.map((o, i) => {
        const on = values.includes(o.value);
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={on}
            onClick={() => onChange(on ? values.filter((v) => v !== o.value) : [...values, o.value])}
            className={cn(
              "flex items-center gap-1 px-2 text-12 focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg",
              i > 0 && "border-l border-line",
              on ? "bg-selection text-fg hover:bg-selection" : "text-muted hover:bg-hover hover:text-fg",
            )}
          >
            {on && <Check aria-hidden="true" className="size-3 shrink-0" />}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
