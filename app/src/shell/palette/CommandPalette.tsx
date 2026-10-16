import { useState } from "react";
import { useEntities, useProfiles, useSubscriptionsFor, type SubscriptionInfo } from "@/engine";
import {
  CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog, CommandShortcut, EntityGlyph, shortcutLabel,
} from "@/ui";
import { useShell } from "../context";
import { treeRowOf } from "../tree/model";
import { buildCommands, paletteHint, type PaletteCommand } from "./commands";

/** Up to this many topics the palette loads every topic's subscriptions so they can be searched. */
const MAX_PREFETCH_TOPICS = 50;
const GROUPS = ["Go to", "This entity", "Connections", "App"] as const;

export function CommandPalette() {
  const { paletteOpen, setPaletteOpen, conn, shell, dispatch, openSheet, refresh, setPrefs, copyConnectionString, copyText } = useShell();
  const connected = conn.status === "connected";
  const profiles = useProfiles().data ?? [];
  const active = profiles.find((p) => p.id === conn.profileId);
  const listing = useEntities(connected ? conn.profileId : null).data;
  const topics = listing?.topics ?? [];
  const searchAll = topics.length <= MAX_PREFETCH_TOPICS;
  // Same query keys as the tree, so topics the tree already expanded are not fetched twice; results arrive as they load.
  const topicNames = paletteOpen && connected && searchAll ? topics.map((t) => t.name) : [];
  const subQueries = useSubscriptionsFor(conn.profileId, topicNames);

  if (!paletteOpen) return null;

  const subs: Record<string, SubscriptionInfo[] | undefined> = {};
  topicNames.forEach((t, i) => (subs[t] = subQueries[i]?.data));

  const cmds = buildCommands({
    listing,
    subs,
    profiles,
    activeProfileId: connected ? conn.profileId : null,
    activeTab: (() => {
      const t = shell.tabs.find((x) => x.id === shell.activeTab);
      return t && t.kind !== "dashboard" ? { id: t.id, kind: t.kind } : null;
    })(),
    host: active?.host ?? conn.view?.host ?? "",
    // Until the profile list is known an Entra profile can't be told from the others: offer no connection string.
    entra: active === undefined || active.auth.kind === "entra",
    copy: (text, message) => void copyText(text, message),
    copyConnectionString: (id) => void copyConnectionString(id),
    open: (tab) => {
      dispatch({ type: "open", tab });
      // Show the entity in the tree too: open the rows above it and select its row. The dashboard has no row.
      if (tab.kind === "dashboard") return;
      const { rowId, ancestors } = treeRowOf(tab, listing?.limited != null);
      for (const id of ancestors) if (!shell.expanded.includes(id)) dispatch({ type: "toggleExpanded", nodeId: id, expanded: true });
      dispatch({ type: "select", nodeId: rowId });
    },
    connect: (id) => void conn.connect(id),
    openSheet,
    refresh,
    toggleDrawer: () => dispatch({ type: "layout", patch: { drawerOpen: !shell.layout.drawerOpen } }),
    setPrefs,
  });
  const run = (c: PaletteCommand) => {
    setPaletteOpen(false);
    c.run();
  };

  return (
    <CommandPaletteDialog open onOpenChange={setPaletteOpen}>
      <PaletteBody cmds={cmds} connected={connected} partial={connected && !searchAll} run={run} />
    </CommandPaletteDialog>
  );
}

/** Mounted only while the palette is open, so the query starts empty every time. */
function PaletteBody({ cmds, connected, partial, run }: { cmds: PaletteCommand[]; connected: boolean; partial: boolean; run: (c: PaletteCommand) => void }) {
  const [query, setQuery] = useState("");
  return (
    <>
      <CommandInput placeholder={paletteHint(connected)} value={query} onValueChange={setQuery} />
      {partial && (
        <p className="m-0 border-b border-line px-3 py-1.5 text-11 text-muted">
          Subscriptions of collapsed topics aren't searched. Expand a topic to include them.
        </p>
      )}
      <CommandList>
        <CommandEmpty>No results for "{query.trim()}"</CommandEmpty>
        {GROUPS.map((g) => {
          const items = cmds.filter((c) => c.group === g);
          if (!items.length) return null;
          return (
            <CommandGroup key={g} heading={g}>
              {items.map((c) => (
                <CommandItem key={c.id} value={[c.label, c.detail, ...(c.keywords ?? [])].filter(Boolean).join(" ")} onSelect={() => run(c)}>
                  {c.glyph ? <EntityGlyph kind={c.glyph} /> : <span aria-hidden="true" className="size-3.5 shrink-0" />}
                  <span className="truncate">{c.label}</span>
                  {c.detail && <span className="min-w-0 truncate text-muted">{c.detail}</span>}
                  {c.production && <span className="ml-auto shrink-0 pl-2 text-11 font-semibold tracking-[0.04em] text-accent">PROD</span>}
                  {c.shortcut && <CommandShortcut>{shortcutLabel(c.shortcut)}</CommandShortcut>}
                </CommandItem>
              ))}
            </CommandGroup>
          );
        })}
      </CommandList>
    </>
  );
}
