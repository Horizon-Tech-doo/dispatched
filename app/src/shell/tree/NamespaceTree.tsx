import { CircleAlert } from "lucide-react";
import { useMemo, useState } from "react";
import { useEntities, useProfiles, useSubscriptionsFor } from "@/engine";
import {
  Button, ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
  CountBadge, EmptyState, EntityGlyph, Input, Skeleton, SkeletonRows, Tree, formatNumber, rowHeight,
} from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";
import { describeListError } from "./listError";
import { buildNodes, type NsNode, type SubsState } from "./model";

const errorKind = (e: Error) => (e as { kind?: string }).kind ?? "";

const glyphOf = { namespace: "namespace", queue: "queue", topic: "topic", subscription: "subscription" } as const;

/** Unlaid-out panes measure 0x0; the tree renders a screenful until it is measured. */
const FLOOR = { width: 280, height: 640 };

function Counts({ n }: { n: NsNode }) {
  const c = n.counts;
  return (
    <span className="ml-auto flex shrink-0 items-baseline gap-1.5">
      <CountBadge value={c?.active} quality={c ?? undefined} className="text-12" />
      {c && c.dead_letter > 0 && <CountBadge value={c.dead_letter} quality={c} attention className="text-12" />}
    </span>
  );
}

function Row({ n, production, onRetry }: { n: NsNode; production: boolean; onRetry: (topic: string) => void }) {
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
          <Button size="sm" variant="quiet" className="ml-auto h-[18px] shrink-0" onClick={(e) => { e.stopPropagation(); onRetry(n.topic!); }}>
            Retry
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
    default:
      return (
        <>
          <EntityGlyph kind={n.entity?.kind ?? glyphOf[n.kind as keyof typeof glyphOf]} className="shrink-0 text-icon" />
          {n.kind === "known" && n.topic ? (
            // Name first, topic after it as secondary text: only the topic gives way when the row is narrow.
            <>
              <span className="shrink-0" title={`${n.topic}/${n.label}`}>{n.label}</span>
              <span className="min-w-0 truncate text-muted">{n.topic}</span>
            </>
          ) : (
            <span className="truncate" title={n.label}>{n.label}</span>
          )}
          {n.kind !== "topic" && <Counts n={n} />}
        </>
      );
  }
}

export function NamespaceTree() {
  const { conn, shell, dispatch, prefs, openSheet } = useShell();
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
  const [filter, setFilter] = useState("");
  const [menuId, setMenuId] = useState<string | null>(null);

  const subs: Record<string, SubsState> = {};
  expandedTopics.forEach((t, i) => {
    const q = subQueries[i];
    subs[t] = { data: q?.data, error: q?.error ? { message: q.error.message } : null, pending: !q || q.isPending };
  });
  // useQueries returns a fresh array each render; key the memo on what the rows actually show.
  const subsKey = JSON.stringify(Object.entries(subs).map(([t, s]) => [t, s.pending, s.data, s.error?.message]));

  const host = shortHost(profile?.host ?? conn.view?.host ?? "");
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
    if (n.entity) dispatch({ type: "open", tab: n.entity });
    else if (n.kind === "error" && n.topic) retryTopic(n.topic);
    else if (n.expandable) dispatch({ type: "toggleExpanded", nodeId: n.id, expanded: !n.expanded });
  }

  const copy = (text: string) => void navigator.clipboard?.writeText(text).catch(() => {});
  const menuNode = menuId ? byId.get(menuId) : undefined;

  const summary = listing && !listing.limited ? `${formatNumber(listing.queues.length)} queues, ${formatNumber(listing.topics.length)} topics` : "";
  const editConnection = profileId ? () => openSheet({ kind: "profile", profileId }) : undefined;
  const notice = (text: string, detail?: string, action?: { label: string; run: () => void }) => (
    <div className="flex shrink-0 items-start gap-1.5 border-y border-line px-3 py-2 text-12 text-fg">
      <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <p className="m-0">{text}</p>
        {detail && <p className="m-0 mt-0.5 break-words text-11 text-muted">{detail}</p>}
        {action && <Button size="sm" variant="quiet" className="-ml-1.5 mt-1" onClick={action.run}>{action.label}</Button>}
      </div>
    </div>
  );
  const retry = { label: "Retry", run: () => void entities.refetch() };
  const limitedText = `This identity can't list entities in ${host}, so only pinned entities are shown. Grant it Manage rights to see everything, or edit this connection to pin more.`;

  return (
    <nav aria-label="Namespace" className="flex h-full min-h-0 flex-col">
      <div className="flex h-[30px] shrink-0 items-center justify-between border-b border-line px-3">
        <span className="font-semibold text-muted">Explorer</span>
        <span className="text-11 tabular-nums text-faint">{summary}</span>
      </div>
      <div className="shrink-0 px-2 py-1.5">
        <Input
          id="tree-filter"
          type="search"
          aria-label="Filter entities"
          placeholder="Filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && setFilter("")}
          className="[&::-webkit-search-cancel-button]:appearance-none"
        />
      </div>
      {entities.error && listing &&
        notice(`${describeListError(errorKind(entities.error), host)} Showing the last loaded data.`, entities.error.message, retry)}
      {listing?.limited && notice(limitedText, undefined, editConnection && { label: "Edit connection", run: editConnection })}
      <div className="min-h-0 flex-1">
        {entities.isPending ? (
          <SkeletonRows rows={12} columns={2} />
        ) : entities.error && !listing ? (
          <EmptyState
            tone="error"
            title="Can't list this namespace"
            body={
              <>
                <span className="block text-fg">{describeListError(errorKind(entities.error), host)}</span>
                <span className="mt-1 block break-words text-11 text-muted">{entities.error.message}</span>
              </>
            }
            action={<Button onClick={() => void entities.refetch()}>Retry</Button>}
          />
        ) : listing?.limited && listing.known.length === 0 ? (
          <EmptyState
            title="No pinned entities"
            body={`This identity can't list entities in ${host} and none are pinned. Edit this connection to pin the queues or subscriptions you use, or grant it Manage rights.`}
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
                  renderRow={(n) => <Row n={n} production={profile?.production ?? conn.view?.production ?? false} onRetry={retryTopic} />}
                  rowHeight={rowHeight[prefs.density]}
                  initialRect={FLOOR}
                />
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode && activate(menuNode.id)}>Open</ContextMenuItem>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode && copy(menuNode.label)}>Copy name</ContextMenuItem>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode?.entity && copy(menuNode.entity.id)}>Copy entity path</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem onSelect={() => void entities.refetch()}>Refresh</ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        )}
      </div>
    </nav>
  );
}
