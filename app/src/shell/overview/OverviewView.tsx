import { PoliciesSection } from "../manage/PoliciesSection";
import { Check, Copy, KeyRound } from "lucide-react";
import { Fragment, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useEntityDetails, useProfiles, useSubscriptionsFor, type EntityCounts, type EntityDetails, type SubscriptionInfo } from "@/engine";
import { Button, CountBadge, EmptyState, Hint, IconButton, Problem, Skeleton, SkeletonRows, cn, formatBytes, formatClock, formatNumber } from "@/ui";
import { accessAdvice, isForbidden, rightsOf, useAuthKind } from "../access";
import { useShell } from "../context";
import { copyFailureBody } from "../copyFailure";
import { connectionStringScope, displayName, entityIdOf } from "../entityId";
import { deadLetterUrl, entityUrl } from "../entityUrls";
import type { EntityTab } from "../state";
import { formatProperty, groupProperties, isTimestamp, labelOf, propertiesJson } from "./labels";

const REFRESH_MS = 10_000;
const FLASH_MS = 1800;
const MAX_SUBSCRIPTIONS = 100;

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

const H2 = "m-0 mb-1.5 flex h-5 items-center text-12 font-semibold text-muted";

/** A count as data: zero is quiet, a number above zero is the primary colour, and only dead letters turn amber. */
const countTone = (value: number | undefined, attention?: boolean) => (value != null && value > 0 && !attention ? "text-fg" : undefined);

function CountRow({ label, value, c, attention }: { label: string; value: number | undefined; c: EntityCounts | null; attention?: boolean }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-0.5">
      <dt className="text-12 text-muted">{label}</dt>
      <dd className="m-0">
        <CountBadge value={value} quality={c ?? undefined} attention={attention} className={cn("text-13", countTone(value, attention))} />
      </dd>
    </div>
  );
}

/** One Size row: what the entity holds, then its limit when the service reports one. */
function SizeRow({ bytes, maxMb }: { bytes: number; maxMb: number | null }) {
  return (
    <div className="flex items-baseline justify-between gap-4 py-0.5">
      <dt className="text-12 text-muted">Size</dt>
      <dd className="m-0 text-13 tabular-nums text-fg">
        {formatBytes(bytes)}
        {maxMb != null && <span className="text-muted"> of {formatBytes(maxMb * 1024 * 1024)}</span>}
      </dd>
    </div>
  );
}

/** One address: its name and copy button on a line, the address below it where a long one can wrap. */
function Address({ label, copyLabel, value, said }: { label: string; copyLabel: string; value: string | null; /** What the status bar says after a copy. */ said: string }) {
  const [state, flash] = useFlash();
  const { copyText } = useShell();
  const copy = () => {
    if (value === null) return;
    // The failure is shown here, beside the button, so the status bar stays quiet about it.
    void copyText(value, said, { inline: true }).then((ok) => flash(ok ? "copied" : "failed"));
  };
  return (
    <div className="py-1">
      <div className="flex items-center gap-1.5">
        <span className="text-12 text-muted">{label}</span>
        <Hint label={copyLabel}>
          <IconButton label={copyLabel} className="size-5" disabled={value === null} onClick={copy}>
            {state === "copied" ? <Check /> : <Copy />}
          </IconButton>
        </Hint>
        <span role="status" className="text-11 text-muted">
          {state === "copied" ? "Copied" : state === "failed" ? "Couldn't copy" : ""}
        </span>
      </div>
      {value === null ? (
        <Skeleton className="mt-1 h-3 w-56" />
      ) : (
        // Wraps after a slash where it can, and anywhere only when one piece is wider than the column.
        <div className="font-mono text-11 leading-4 [overflow-wrap:anywhere]" data-selectable>
          {value.split(/(?<=\/)/).map((part, i) => (
            <Fragment key={i}>
              {i > 0 && <wbr />}
              {part}
            </Fragment>
          ))}
        </div>
      )}
    </div>
  );
}

