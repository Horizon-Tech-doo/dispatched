import { useQueryClient } from "@tanstack/react-query";
import { keys, useEntities, useProfiles, type SubscriptionInfo } from "@/engine";
import {
  CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog, CommandShortcut, EntityGlyph, shortcutLabel,
} from "@/ui";
import { useShell } from "../context";
import { treeRowOf } from "../tree/model";
import { buildCommands, type PaletteCommand } from "./commands";

export function CommandPalette() {
  const { paletteOpen, setPaletteOpen, conn, shell, dispatch, openSheet, refresh, setPrefs } = useShell();
  const qc = useQueryClient();
  const profiles = useProfiles().data ?? [];
  const listing = useEntities(conn.status === "connected" ? conn.profileId : null).data;

  if (!paletteOpen) return null;

  const subs: Record<string, SubscriptionInfo[] | undefined> = {};
  for (const t of listing?.topics ?? []) subs[t.name] = qc.getQueryData(keys.subscriptions(conn.profileId, t.name));

  const cmds = buildCommands({
    listing,
    subs,
    profiles,
    activeProfileId: conn.status === "connected" ? conn.profileId : null,
    open: (tab) => {
      // Show the entity in the tree too: open the rows above it and select its row.
      const { rowId, ancestors } = treeRowOf(tab, listing?.limited != null);
      for (const id of ancestors) if (!shell.expanded.includes(id)) dispatch({ type: "toggleExpanded", nodeId: id, expanded: true });
      dispatch({ type: "open", tab });
      dispatch({ type: "select", nodeId: rowId });
    },
    connect: (id) => void conn.connect(id),
    openSheet,
    refresh,
    toggleDrawer: () => dispatch({ type: "layout", patch: { drawerOpen: !shell.layout.drawerOpen } }),
    setPrefs,
  });
  const groups = ["Go to", "Connections", "App"] as const;
  const run = (c: PaletteCommand) => {
    setPaletteOpen(false);
    c.run();
  };

  return (
    <CommandPaletteDialog open onOpenChange={setPaletteOpen}>
      <CommandInput placeholder="Go to an entity or run a command" />
      <CommandList>
        <CommandEmpty>No matching entity or command.</CommandEmpty>
        {groups.map((g) => {
          const items = cmds.filter((c) => c.group === g);
          if (!items.length) return null;
          return (
            <CommandGroup key={g} heading={g}>
              {items.map((c) => (
                <CommandItem key={c.id} value={`${c.label} ${c.keywords?.join(" ") ?? ""}`} onSelect={() => run(c)}>
                  {c.glyph && <EntityGlyph kind={c.glyph} />}
                  <span className="truncate">{c.label}</span>
                  {c.shortcut && <CommandShortcut>{shortcutLabel(c.shortcut)}</CommandShortcut>}
                </CommandItem>
              ))}
            </CommandGroup>
          );
        })}
      </CommandList>
    </CommandPaletteDialog>
  );
}
