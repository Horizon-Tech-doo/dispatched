import type { ConfirmRequest } from "@/engine";
import { Field, Input, ProdBadge } from "@/ui";

/** The core's confirmation, word for word. On production the entity's name must be typed: the confirm button (in the
 *  sheet's footer) is enabled only when `typed` equals `request.target`. */
export function ConfirmPanel({ request, typed, onTyped }: { request: ConfirmRequest; typed: string; onTyped: (text: string) => void }) {
  return (
    <section aria-label="Confirmation" className="flex flex-col gap-3">
      {request.step > 1 && <p className="m-0 text-12 font-semibold text-muted">Confirmation {request.step}</p>}
      {request.production && (
        <p className="m-0 flex items-center gap-1.5 text-12 text-muted">
          <ProdBadge />
          <span>This connection is marked as production.</span>
        </p>
      )}
      <p className="m-0 select-text break-words text-13 text-fg" data-selectable>
        {request.summary}
      </p>
      {!request.reversible && <p className="m-0 text-12 text-muted">This can&apos;t be undone.</p>}
      {request.production && (
        <Field label={`Type ${request.target} to confirm`} hint="The name must match exactly.">
          {(ids) => <Input {...ids} mono autoFocus value={typed} onChange={(e) => onTyped(e.target.value)} />}
        </Field>
      )}
    </section>
  );
}

/** The answer a confirmation gets from the panel. */
export const answerFor = (request: ConfirmRequest, typed: string) =>
  request.production ? ({ type: "typed", text: typed } as const) : ({ type: "allow" } as const);

/** The confirm button may be pressed. */
export const canConfirm = (request: ConfirmRequest, typed: string) => !request.production || typed === request.target;
