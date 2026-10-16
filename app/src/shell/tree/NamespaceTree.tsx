import { CircleAlert, SlidersHorizontal } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { useEntities, useProfiles, useSubscriptionsFor } from "@/engine";
import {
  Button, ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
  CountBadge, EmptyState, EntityGlyph, IconButton, Input, LabelSwatch, Problem, Skeleton, SkeletonRows, Tree, formatNumber, rowHeight, type LabelColor,
} from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";
import { copyFailureBody } from "../copyFailure";
import { deadLetterUrl, entityUrl } from "../entityUrls";
import { keepTogether } from "../text";
import { DASHBOARD_ID, MAX_FILTER_LENGTH, type CountKind } from "../state";
import { compileRules, highlightOf, type Compiled } from "./highlight";
import { describeListError } from "./listError";
import { buildNodes, type NsNode, type SubsState } from "./model";

const errorKind = (e: Error) => (e as { kind?: string }).kind ?? "";

const glyphOf = { namespace: "namespace", queue: "queue", topic: "topic", subscription: "subscription" } as const;

/** A pinned entity is known by name only: unless it is a subscription (the id says so), its kind is not known. */
const glyphKind = (n: NsNode) => (n.kind === "known" && n.entity?.kind !== "subscription" ? "unknown" : (n.entity?.kind ?? glyphOf[n.kind as keyof typeof glyphOf]));

/** Unlaid-out panes measure 0x0; the tree renders a screenful until it is measured. */
const FLOOR = { width: 280, height: 640 };

const COUNT_TITLE: Record<CountKind, string> = { active: "Active", scheduled: "Scheduled", "transfer-dlq": "Transfer dead-letter", "dead-letter": "Dead-letter" };

/** A row's marks, right-aligned and never truncated: the highlight swatch, then the chosen counts in the order active,
 *  scheduled, transfer dead-letter, dead-letter. Active always shows; the others only above zero; dead letters are amber. */
function Counts({ n, counts, color }: { n: NsNode; counts: CountKind[]; color: LabelColor | null }) {
  const c = n.counts;
  const shown = (k: CountKind) => counts.includes(k);
  const value = (k: CountKind) => (!c ? 0 : k === "active" ? c.active : k === "scheduled" ? c.scheduled : k === "transfer-dlq" ? c.transfer_dead_letter : c.dead_letter);
  const optional = (["scheduled", "transfer-dlq"] as const).filter((k) => shown(k) && value(k) > 0);
  return (
    <span className="ml-auto flex min-w-[3ch] shrink-0 items-baseline justify-end gap-1.5 pl-1">
      {color && (
        <span className="flex items-center self-center">
          <LabelSwatch color={color} />
          <span className="sr-only">highlighted</span>
        </span>
      )}
      {shown("active") && <CountBadge value={c?.active} quality={c ?? undefined} title={COUNT_TITLE.active} className="text-12" />}
      {c && optional.map((k) => <CountBadge key={k} value={value(k)} quality={c} title={COUNT_TITLE[k]} className="text-12" />)}
      {c && shown("dead-letter") && c.dead_letter > 0 && <CountBadge value={c.dead_letter} quality={c} attention title={COUNT_TITLE["dead-letter"]} className="text-12" />}
    </span>
  );
}

function Row({ n, production, onRetry, counts, rules }: { n: NsNode; production: boolean; onRetry: (topic: string) => void; counts: CountKind[]; rules: Compiled[] }) {
  switch (n.kind) {
    case "loading":
      return <Skeleton className="w-32" />;
    case "note":
      return <span className="truncate text-faint">{n.label}</span>;
    case "error":
      return (
        <>
          <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />
          <span className="truncate" title={n.message}>{n.label}: {n.message}</span>
          <Button size="sm" className="ml-auto h-[18px] shrink-0" onClick={(e) => { e.stopPropagation(); onRetry(n.topic!); }}>
            Try again
          </Button>
        </>
      );
    case "group":
      return (
        <>
          <span className="truncate font-semibold text-muted">{n.label}</span>
          {n.count != null && <CountBadge value={n.count} className="ml-auto shrink-0 text-12" />}
        </>
      );
    case "namespace":
      return (
        <>
          <EntityGlyph kind="namespace" className="shrink-0 text-icon" />
          <span className="truncate font-semibold">{n.label}</span>
          {production && <span className="ml-auto shrink-0 pl-2 text-11 font-semibold tracking-[0.04em] text-accent">PROD</span>}
        </>
      );
    default: {
      const color = highlightOf(n.label, n.counts, rules);
      return (
        <>
          <EntityGlyph kind={glyphKind(n)} className="shrink-0 text-icon" />
          {n.kind === "known" && n.topic ? (
            // Name first, topic after it as secondary text: only the topic gives way when the row is narrow.
            <>
              <span className="shrink-0" title={`${n.topic}/${n.label}`}>{n.label}</span>
              <span className="min-w-0 truncate text-muted">{n.topic}</span>
            </>
          ) : (
            <span className="min-w-0 truncate" title={n.label}>{n.label}</span>
          )}
          {(n.kind !== "topic" || color) && <Counts n={n} counts={n.kind === "topic" ? [] : counts} color={color} />}
        </>
      );
    }
  }
}