function ConnectionStringCopy({ entity, eligible, entra }: { entity: string; eligible: boolean; entra: boolean }) {
  const { copyConnectionString } = useShell();
  // `EntityPath` names the queue or topic, so a subscription's string is scoped to its topic.
  const scope = connectionStringScope(entity, "this entity");
  const [state, setState] = useState<{ kind: "idle" | "copied" } | { kind: "failed"; message: string }>({ kind: "idle" });
  useEffect(() => {
    if (state.kind !== "copied") return;
    const t = setTimeout(() => setState({ kind: "idle" }), FLASH_MS);
    return () => clearTimeout(t);
  }, [state]);
  const copy = async () => {
    setState({ kind: "idle" });
    // The status bar says it was copied; a failure is a Problem right here, where the button is.
    const r = await copyConnectionString(entity, { inline: true });
    setState(r.ok ? { kind: "copied" } : { kind: "failed", message: r.message });
  };
  return (
    <div className="mt-2 flex flex-col gap-1.5">
      <Button className="self-start" disabled={!eligible} onClick={() => void copy()}>
        {state.kind === "copied" ? <Check aria-hidden="true" className="size-3.5 text-icon" /> : <KeyRound aria-hidden="true" className="size-3.5 text-icon" />}
        Copy connection string
      </Button>
      {state.kind === "failed" ? (
        <Problem role="alert" title="Couldn't copy the connection string" body={copyFailureBody(state.message)} detail={state.message} />
      ) : (
        <span role="status" className="text-11 text-muted">
          {entra
            ? "Entra ID connections have no connection string. Copy the URL instead."
            : state.kind === "copied"
              ? `Copied to the clipboard. It is scoped to ${scope}.`
              : `Scoped to ${scope}. It goes to the clipboard and is never shown.`}
        </span>
      )}
    </div>
  );
}

