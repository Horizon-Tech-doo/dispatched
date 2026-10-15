import { Button, CountBadge, EmptyState, SkeletonRows, Tabs, TabsContent, TabsList, TabsTrigger } from "@/ui";
import { useShell } from "../context";
import type { OpenTab, SubTabId } from "../state";
import { splitSubscription, subTabsFor, useEntityInfo, validSubTab } from "./subtabs";

const KIND_LINE = { queue: "Queue", topic: "Topic", subscription: "Subscription" } as const;

/** A sub-tab count's reserved width: room for "99,999" in tabular numerals (five digit-wide ch plus a
 *  narrower comma), so counts up to that size, and the placeholder before them, never move the row. */
const COUNT_SLOT = "min-w-[6ch] tabular-nums";

export function EntityView({ tab }: { tab: OpenTab }) {
  const { dispatch } = useShell();
  const info = useEntityInfo(tab);
  const sub = tab.kind === "subscription" ? splitSubscription(tab.id) : null;
  const name = sub ? sub.name : tab.id;

  if (info.status === "gone")
    return (
      <EmptyState
        tone="error"
        title="This entity wasn't found"
        body={`${name} may have been deleted or renamed. Refresh the tree to see what is there now.`}
        action={<Button onClick={() => dispatch({ type: "close", id: tab.id })}>Close tab</Button>}
      />
    );

  // The name and kind are known from the tab itself, so the header and sub-tab row show at once;
  // only the content below waits for the entity to load.
  const defs = subTabsFor(tab.kind);
  const active = validSubTab(tab.kind, tab.subTab);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1 px-3.5 pt-2.5">
        <h1 className="m-0 text-14 font-semibold">{name}</h1>
        <span className="text-muted">{sub ? `Subscription on ${sub.topic}` : KIND_LINE[tab.kind]}</span>
        {info.facts.length > 0 && <span className="text-faint">{info.facts.join(", ")}</span>}
        {info.status === "unknown" && <span className="text-muted">Counts are unavailable while the namespace can't be listed.</span>}
      </div>
      <Tabs value={active} onValueChange={(v) => dispatch({ type: "subTab", id: tab.id, subTab: v as SubTabId })} className="flex min-h-0 flex-1 flex-col">
        <TabsList variant="underline" aria-label="Entity views" className="mt-1.5 border-b border-line">
          {defs.map((d) => {
            const c = d.count && info.counts ? d.count(info.counts) : null;
            return (
              <TabsTrigger key={d.id} value={d.id}>
                <span>{d.label}</span>
                {c ? (
                  <CountBadge value={c.value} quality={info.counts ?? undefined} attention={c.attention} className={COUNT_SLOT} />
                ) : (
                  // Hold the count's place while it loads, so the row does not shift when it arrives.
                  d.count && info.status === "loading" && <span aria-hidden="true" data-count-placeholder className={COUNT_SLOT} />
                )}
              </TabsTrigger>
            );
          })}
        </TabsList>
        {defs.map((d) => (
          <TabsContent key={d.id} value={d.id} className="overflow-y-auto">
            {info.status === "loading" ? <SkeletonRows rows={6} /> : <EmptyState title={`${d.label} isn't available yet`} body={d.description} />}
          </TabsContent>
        ))}
      </Tabs>
    </div>
  );
}
