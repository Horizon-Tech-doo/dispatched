import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { commands, type ReasonGroup, type SubQueue } from "@/engine";
import { Button, Hint, Problem, SkeletonRows, formatNumber } from "@/ui";
import { useAuthKind } from "../access";
import { adviceFor } from "../operate/advice";

/** The filter that shows one reason's messages. */
export const reasonFilter = (reason: string) => `DeadLetterReason = '${reason.replace(/'/g, "''")}'`;

/** What a group's tooltip says: its description and sequence range. */
const groupHint = (g: ReasonGroup) =>
  [g.description, g.first_sequence === g.last_sequence ? `Seq ${g.first_sequence}` : `Seq ${g.first_sequence}–${g.last_sequence}`, g.reason ? "Show only these messages." : "Messages without a reason can't be filtered by it."]
    .filter(Boolean)
    .join(" · ");

/** The dead-letter queue grouped by reason: one row per reason with its count. A reason filters the view to it, joined
 *  to a property filter already applied (`combined`), which the panel then says. */
export function DlqSummary({
  profileId, entity, subQueue, combined, onPick,
}: {
  profileId: string | null;
  entity: string;
  subQueue: SubQueue;
  combined?: boolean;
  onPick(reason: string): void;
}) {
  const auth = useAuthKind();
  const [joined, setJoined] = useState(false);
  const q = useQuery({
    queryKey: ["dlq-summary", profileId, entity, subQueue],
    queryFn: () => commands.dlqSummary(profileId!, entity, subQueue),
    enabled: profileId !== null,
    staleTime: 30_000,
  });
  return (
    <section aria-label="Dead-letter reasons" className="flex max-h-[40%] shrink-0 flex-col border-b border-line bg-side">
      <div className="flex h-8 shrink-0 items-center gap-2 px-3">
        <span className="text-12 font-semibold text-muted">Reasons</span>
        {q.data && (
          <span className="text-11 tabular-nums text-muted">
            {formatNumber(q.data.scanned)} messages examined{q.data.truncated ? " · the count stopped at its cap, so there may be more" : ""}
          </span>
        )}
        <Button size="sm" variant="quiet" className="ml-auto" disabled={q.isFetching} onClick={() => void q.refetch()}>
          {q.isFetching ? "Reading…" : "Read again"}
        </Button>
      </div>
      {joined && <p className="m-0 px-3 pb-1 text-11 text-muted">Added to the filter already applied. Clear the filter to see this reason alone.</p>}
      <div className="min-h-0 overflow-y-auto pb-1">
        {q.isPending ? (
          <SkeletonRows rows={3} />
        ) : q.isError ? (
          <div className="px-3 py-2">
            <Problem
              title="Couldn't group the dead letters"
              body={adviceFor((q.error as { kind?: string }).kind ?? "unknown", q.error.message, "Listen", auth)}
              detail={q.error.message}
              action={<Button onClick={() => void q.refetch()}>Try again</Button>}
            />
          </div>
        ) : q.data.groups.length === 0 ? (
          <p className="m-0 px-3 py-1 text-12 text-muted">No dead letters.</p>
        ) : (
          <ul className="m-0 list-none p-0">
            {q.data.groups.map((g: ReasonGroup) => (
              <li key={`${g.reason}|${g.description}`}>
                <Hint label={groupHint(g)}>
                  <button
                    type="button"
                    aria-disabled={!g.reason || undefined}
                    onClick={() => {
                      if (!g.reason) return;
                      setJoined(!!combined);
                      onPick(g.reason);
                    }}
                    className="flex h-row w-full items-center gap-3 px-3 text-left hover:bg-hover focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg aria-disabled:hover:bg-transparent"
                  >
                    <span data-line className="flex min-w-0 flex-1 items-baseline gap-2">
                      <span className="truncate text-12 text-fg">{g.reason ?? "No reason"}</span>
                      {g.description && <span className="min-w-0 truncate text-11 text-muted">{g.description}</span>}
                    </span>
                    <span className="text-12 tabular-nums text-fg">{formatNumber(g.count)}</span>
                  </button>
                </Hint>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
