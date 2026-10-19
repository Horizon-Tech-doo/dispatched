import { ChevronRight } from "lucide-react";
import { useId, useState, type ReactNode } from "react";
import { Checkbox, cn } from "@/ui";

/** A group of a sheet's form: a 12/600 muted heading over its controls (a labelled group, not a landmark). */
export function Group({ label, children }: { label: string; children: ReactNode }) {
  const id = useId();
  return (
    <div role="group" aria-labelledby={id} className="flex flex-col gap-1.5">
      <h3 id={id} className="m-0 text-12 font-semibold text-muted">
        {label}
      </h3>
      {children}
    </div>
  );
}

/** A group whose fields are optional: closed, it says how many are set. It starts open when any is. */
export function Disclosure({ label, set, children }: { label: string; set: number; children: ReactNode }) {
  const [open, setOpen] = useState(set > 0);
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 self-start rounded-ctl text-12 font-semibold text-muted hover:text-fg focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg"
      >
        <ChevronRight aria-hidden="true" className={cn("size-3 text-icon", open && "rotate-90")} />
        {label}
        {set > 0 && <span className="font-normal tabular-nums"> ({set} set)</span>}
      </button>
      {open && (
        <div id={id} role="group" aria-label={label} className="flex flex-col gap-1.5">
          {children}
        </div>
      )}
    </div>
  );
}

/** An on/off choice with what it means beneath. */
export function Check({ label, hint, checked, onChange, disabled }: { label: string; hint?: ReactNode; checked: boolean; onChange(v: boolean): void; disabled?: boolean }) {
  return (
    <label className="flex items-start gap-2 text-12">
      <Checkbox className="mt-px" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <span>{label}</span>
        {hint && <span className="block text-11 text-muted">{hint}</span>}
      </span>
    </label>
  );
}

/** The topic a subscription id belongs to, or the id itself: where messages for it are sent. */
export const sendTarget = (entity: string) => (entity.includes("/subscriptions/") ? entity.split("/subscriptions/")[0] : entity);
