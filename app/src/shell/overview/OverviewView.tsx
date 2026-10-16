import { Check, Copy, KeyRound } from "lucide-react";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { commands, useEntityDetails, useProfiles, type EntityCounts } from "@/engine";
import { Button, CountBadge, EmptyState, Hint, IconButton, Problem, SkeletonRows, formatBytes, formatClock } from "@/ui";
import { useShell } from "../context";
import { deadLetterUrl, entityUrl } from "../entityUrls";
import type { EntityTab } from "../state";
import { formatProperty, labelOf } from "./labels";

const REFRESH_MS = 10_000;
const FLASH_MS = 1800;
/** Shown with the counts (the footer line), so the properties list leaves it out. */
const WITH_COUNTS = new Set(["SizeInBytes"]);

/** A short-lived message (copied, couldn't copy) that clears itself. */
function useFlash(): [string, (msg: string) => void] {
  const [msg, setMsg] = useState("");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const flash = useCallback((m: string) => {
    clearTimeout(timer.current);
    setMsg(m);
    timer.current = setTimeout(() => setMsg(""), FLASH_MS);
  }, []);
  return [msg, flash];
}

const H2 = "m-0 mb-1.5 text-12 font-semibold text-muted";

function CountRow({ label, value, c, attention }: { label: string; value: number | undefined; c: EntityCounts | null; attention?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-0.5">
      <dt className="text-12 text-muted">{label}</dt>
      <dd className="m-0">
        <CountBadge value={value} quality={c ?? undefined} attention={attention} className="text-13" />
      </dd>
    </div>
  );
}

/** One address: its name and copy button on a line, the address below it where a long one can wrap. */
function Address({ label, copyLabel, value }: { label: string; copyLabel: string; value: string }) {
  const [state, flash] = useFlash();
  const copy = () => {
    if (!navigator.clipboard) return flash("failed");
    navigator.clipboard.writeText(value).then(() => flash("copied"), () => flash("failed"));
  };
  return (
    <div className="py-1">
      <div className="flex items-center gap-1.5">
        <span className="text-12 text-muted">{label}</span>
        <Hint label={copyLabel}>
          <IconButton label={copyLabel} className="size-5" onClick={copy}>
            {state === "copied" ? <Check /> : <Copy />}
          </IconButton>
        </Hint>
        <span role="status" className="text-11 text-muted">
          {state === "copied" ? "Copied" : state === "failed" ? "Couldn't copy" : ""}
        </span>
      </div>
      {/* Wraps after a slash where it can, and anywhere only when one piece is wider than the column. */}
      <div className="font-mono text-11 leading-4 [overflow-wrap:anywhere]" data-selectable>
        {value.split(/(?<=\/)/).map((part, i) => (
          <Fragment key={i}>
            {i > 0 && <wbr />}
            {part}
          </Fragment>
        ))}
      </div>
    </div>
  );
}

function ConnectionStringCopy({ profileId, entity, entra }: { profileId: string | null; entity: string; entra: boolean }) {
  const [state, setState] = useState<{ kind: "idle" | "copied" } | { kind: "failed"; message: string }>({ kind: "idle" });
  useEffect(() => {
    if (state.kind !== "copied") return;
    const t = setTimeout(() => setState({ kind: "idle" }), FLASH_MS);
    return () => clearTimeout(t);
  }, [state]);
  const copy = async () => {
    if (!profileId) return;
    try {
      await commands.copyEntityConnectionString(profileId, entity);
      setState({ kind: "copied" });
    } catch (e) {
      setState({ kind: "failed", message: e instanceof Error ? e.message : String((e as { message?: string })?.message ?? e) });
    }
  };
  return (
    <div className="mt-2 flex flex-col gap-1">
      <Button className="self-start" disabled={entra || !profileId} onClick={() => void copy()}>
        {state.kind === "copied" ? <Check aria-hidden="true" className="size-3.5 text-icon" /> : <KeyRound aria-hidden="true" className="size-3.5 text-icon" />}
        Copy connection string
      </Button>
      {state.kind === "failed" ? (
        <span role="alert" className="text-11 text-fg">{state.message}</span>
      ) : (
        <span role="status" className="text-11 text-muted">
          {entra
            ? "Entra ID connections have no connection string. Copy the URL instead."
            : state.kind === "copied"
              ? "Copied to the clipboard. It is scoped to this entity."
              : "Scoped to this entity. It goes to the clipboard and is never shown."}
        </span>
      )}
    </div>
  );
}

