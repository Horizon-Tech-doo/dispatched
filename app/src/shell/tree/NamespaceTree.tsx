import { CircleAlert, SlidersHorizontal } from "lucide-react";
import { useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { useEntities, useProfiles, useSubscriptionsFor } from "@/engine";
import {
  Button, ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
  CountBadge, EmptyState, EntityGlyph, Hint, IconButton, Input, LabelSwatch, Problem, Skeleton, SkeletonRows, Tree, cn, formatCount, formatNumber, rowHeight,
} from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";
import { deadLetterUrl, entityUrl } from "../entityUrls";
import { keepTogether } from "../text";
import { DASHBOARD_ID, MAX_FILTER_LENGTH, type CountKind } from "../state";
import { compileRules, highlightOf, type Highlight } from "./highlight";
import { describeListError } from "./listError";
import { buildNodes, type NsNode, type SubsState } from "./model";
import type { EntityCounts } from "@/engine";

const errorKind = (e: Error) => (e as { kind?: string }).kind ?? "";

const glyphOf = { namespace: "namespace", queue: "queue", topic: "topic", subscription: "subscription" } as const;

/** A pinned entity is known by name only: unless it is a subscription (the id says so), its kind is not known. */
const glyphKind = (n: NsNode) => (n.kind === "known" && n.entity?.kind !== "subscription" ? "unknown" : (n.entity?.kind ?? glyphOf[n.kind as keyof typeof glyphOf]));

/** Unlaid-out panes measure 0x0; the tree renders a screenful until it is measured. */
const FLOOR = { width: 280, height: 640 };

const SLOTS: CountKind[] = ["active", "scheduled", "transfer-dlq", "dead-letter"];
const COUNT_WORD: Record<CountKind, string> = { active: "active", scheduled: "scheduled", "transfer-dlq": "transfer DLQ", "dead-letter": "dead-letter" };
const COUNT_TITLE: Record<CountKind, string> = { active: "Active", scheduled: "Scheduled", "transfer-dlq": "Transfer dead-letter", "dead-letter": "Dead-letter" };
const countOf = (c: EntityCounts, k: CountKind) => (k === "active" ? c.active : k === "scheduled" ? c.scheduled : k === "transfer-dlq" ? c.transfer_dead_letter : c.dead_letter);
/** Active always shows (a zero is data); the other counts only above zero, like dead letters. */
const showsValue = (k: CountKind, c: EntityCounts | null | undefined) => (k === "active" ? true : !!c && countOf(c, k) > 0);
const capital = (w: string) => w[0].toUpperCase() + w.slice(1);

/** A row in the tree: the node and what the connection's rules say about it. */
type TreeRow = NsNode & { highlight: Highlight | null };

/** How wide, in characters, each count's slot is: as wide as its widest value among the loaded entities, so a count's
 *  place in the row tells which count it is. */
function slotWidths(all: (EntityCounts | null | undefined)[]): Record<CountKind, number> {
  const w: Record<CountKind, number> = { active: 1, scheduled: 1, "transfer-dlq": 1, "dead-letter": 1 };
  for (const c of all) {
    if (!c) continue;
    for (const k of SLOTS) if (showsValue(k, c) && (k === "active" || countOf(c, k) > 0)) w[k] = Math.max(w[k], formatCount(countOf(c, k), c).length);
  }
  return w;
}

/** The row said in words, for a screen reader: its name, the counts it shows and the rule that marks it. */
function rowLabel(n: NsNode, counts: CountKind[], highlight: Highlight | null): string | undefined {
  const parts: string[] = [];
  if (n.kind !== "topic" && n.counts) for (const k of SLOTS) if (counts.includes(k) && showsValue(k, n.counts)) parts.push(`${formatCount(countOf(n.counts, k), n.counts)} ${COUNT_WORD[k]}`);
  if (highlight) parts.push(`highlighted ${highlight.color}: ${highlight.description}`);
  return parts.length ? [n.label, ...parts].join(", ") : undefined;
}

interface RowMarks { counts: CountKind[]; widths: Record<CountKind, number>; swatchSlot: boolean }

/** A row's marks, right-aligned and never truncated: a fixed slot for each chosen count (active, scheduled, transfer
 *  DLQ, dead-letter; blank when there is nothing to show), then the highlight swatch in a slot of its own at the edge.
 *  Scheduled and transfer DLQ give way first when the pane is narrow. */
function Marks({ n, highlight, marks }: { n: TreeRow; highlight: Highlight | null; marks: RowMarks }) {
  const c = n.counts;
  const kinds = n.kind === "topic" ? [] : SLOTS.filter((k) => marks.counts.includes(k));
  if (!kinds.length && !marks.swatchSlot) return null;
  return (
    <span className="ml-auto flex shrink-0 items-baseline justify-end gap-1 pl-1">
      {kinds.map((k) => {
        const slot = (children: ReactNode) => (
          <span
            key={k}
            data-count-slot={k}
            // Fixed slots from 230px up, where position must tell the kind; below, only what each value needs, so the
            // counts that stay (active and dead-letter) fit beside a name that has to give way.
            style={{ "--w": `${marks.widths[k]}ch` } as CSSProperties}
            className={cn("inline-block text-right @[230px]:min-w-[var(--w)]", (k === "scheduled" || k === "transfer-dlq") && "hidden @[230px]:inline-block")}
          >
            {children}
          </span>
        );
        if (!showsValue(k, c)) return slot(null);
        const badge = <CountBadge value={c ? countOf(c, k) : undefined} quality={c ?? undefined} attention={k === "dead-letter"} className="text-12" />;
        return (
          <Hint key={k} label={COUNT_TITLE[k]}>
            {slot(badge)}
          </Hint>
        );
      })}
      {marks.swatchSlot &&
        (highlight ? (
          <Hint label={`${capital(highlight.color)}: ${highlight.description}`}>
            <span data-swatch-slot className="ml-0.5 flex size-2 shrink-0 items-center self-center">
              <LabelSwatch color={highlight.color} />
              <span className="sr-only">highlighted</span>
            </span>
          </Hint>
        ) : (
          <span data-swatch-slot className="ml-0.5 size-2 shrink-0 self-center" />
        ))}
    </span>
  );
}

function Row({ n, production, onRetry, marks }: { n: TreeRow; production: boolean; onRetry: (topic: string) => void; marks: RowMarks }) {
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
            <span className="min-w-[2ch] truncate @[230px]:min-w-[6ch]" title={n.label}>{n.label}</span>
          )}
          <Marks n={n} highlight={n.highlight} marks={marks} />
        </>
      );
    }
  }
}

