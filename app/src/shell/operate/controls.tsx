import type { ReactNode } from "react";

/** A group of a sheet's form: a 12/600 muted heading over its controls. */
export function Group({ label, children, id }: { label: string; children: ReactNode; id?: string }) {
  return (
    <section aria-labelledby={id} aria-label={id ? undefined : label} className="flex flex-col gap-1.5">
      <h3 id={id} className="m-0 text-12 font-semibold text-muted">
        {label}
      </h3>
      {children}
    </section>
  );
}

/** An on/off choice with what it means beneath. */
export function Check({ label, hint, checked, onChange, disabled }: { label: string; hint?: ReactNode; checked: boolean; onChange(v: boolean): void; disabled?: boolean }) {
  return (
    <label className="flex items-start gap-2 text-12">
      <input type="checkbox" className="mt-0.5 size-3.5 accent-[var(--c-fg)]" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <span>{label}</span>
        {hint && <span className="block text-11 text-muted">{hint}</span>}
      </span>
    </label>
  );
}

/** The topic a subscription id belongs to, or the id itself: where messages for it are sent. */
export const sendTarget = (entity: string) => (entity.includes("/subscriptions/") ? entity.split("/subscriptions/")[0] : entity);
