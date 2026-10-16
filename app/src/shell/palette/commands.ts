import type { EntityListing, Profile, SubscriptionInfo } from "@/engine";
import type { SheetState } from "../context";
import { DASHBOARD_ID, type EntityKind, type Prefs } from "../state";
import { entityIdOf, splitSubscriptionId } from "../entityId";
import { entityUrl } from "../entityUrls";

export interface PaletteCommand {
  id: string;
  group: "Go to" | "Connections" | "App" | "This entity";
  label: string;
  /** Secondary text after the label: the kind of an entity, the topic of a subscription. */
  detail?: string;
  /** A production connection: shown with the same PROD mark as the tree. */
  production?: boolean;
  keywords?: string[];
  shortcut?: string;
  glyph?: EntityKind;
  run(): void;
}

/** The one description of what the palette does, for the top bar field and the palette input. */
export const paletteHint = (connected: boolean) => (connected ? "Go to entity or command" : "Run a command");

export interface PaletteContext {
  listing: EntityListing | undefined;
  subs: Record<string, SubscriptionInfo[] | undefined>;
  profiles: Profile[];
  activeProfileId: string | null;
  /** The entity the active tab shows, if it shows one (not the dashboard). */
  activeTab: { id: string; kind: EntityKind } | null;
  /** The connected namespace's full host, for entity URLs. */
  host: string;
  /** The active connection signs in with Entra ID, so it has no connection string to copy. */
  entra: boolean;
  copy(text: string): void;
  copyConnectionString(entityId: string): void;
  open(tab: { id: string; kind: EntityKind } | { id: string; kind: "dashboard" }): void;
  connect(profileId: string): void;
  openSheet(s: Exclude<SheetState, null>): void;
  refresh(): void;
  toggleDrawer(): void;
  setPrefs(p: Partial<Prefs>): void;
}

export function buildCommands(c: PaletteContext): PaletteCommand[] {
  const out: PaletteCommand[] = [];
  const seen = new Set<string>();
  const goTo = (id: string, kind: EntityKind, label: string, detail: string, keywords: string[] = []) => {
    if (seen.has(id)) return; // a pinned entity the namespace also lists appears once
    seen.add(id);
    out.push({ id: `go:${kind}:${id}`, group: "Go to", label, detail, glyph: kind, keywords, run: () => c.open({ id, kind }) });
  };
  if (c.listing) {
    for (const q of c.listing.queues) goTo(q.name, "queue", q.name, "queue");
    for (const t of c.listing.topics) {
      goTo(t.name, "topic", t.name, "topic");
      for (const s of c.subs[t.name] ?? []) goTo(entityIdOf("subscription", s.name, t.name), "subscription", s.name, t.name, ["subscription"]);
    }
    for (const k of c.listing.known) {
      const id = k.subscription ? entityIdOf("subscription", k.subscription, k.entity) : k.entity;
      if (k.subscription) goTo(id, "subscription", k.subscription, k.entity, ["subscription", "pinned"]);
      else goTo(id, "queue", k.entity, "queue", ["pinned"]);
    }
  }
  for (const p of c.profiles) {
    if (p.id === c.activeProfileId) continue;
    out.push({ id: `conn:${p.id}`, group: "Connections", label: `Switch to ${p.name}`, production: p.production, keywords: [p.host], run: () => c.connect(p.id) });
  }
  out.push({ id: "conn:add", group: "Connections", label: "Add connection", run: () => c.openSheet({ kind: "profile", profileId: null }) });
  out.push({ id: "conn:manage", group: "Connections", label: "Manage connections", run: () => c.openSheet({ kind: "profiles" }) });
  const app: Omit<PaletteCommand, "group">[] = [
    ...(c.activeProfileId !== null ? [{ id: "app:dashboard", label: "Open dashboard", shortcut: "mod+shift+d", keywords: ["counts", "overview", "dead letters"], run: () => c.open({ id: DASHBOARD_ID, kind: "dashboard" as const }) }] : []),
    ...(c.activeProfileId !== null ? [{ id: "app:explorer-options", label: "Explorer options", keywords: ["tree", "counts", "highlight", "rules", "open on"], run: () => c.openSheet({ kind: "explorer-options" }) }] : []),
    { id: "app:refresh", label: "Refresh", shortcut: "f5", run: c.refresh },
    { id: "app:drawer", label: "Toggle Jobs and Activity", shortcut: "mod+j", run: c.toggleDrawer },
    { id: "app:theme-system", label: "Theme: follow system", keywords: ["appearance"], run: () => c.setPrefs({ theme: "system" }) },
    { id: "app:theme-dark", label: "Theme: dark", keywords: ["appearance"], run: () => c.setPrefs({ theme: "dark" }) },
    { id: "app:theme-light", label: "Theme: light", keywords: ["appearance"], run: () => c.setPrefs({ theme: "light" }) },
    { id: "app:density-compact", label: "Density: compact", run: () => c.setPrefs({ density: "compact" }) },
    { id: "app:density-comfortable", label: "Density: comfortable", run: () => c.setPrefs({ density: "comfortable" }) },
    { id: "app:settings", label: "Open settings", shortcut: "mod+,", run: () => c.openSheet({ kind: "settings" }) },
    { id: "app:update", label: "Check for updates", run: () => c.openSheet({ kind: "update" }) },
  ];
  for (const a of app) out.push({ ...a, group: "App" });
  const tab = c.activeTab;
  if (tab && c.activeProfileId !== null) {
    const name = tab.kind === "subscription" ? splitSubscriptionId(tab.id).name : tab.id;
    if (c.host) out.push({ id: "entity:url", group: "This entity", label: `Copy URL of ${name}`, keywords: ["address", "sb://"], run: () => c.copy(entityUrl(c.host, tab.id, tab.kind)) });
    if (!c.entra) out.push({ id: "entity:string", group: "This entity", label: `Copy connection string of ${name}`, keywords: ["key", "secret"], run: () => c.copyConnectionString(tab.id) });
  }
  return out;
}