export function NamespaceTree() {
  const { conn, shell, dispatch, prefs, openSheet, copyConnectionString, copyNotice, dismissCopyNotice } = useShell();
  const profileId = conn.profileId;
  const refetchMs = prefs.autoRefresh * 1000;
  const profile = useProfiles().data?.find((p) => p.id === profileId);
  const entities = useEntities(profileId, refetchMs);
  const listing = entities.data;

  // Only topics that still exist: a stale expanded id must not trigger a request.
  const expandedTopics = useMemo(
    () => (listing ? shell.expanded.filter((e) => e.startsWith("t:") && listing.topics.some((t) => t.name === e.slice(2))).map((e) => e.slice(2)) : []),
    [listing, shell.expanded],
  );
  const subQueries = useSubscriptionsFor(profileId, expandedTopics, refetchMs);
  const filter = shell.treeFilter;
  const setFilter = (value: string) => dispatch({ type: "treeFilter", value });
  const rules = useMemo(() => compileRules(shell.explorer.rules), [shell.explorer.rules]);
  const [menuId, setMenuId] = useState<string | null>(null);

  const subs: Record<string, SubsState> = {};
  expandedTopics.forEach((t, i) => {
    const q = subQueries[i];
    subs[t] = { data: q?.data, error: q?.error ? { message: q.error.message } : null, pending: !q || q.isPending };
  });
  // useQueries returns a fresh array each render; key the memo on what the rows actually show.
  const subsKey = JSON.stringify(Object.entries(subs).map(([t, s]) => [t, s.pending, s.data, s.error?.message]));

  const fullHost = profile?.host ?? conn.view?.host ?? "";
  const host = shortHost(fullHost);
  const entra = profile?.auth.kind === "entra";
  // Like the Overview's button: until the profile list is known an Entra profile can't be told from the others.
  const canCopyString = profile !== undefined && !entra;
  const nodes = useMemo(
    () => (listing ? buildNodes({ host, listing, subs, expanded: new Set(shell.expanded), filter }) : []),
    // `subs` is covered by `subsKey`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [listing, subsKey, shell.expanded, filter, host],
  );
  const byId = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);

  const retryTopic = (topic: string) => void subQueries[expandedTopics.indexOf(topic)]?.refetch();

  function activate(id: string) {
    const n = byId.get(id);
    if (!n) return;
    if (n.kind === "namespace") dispatch({ type: "open", tab: { id: DASHBOARD_ID, kind: "dashboard" } });
    else if (n.entity) dispatch({ type: "open", tab: n.entity });
    else if (n.kind === "error" && n.topic) retryTopic(n.topic);
    else if (n.expandable) dispatch({ type: "toggleExpanded", nodeId: n.id, expanded: !n.expanded });
  }

  const copy = (text: string) => void navigator.clipboard?.writeText(text).catch(() => {});
  const menuNode = menuId ? byId.get(menuId) : undefined;
  const menuEntity = menuNode?.entity;
  const menuDlqUrl = menuEntity && fullHost ? deadLetterUrl(fullHost, menuEntity.id, menuEntity.kind) : null;

  const summary = listing && !listing.limited ? `${formatNumber(listing.queues.length)} queues, ${formatNumber(listing.topics.length)} topics` : "";
  const editConnection = profileId ? () => openSheet({ kind: "profile", profileId }) : undefined;
  const failure = entities.error
    ? describeListError(errorKind(entities.error), { name: profile?.name ?? conn.view?.profile_name ?? host, host, entra: profile?.auth.kind === "entra" })
    : null;
  // A listing that needs the connection fixed offers to edit it; anything else runs the listing again.
  const recover =
    failure?.fix === "edit" && editConnection ? (
      <Button onClick={editConnection}>Edit connection</Button>
    ) : (
      <Button onClick={() => void entities.refetch()}>Try again</Button>
    );
  const banner = (children: ReactNode) => <div className="shrink-0 border-y border-line px-3 py-2.5">{children}</div>;
  return (
    <nav aria-label="Namespace" className="@container flex h-full min-h-0 flex-col">
      <div className="flex h-[30px] shrink-0 items-center gap-1.5 border-b border-line pl-3 pr-1.5">
        <span className="font-semibold text-muted">Explorer</span>
        {/* Left out when the pane is too narrow to hold it beside the title. */}
        <span className="ml-auto hidden whitespace-nowrap text-11 tabular-nums text-faint @[230px]:inline">{summary}</span>
        <IconButton label="Explorer options" className="ml-auto shrink-0 @[230px]:ml-0" onClick={() => openSheet({ kind: "explorer-options" })}>
          <SlidersHorizontal />
        </IconButton>
      </div>
      <div className="shrink-0 px-2 py-1.5">
        <Input
          id="tree-filter"
          type="search"
          aria-label="Filter entities"
          placeholder="Filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          maxLength={MAX_FILTER_LENGTH}
          onKeyDown={(e) => e.key === "Escape" && setFilter("")}
          className="[&::-webkit-search-cancel-button]:appearance-none"
        />
      </div>
      {copyNotice &&
        banner(
          copyNotice.failure ? (
            <Problem
              role="alert"
              title="Couldn't copy the connection string"
              body={copyFailureBody(copyNotice.failure)}
              detail={copyNotice.failure}
              action={<Button onClick={dismissCopyNotice}>Dismiss</Button>}
            />
          ) : (
            <p role="status" className="m-0 text-11 text-muted">
              {keepTogether(`Copied the connection string for ${copyNotice.entity}. It is scoped to that entity and was never shown.`, copyNotice.entity)}
            </p>
          ),
        )}
      {entities.error && failure && listing &&
        banner(
          <Problem
            title="Couldn't refresh"
            body={<>{keepTogether(failure.message, host)} Showing the last loaded data.</>}
            detail={entities.error.message}
            action={recover}
          />,
        )}
      {listing?.limited &&
        banner(
          <Problem
            title="Showing pinned entities only"
            body={keepTogether(`This identity can't list entities in ${host}. Grant it Manage rights to see everything, or edit this connection to pin more.`, host)}
            action={editConnection && <Button onClick={editConnection}>Edit connection</Button>}
          />,
        )}
      <div className="min-h-0 flex-1">
        {entities.isPending ? (
          <SkeletonRows rows={12} columns={2} />
        ) : entities.error && failure && !listing ? (
          <EmptyState tone="error" title="Can't list this namespace" body={keepTogether(failure.message, host)} detail={entities.error.message} action={recover} />
        ) : listing?.limited && listing.known.length === 0 ? (
          <EmptyState
            title="No pinned entities"
            body={keepTogether(`This identity can't list entities in ${host} and none are pinned. Edit this connection to pin the queues or subscriptions you use, or grant it Manage rights.`, host)}
            action={editConnection && <Button onClick={editConnection}>Edit connection</Button>}
          />
        ) : nodes.length === 1 && filter.trim() ? (
          <EmptyState title="No matches" body={`Nothing loaded matches "${filter.trim()}". Expand topics to search their subscriptions.`} />
        ) : (
          <ContextMenu>
            <ContextMenuTrigger asChild>
              <div
                className="h-full"
                onContextMenuCapture={(e) => {
                  const under = (e.target as HTMLElement).closest<HTMLElement>("[data-node-id]")?.dataset.nodeId ?? null;
                  // The Menu key / Shift+F10 reports a primary button over the tree itself: act on the selected row.
                  const id = under ?? (e.button === 0 ? shell.selected : null);
                  setMenuId(id);
                  if (under) dispatch({ type: "select", nodeId: under });
                }}
              >
                <Tree
                  label="Namespace"
                  nodes={nodes}
                  selectedId={shell.selected}
                  onSelect={(id) => dispatch({ type: "select", nodeId: id })}
                  onActivate={activate}
                  onToggle={(id, expanded) => dispatch({ type: "toggleExpanded", nodeId: id, expanded })}
                  renderRow={(n) => <Row n={n} production={profile?.production ?? conn.view?.production ?? false} onRetry={retryTopic} counts={shell.explorer.counts} rules={rules} />}
                  rowHeight={rowHeight[prefs.density]}
                  initialRect={FLOOR}
                />
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode && activate(menuNode.id)}>Open</ContextMenuItem>
              <ContextMenuItem onSelect={() => dispatch({ type: "open", tab: { id: DASHBOARD_ID, kind: "dashboard" } })}>Open dashboard</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode && copy(menuNode.label)}>Copy name</ContextMenuItem>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode?.entity && copy(menuNode.entity.id)}>Copy entity path</ContextMenuItem>
              <ContextMenuItem disabled={!menuEntity || !fullHost} onSelect={() => menuEntity && copy(entityUrl(fullHost, menuEntity.id, menuEntity.kind))}>Copy URL</ContextMenuItem>
              <ContextMenuItem disabled={!menuDlqUrl} onSelect={() => menuDlqUrl && copy(menuDlqUrl)}>Copy dead-letter URL</ContextMenuItem>
              <ContextMenuItem
                disabled={!menuEntity || !canCopyString}
                title={entra ? "Entra ID connections have no connection string. Copy the URL instead." : undefined}
                onSelect={() => menuEntity && void copyConnectionString(menuEntity.id)}
              >
                Copy connection string
              </ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem onSelect={() => void entities.refetch()}>Refresh</ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        )}
      </div>
    </nav>
  );
}
