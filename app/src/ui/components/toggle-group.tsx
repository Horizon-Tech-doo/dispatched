import { cn } from "../lib/cn";

/** Several independent on/off buttons in one frame (which counts to show, say). For one-of-many choices use `Segmented`. */
export function ToggleGroup<T extends string>({ label, values, options, onChange }: {
  label: string;
  values: T[];
  options: { value: T; label: string }[];
  onChange(values: T[]): void;
}) {
  return (
    <div role="group" aria-label={label} className="inline-flex h-6 items-stretch rounded-ctl border border-line bg-field p-px">
      {options.map((o) => {
        const on = values.includes(o.value);
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={on}
            onClick={() => onChange(on ? values.filter((v) => v !== o.value) : [...values, o.value])}
            className={cn(
              "rounded-[2px] px-2 text-12 focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg",
              on ? "bg-selection text-fg" : "text-muted hover:text-fg",
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
