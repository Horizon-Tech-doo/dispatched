import { useQuery } from "@tanstack/react-query";
import { commands, type ReasonGroup, type SubQueue } from "@/engine";
import { Button, Problem, SkeletonRows, formatNumber } from "@/ui";

/** The filter that shows one reason's messages. */
export const reasonFilter = (reason: string) => `DeadLetterReason = '${reason.replace(/'/g, "''")}'`;

/** The dead-letter queue grouped by reason, with each group's count and sequence range. A group with a reason filters
 *  the view to it. */
export function DlqSummary({ profileId, entity, subQueue, onPick }: { profileId: string | null; entity: string; subQueue: SubQueue; onPick(reason: string): void }) {
  const q = useQuery({
    queryKey: ["dlq-summary", profileId, entity, subQueue],
    queryFn: () => commands.dlqSummary(profileId!, entity, subQueue),
    enabled: profileId !== null,
    staleTime: 30_000,
  });
  return (
    <section aria-label="Dead-letter reasons" className="flex max-h-[40%] shrink-0 flex-col border-b border-line bg-side">
      <div className="flex h-7 shrink-0 items-center gap-2 px-3 text-11 text-muted">
        <span className="font-semibold">Reasons</span>
        {q.data && (
          <span className="tabular-nums">
            {formatNumber(q.data.scanned)} messages examined{q.data.truncated ? " · the count stopped at its cap, so there may be more" : ""}
          </span>
        )}
        <Button size="sm" variant="quiet" className="ml-auto h-[18px]" onClick={() => void q.refetch()}>
          Read again
        </Button>
      </div>
      <div className="min-h-0 overflow-y-auto pb-1">
        {q.isPending ? (
          <SkeletonRows rows={3} />
        ) : q.isError ? (
          <div className="px-3 py-2">
            <Problem title="Couldn't group the dead letters" body="Try again." detail={q.error.message} />
          </div>
        ) : q.data.groups.length === 0 ? (
          <p className="m-0 px-3 py-1 text-12 text-muted">No dead letters.</p>
        ) : (
          <ul className="m-0 list-none p-0">
            {q.data.groups.map((g: ReasonGroup) => (
              <li key={`${g.reason}|${g.description}`}>
                <button
                  type="button"
                  disabled={!g.reason}
                  title={g.reason ? `Show only ${g.reason}` : "Messages without a reason can't be filtered by it."}
                  onClick={() => g.reason && onPick(g.reason)}
                  className="grid w-full grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-3 px-3 py-0.5 text-left hover:bg-hover focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg disabled:hover:bg-transparent"
                >
                  <span className="min-w-0">
                    <span className="block truncate text-12 text-fg">{g.reason ?? "No reason"}</span>
                    {g.description && <span className="block truncate text-11 text-muted">{g.description}</span>}
                    <span className="block font-mono text-11 tabular-nums text-faint">
                      {g.first_sequence === g.last_sequence ? `seq ${g.first_sequence}` : `seq ${g.first_sequence}–${g.last_sequence}`}
                    </span>
                  </span>
                  <span className="text-12 tabular-nums text-fg">{formatNumber(g.count)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
