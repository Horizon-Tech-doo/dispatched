import { CircleAlert } from "lucide-react";
import { useId, type ReactNode } from "react";

export interface FieldControlProps {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: true;
}

export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string;
  hint?: ReactNode;
  error?: ReactNode;
  children: (ids: FieldControlProps) => ReactNode;
}) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-12 font-semibold text-muted">
        {label}
      </label>
      {children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined })}
      {hint && <p id={hintId} className="m-0 text-11 text-muted">{hint}</p>}
      {error && (
        <p id={errorId} className="m-0 flex items-start gap-1.5 text-12 text-fg">
          <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
          <span>{error}</span>
        </p>
      )}
    </div>
  );
}