/** A topic's subscriptions with their counts; a name opens that subscription. */
function SubscriptionsTable({ topic, profileId }: { topic: string; profileId: string | null }) {
  const { dispatch, openSheet } = useShell();
  const auth = useAuthKind();
  // Read again while the Overview is shown, like its details (the engine logs only failures of either).
  const [q] = useSubscriptionsFor(profileId, [topic], REFRESH_MS);
  const subs: SubscriptionInfo[] | undefined = q?.data;
  const body = ((): ReactNode => {
    if (!q || q.isPending) return <SkeletonRows rows={3} columns={3} />;
    if (!subs)
      return (
        <Problem
          title="Couldn't list the subscriptions"
          body={isForbidden(q.error) ? accessAdvice(rightsOf(q.error!.message, "Manage"), auth) : "Try refreshing. If it keeps failing, the detail below says why."}
          detail={q.error?.message}
          action={isForbidden(q.error) && profileId ? <Button onClick={() => openSheet({ kind: "profile", profileId })}>Edit connection</Button> : undefined}
        />
      );
    if (subs.length === 0) return <p className="m-0 text-12 text-muted">This topic has no subscriptions.</p>;
    const shown = subs.slice(0, MAX_SUBSCRIPTIONS);
    return (
      <>
        <table aria-label="Subscriptions" className="w-full border-collapse text-12">
          <thead>
            <tr className="border-b border-line text-12 text-muted">
              <th scope="col" className="py-1 pr-3 text-left font-normal">Name</th>
              <th scope="col" className="px-3 py-1 text-right font-normal">Active</th>
              <th scope="col" className="py-1 pl-3 text-right font-normal">Dead-letter</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((s) => (
              <tr key={s.name}>
                <td className="max-w-0 py-0.5 pr-3">
                  <button
                    type="button"
                    className="block max-w-full truncate rounded-ctl text-left hover:underline focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg"
                    title={s.name}
                    onClick={() => dispatch({ type: "open", tab: { id: entityIdOf("subscription", s.name, topic), kind: "subscription" } })}
                  >
                    {s.name}
                  </button>
                </td>
                <td className="px-3 py-0.5 text-right tabular-nums">
                  <CountBadge value={s.counts?.active} quality={s.counts ?? undefined} className={cn("text-12", countTone(s.counts?.active))} />
                </td>
                <td className="py-0.5 pl-3 text-right tabular-nums">
                  <CountBadge value={s.counts?.dead_letter} quality={s.counts ?? undefined} attention className={cn("text-12", countTone(s.counts?.dead_letter, true))} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {subs.length > shown.length && (
          <p className="m-0 mt-1 text-11 text-muted">Showing the first {formatNumber(shown.length)} of {formatNumber(subs.length)}. The tree lists them all.</p>
        )}
      </>
    );
  })();
  return (
    <section aria-label="Subscriptions" className="mb-6">
      <h2 className={H2}>
        Subscriptions{subs && <span className="ml-1.5 font-normal tabular-nums">{formatNumber(subs.length)}</span>}
      </h2>
      {body}
    </section>
  );
}

function PropertiesSection({ d, hasCounts, fresh, maxMb, onEdit, onStatus }: { d: EntityDetails; hasCounts: boolean; fresh: boolean; maxMb: number | null; onEdit?: () => void; onStatus?: () => void }) {
  const [state, flash] = useFlash();
  const copyJson = () => {
    if (!navigator.clipboard) return flash("failed");
    navigator.clipboard.writeText(propertiesJson(d.properties)).then(() => flash("copied"), () => flash("failed"));
  };
  const groups = groupProperties(d.properties, { size: d.size_in_bytes != null, maxSize: d.size_in_bytes != null && maxMb !== null, messageCount: hasCounts, subscriptionCount: d.kind === "topic" });
  return (
    <section aria-label="Properties" className="min-w-0 flex-1">
      <div className="mb-1.5 flex h-5 items-center gap-2">
        <h2 className="m-0 text-12 font-semibold text-muted">Properties</h2>
        <span role="status" className="text-11 text-muted">
          {state === "copied" ? "Copied" : state === "failed" ? "Couldn't copy" : ""}
        </span>
        <Button variant="quiet" size="sm" className="ml-auto h-5 text-muted" onClick={copyJson} disabled={d.properties.length === 0}>
          <Copy aria-hidden="true" className="text-icon" />
          Copy JSON
        </Button>
        {onStatus && (
          <Button variant="quiet" size="sm" className="h-5 text-muted" onClick={onStatus}>
            Change status…
          </Button>
        )}
        {onEdit && (
          <Button variant="quiet" size="sm" className="h-5 text-muted" onClick={onEdit}>
            Edit settings…
          </Button>
        )}
      </div>
      {!fresh && <Problem className="mb-2" title="Couldn't refresh the details" body="Showing the last details read." />}
      {d.status && (
        <dl className="m-0 grid grid-cols-[12.5rem_minmax(0,1fr)] gap-x-4 gap-y-1 pb-2">
          <dt className="text-12 text-muted">Status</dt>
          <dd className="m-0 text-12">{d.status}</dd>
        </dl>
      )}
      {groups.map((g, i) => (
        <div key={g.title} className={cn("border-line pt-2", (i > 0 || d.status) && "mt-2 border-t")}>
          <h3 className="m-0 mb-1 text-12 font-semibold text-muted">{g.title}</h3>
          <dl className="m-0 grid grid-cols-[12.5rem_minmax(0,1fr)] gap-x-4 gap-y-1">
            {g.items.map((p) => (
              <div key={p.name} className="contents">
                <dt className="text-12 text-muted" title={p.name}>{labelOf(p.name)}</dt>
                <dd className={cn("m-0 break-words tabular-nums", isTimestamp(p.name) ? "font-mono text-11" : "text-12")} title={p.value} data-selectable>
                  {p.value === "" ? <span className="text-muted">None</span> : formatProperty(p.name, p.value)}
                </dd>
              </div>
            ))}
          </dl>
        </div>
      ))}
    </section>
  );
}

export function OverviewView({ tab }: { tab: EntityTab }) {
  const { conn, dispatch, openSheet } = useShell();
  const kind = tab.kind;
  const q = useEntityDetails(conn.profileId, kind, tab.id, REFRESH_MS);
  const profiles = useProfiles();
  const profile = profiles.data?.find((p) => p.id === conn.profileId);
  const host = profile?.host ?? conn.view?.host ?? "";
  const entra = profile?.auth.kind === "entra";
  const auth = useAuthKind();
  // The button waits for the profile list: until it is known, an Entra profile can't be told from the others.
  const eligible = profiles.data !== undefined && profile !== undefined && !entra;

  const denied = isForbidden(q.error);
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
        body={denied ? accessAdvice(rightsOf(q.error!.message, "Manage"), auth) : "Try again. If it keeps failing, the detail below says why."}
        detail={q.error?.message}
        action={
          denied && conn.profileId ? (
            <Button onClick={() => openSheet({ kind: "profile", profileId: conn.profileId! })}>Edit connection</Button>
          ) : (
            <Button onClick={() => void q.refetch()}>Try again</Button>
          )
        }
      />
    );

  const d = q.data;
  const c = d.counts;
  const isTopic = kind === "topic";
  const maxMbText = d.properties.find((p) => p.name === "MaxSizeInMegabytes")?.value;
  const maxMb = maxMbText && /^\d+$/.test(maxMbText) ? Number(maxMbText) : null;
  // A topic holds no messages of its own: its counts block is its size and, when the service reports it, its scheduled messages.
  const showCounts = !isTopic || d.size_in_bytes != null || c !== null;
  const url = host ? entityUrl(host, tab.id, kind) : null;
  const dlq = host ? deadLetterUrl(host, tab.id, kind) : null;

  return (
    <div className="flex h-full min-h-0 flex-wrap content-start gap-x-10 gap-y-6 overflow-y-auto px-3.5 py-3" data-shot-scroller>
      <div className="flex w-[340px] max-w-full flex-col gap-6">
        {showCounts && (
          <section aria-label="Counts">
            <h2 className={H2}>Counts</h2>
            <dl className="m-0">
              {d.size_in_bytes != null && <SizeRow bytes={d.size_in_bytes} maxMb={maxMb} />}
              {!isTopic && (
                <>
                  <CountRow label="Active" value={c?.active} c={c} />
                  <CountRow label="Dead-letter" value={c?.dead_letter} c={c} attention />
                </>
              )}
              {(!isTopic || c !== null) && <CountRow label="Scheduled" value={c?.scheduled} c={c} />}
              {!isTopic && (
                <>
                  <CountRow label="Transfer" value={c?.transfer} c={c} />
                  <CountRow label="Transfer DLQ" value={c?.transfer_dead_letter} c={c} attention />
                </>
              )}
            </dl>
            <p className="m-0 mt-2 text-11 text-muted">
              {c === null && !isTopic && "The service didn't report counts. "}
              {c?.truncated ? "Counted by peeking, stopped at the limit. " : c?.approximate ? "Counted by peeking. " : ""}
              Updated {formatClock(new Date(q.dataUpdatedAt).toISOString())}, every 10 s.
            </p>
            {q.isError && <Problem
                className="mt-2"
                title="Couldn't refresh the counts"
                body={denied ? `${accessAdvice(rightsOf(q.error!.message, "Manage"), auth)} Showing the last counts read.` : "Showing the last counts read."}
                detail={q.error?.message}
                action={denied && conn.profileId ? <Button onClick={() => openSheet({ kind: "profile", profileId: conn.profileId! })}>Edit connection</Button> : undefined}
              />}
          </section>
        )}
        <section aria-label="Addresses">
          <h2 className={H2}>Addresses</h2>
          <Address label="URL" copyLabel="Copy URL" value={url} said={`Copied the URL for ${displayName(tab.id)}`} />
          {kind !== "topic" && <Address label="Dead-letter URL" copyLabel="Copy dead-letter URL" value={dlq} said={`Copied the dead-letter URL for ${displayName(tab.id)}`} />}
          <ConnectionStringCopy entity={tab.id} eligible={eligible} entra={entra} />
        </section>
        {kind !== "subscription" && <PoliciesSection kind={kind} entity={tab.id} />}
      </div>
      <div className="flex min-w-[320px] max-w-[680px] flex-1 flex-col">
        {isTopic && <SubscriptionsTable topic={tab.id} profileId={conn.profileId} />}
        <PropertiesSection
          d={d}
          hasCounts={c !== null}
          fresh={!q.isError || showCounts}
          maxMb={maxMb}
          onEdit={conn.profileId ? () => openSheet({ kind: "operation", op: "entity-edit", entityKind: kind, entity: tab.id }) : undefined}
          onStatus={conn.profileId ? () => openSheet({ kind: "operation", op: "entity-status", target: { kind, entity: tab.id } }) : undefined}
        />
      </div>
    </div>
  );
}