export function NamespaceTree() {
  const { conn, shell, dispatch, prefs, openSheet, copyConnectionString, copyText } = useShell();
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
  const chosen = shell.explorer.counts;
  // The rows with what the rules say about them and how a screen reader names them.
  const rows = useMemo<TreeRow[]>(
    () =>
      nodes.map((n) => {
        const entityRow = n.kind === "queue" || n.kind === "topic" || n.kind === "subscription" || n.kind === "known";
        const highlight = entityRow ? highlightOf(n.label, n.counts, rules) : null;
        return { ...n, highlight, ariaLabel: entityRow ? rowLabel(n, chosen, highlight) : undefined };
      }),
    [nodes, rules, chosen],
  );
  // The loaded entities' counts, for the width of each count's slot (not only the rows in view or unfiltered).
  const widths = useMemo(
    () => slotWidths([...(listing?.queues.map((q) => q.counts) ?? []), ...Object.values(subs).flatMap((s) => s.data?.map((x) => x.counts) ?? [])]),
    // `subs` is covered by `subsKey`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [listing, subsKey],
  );
  const marks: RowMarks = { counts: chosen, widths, swatchSlot: rules.length > 0 };

  const retryTopic = (topic: string) => void subQueries[expandedTopics.indexOf(topic)]?.refetch();

  function activate(id: string) {
    const n = byId.get(id);
    if (!n) return;
    if (n.kind === "namespace") dispatch({ type: "open", tab: { id: DASHBOARD_ID, kind: "dashboard" } });
    else if (n.entity) dispatch({ type: "open", tab: n.entity });
    else if (n.kind === "error" && n.topic) retryTopic(n.topic);
    else if (n.expandable) dispatch({ type: "toggleExpanded", nodeId: n.id, expanded: !n.expanded });
  }

  const copy = (text: string, said: string) => void copyText(text, said);
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
                  nodes={rows}
                  selectedId={shell.selected}
                  onSelect={(id) => dispatch({ type: "select", nodeId: id })}
                  onActivate={activate}
                  onToggle={(id, expanded) => dispatch({ type: "toggleExpanded", nodeId: id, expanded })}
                  renderRow={(n) => <Row n={n} production={profile?.production ?? conn.view?.production ?? false} onRetry={retryTopic} marks={marks} />}
                  rowHeight={rowHeight[prefs.density]}
                  initialRect={FLOOR}
                />
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode && activate(menuNode.id)}>Open</ContextMenuItem>
              <ContextMenuItem onSelect={() => dispatch({ type: "open", tab: { id: DASHBOARD_ID, kind: "dashboard" } })}>Open dashboard</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode && copy(menuNode.label, `Copied the name ${menuNode.label}`)}>Copy name</ContextMenuItem>
              <ContextMenuItem disabled={!menuNode?.entity} onSelect={() => menuNode?.entity && copy(menuNode.entity.id, `Copied the entity path for ${menuNode.label}`)}>Copy entity path</ContextMenuItem>
              <ContextMenuItem disabled={!menuEntity || !fullHost} onSelect={() => menuEntity && menuNode && copy(entityUrl(fullHost, menuEntity.id, menuEntity.kind), `Copied the URL for ${menuNode.label}`)}>Copy URL</ContextMenuItem>
              <ContextMenuItem disabled={!menuDlqUrl} onSelect={() => menuDlqUrl && menuNode && copy(menuDlqUrl, `Copied the dead-letter URL for ${menuNode.label}`)}>Copy dead-letter URL</ContextMenuItem>
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
