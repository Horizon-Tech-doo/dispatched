import { CircleAlert } from "lucide-react";
import { useId, type ReactNode } from "react";

export interface FieldControlProps {
  id: string;
  "aria-describedby"?: string;
  "aria-invalid"?: true;
}

/** A problem under a control: what is wrong in plain words, and (optionally) the engine's own text on a mono line
 *  that can be selected and copied. */
export function FieldMessage({ id, error, detail }: { id?: string; error: ReactNode; detail?: ReactNode }) {
  return (
    <div id={id} className="flex flex-col gap-0.5">
      <p className="m-0 flex items-start gap-1.5 text-12 text-fg">
        <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
        <span>{error}</span>
      </p>
      {detail && <p className="m-0 select-text break-words pl-5 font-mono text-11 text-muted">{detail}</p>}
    </div>
  );
}

export function Field({
  label,
  hint,
  error,
  detail,
  inline,
  children,
}: {
  label: string;
  hint?: ReactNode;
  error?: ReactNode;
  /** The engine's text for `error`, on a mono line below it. */
  detail?: ReactNode;
  /** The label sits left of the control, in a column this many px wide, and the messages line up with the control. */
  inline?: number;
  children: (ids: FieldControlProps) => ReactNode;
}) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  const messages = (
    <>
      {hint && <p id={hintId} className="m-0 text-11 text-muted">{hint}</p>}
      {error && <FieldMessage id={errorId} error={error} detail={detail} />}
    </>
  );
  if (inline !== undefined)
    return (
      <div className="grid items-start gap-x-2 gap-y-1" style={{ gridTemplateColumns: `${inline}px minmax(0, 1fr)` }}>
        <label htmlFor={id} className="flex h-6 items-center text-12 font-semibold text-muted">
          {label}
        </label>
        {children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined })}
        {(hint || error) && <div className="col-start-2 flex flex-col gap-1">{messages}</div>}
      </div>
    );
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-12 font-semibold text-muted">
        {label}
      </label>
      {children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined })}
      {messages}
    </div>
  );
}