export function OverviewView({ tab }: { tab: EntityTab }) {
  const { conn, dispatch } = useShell();
  const kind = tab.kind;
  const q = useEntityDetails(conn.profileId, kind, tab.id, REFRESH_MS);
  const profile = useProfiles().data?.find((p) => p.id === conn.profileId);
  const host = profile?.host ?? conn.view?.host ?? "";
  const entra = profile?.auth.kind === "entra";

  if (q.isPending) return <SkeletonRows rows={10} columns={2} />;
  if (!q.data)
    return (q.error as { kind?: string } | null)?.kind === "not_found" ? (
      <EmptyState
        tone="error"
        title="This entity wasn't found"
        body="It may have been deleted or renamed. Refresh the tree to see what is there now."
        action={<Button onClick={() => dispatch({ type: "close", id: tab.id })}>Close tab</Button>}
      />
    ) : (
      <EmptyState
        tone="error"
        title="Couldn't read this entity"
        body="Try again. If it keeps failing, the detail below says why."
        detail={q.error?.message}
        action={<Button onClick={() => void q.refetch()}>Try again</Button>}
      />
    );

  const d = q.data;
  const c = d.counts;
  const hasCounts = kind !== "topic";
  const dlqUrl = deadLetterUrl(host, tab.id, kind);
  const properties = d.properties.filter((p) => !(hasCounts && WITH_COUNTS.has(p.name)) && !(d.status && p.name === "Status"));
  // A topic has no counts block to carry its size, so the size joins its properties.
  const topicSize = !hasCounts && d.size_in_bytes != null && !properties.some((p) => p.name === "SizeInBytes") ? formatBytes(d.size_in_bytes) : null;

  return (
    <div className="flex h-full min-h-0 flex-wrap content-start gap-x-10 gap-y-6 overflow-y-auto px-3.5 py-3" data-shot-scroller>
      <div className="flex w-[340px] max-w-full flex-col gap-6">
        {hasCounts && (
          <section aria-label="Counts">
            <h2 className={H2}>Counts</h2>
            <dl className="m-0">
              <CountRow label="Active" value={c?.active} c={c} />
              <CountRow label="Dead-letter" value={c?.dead_letter} c={c} attention />
              <CountRow label="Scheduled" value={c?.scheduled} c={c} />
              <CountRow label="Transfer" value={c?.transfer} c={c} />
              <CountRow label="Transfer DLQ" value={c?.transfer_dead_letter} c={c} attention />
            </dl>
            <p className="m-0 mt-2 text-11 text-muted">
              {d.size_in_bytes != null && `${formatBytes(d.size_in_bytes)} stored. `}
              {c === null && "The service didn't report counts. "}
              {c?.truncated ? "Counted by peeking, stopped at the limit. " : c?.approximate ? "Counted by peeking. " : ""}
              Updated {formatClock(new Date(q.dataUpdatedAt).toISOString())}, every 10 s.
            </p>
            {q.isError && (
              <Problem className="mt-2" title="Couldn't refresh the counts" body="Showing the last counts read." detail={q.error?.message} />
            )}
          </section>
        )}
        <section aria-label="Addresses">
          <h2 className={H2}>Addresses</h2>
          <Address label="URL" copyLabel="Copy URL" value={entityUrl(host, tab.id, kind)} />
          {dlqUrl && <Address label="Dead-letter URL" copyLabel="Copy dead-letter URL" value={dlqUrl} />}
          {hasCounts && <ConnectionStringCopy profileId={conn.profileId} entity={tab.id} entra={entra} />}
        </section>
      </div>
      <section aria-label="Properties" className="min-w-[320px] flex-1">
        <h2 className={H2}>Properties</h2>
        {!hasCounts && q.isError && <Problem className="mb-2" title="Couldn't refresh the details" body="Showing the last details read." detail={q.error?.message} />}
        <dl className="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-6 gap-y-1">
          {d.status && (
            <>
              <dt className="text-12 text-muted">Status</dt>
              <dd className="m-0 text-12">{d.status}</dd>
            </>
          )}
          {topicSize && (
            <>
              <dt className="text-12 text-muted">Size</dt>
              <dd className="m-0 text-12">{topicSize}</dd>
            </>
          )}
          {properties.map((p) => (
            <div key={p.name} className="contents">
              <dt className="text-12 text-muted" title={p.name}>{labelOf(p.name)}</dt>
              <dd className="m-0 break-words text-12" data-selectable>
                {p.value === "" ? <span className="text-muted">None</span> : formatProperty(p.name, p.value)}
              </dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
