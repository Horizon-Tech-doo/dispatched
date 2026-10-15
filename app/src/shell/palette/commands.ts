import type { EntityListing, Profile, SubscriptionInfo } from "@/engine";
import type { SheetState } from "../context";
import type { EntityKind, Prefs } from "../state";
import { entityIdOf } from "../tree/model";

export interface PaletteCommand {
  id: string;
  group: "Go to" | "Connections" | "App";
  label: string;
  keywords?: string[];
  shortcut?: string;
  glyph?: EntityKind;
  run(): void;
}

export interface PaletteContext {
  listing: EntityListing | undefined;
  subs: Record<string, SubscriptionInfo[] | undefined>;
  profiles: Profile[];
  activeProfileId: string | null;
  open(tab: { id: string; kind: EntityKind }): void;
  connect(profileId: string): void;
  openSheet(s: Exclude<SheetState, null>): void;
  refresh(): void;
  toggleDrawer(): void;
  setPrefs(p: Partial<Prefs>): void;
}

export function buildCommands(c: PaletteContext): PaletteCommand[] {
  const out: PaletteCommand[] = [];
  const seen = new Set<string>();
  const goTo = (id: string, kind: EntityKind, label: string, keywords: string[] = []) => {
    if (seen.has(id)) return; // a pinned entity the namespace also lists appears once
    seen.add(id);
    out.push({ id: `go:${kind}:${id}`, group: "Go to", label, glyph: kind, keywords, run: () => c.open({ id, kind }) });
  };
  if (c.listing) {
    for (const q of c.listing.queues) goTo(q.name, "queue", q.name, ["queue"]);
    for (const t of c.listing.topics) {
      goTo(t.name, "topic", t.name, ["topic"]);
      for (const s of c.subs[t.name] ?? []) goTo(entityIdOf("subscription", s.name, t.name), "subscription", `${t.name}/${s.name}`, ["subscription"]);
    }
    for (const k of c.listing.known) {
      const id = k.subscription ? entityIdOf("subscription", k.subscription, k.entity) : k.entity;
      goTo(id, k.subscription ? "subscription" : "queue", k.subscription ? `${k.entity}/${k.subscription}` : k.entity, ["pinned"]);
    }
  }
  for (const p of c.profiles) {
    if (p.id === c.activeProfileId) continue;
    out.push({ id: `conn:${p.id}`, group: "Connections", label: `Switch to ${p.name}`, keywords: [p.host], run: () => c.connect(p.id) });
  }
  out.push({ id: "conn:add", group: "Connections", label: "Add connection", run: () => c.openSheet({ kind: "profile", profileId: null }) });
  out.push({ id: "conn:manage", group: "Connections", label: "Manage connections", run: () => c.openSheet({ kind: "profiles" }) });
  const app: Omit<PaletteCommand, "group">[] = [
    { id: "app:refresh", label: "Refresh", shortcut: "f5", run: c.refresh },
    { id: "app:drawer", label: "Toggle drawer", shortcut: "mod+j", run: c.toggleDrawer },
    { id: "app:theme-system", label: "Theme: follow system", keywords: ["appearance"], run: () => c.setPrefs({ theme: "system" }) },
    { id: "app:theme-dark", label: "Theme: dark", keywords: ["appearance"], run: () => c.setPrefs({ theme: "dark" }) },
    { id: "app:theme-light", label: "Theme: light", keywords: ["appearance"], run: () => c.setPrefs({ theme: "light" }) },
    { id: "app:density-compact", label: "Density: compact", run: () => c.setPrefs({ density: "compact" }) },
    { id: "app:density-comfortable", label: "Density: comfortable", run: () => c.setPrefs({ density: "comfortable" }) },
    { id: "app:settings", label: "Settings", shortcut: "mod+,", run: () => c.openSheet({ kind: "settings" }) },
    { id: "app:update", label: "Check for updates", run: () => c.openSheet({ kind: "update" }) },
  ];
  for (const a of app) out.push({ ...a, group: "App" });
  return out;
}
