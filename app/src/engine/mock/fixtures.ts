import type {
  DiscoveredNamespace, DiscoveryView, EntityCounts, EntityKindName, EntityProperty, QueueInfo, RuleInfo, SkippedSubscription, StoredProfile, SubscriptionInfo,
  TenantView, TopicInfo,
} from "../types";
import { generate, type MockMessage, type Spec } from "./messages";

export const counts = (active: number, dead_letter: number, scheduled = 0): EntityCounts => ({
  active, dead_letter, scheduled, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false,
});

export const profiles: StoredProfile[] = [
  { id: "p-logistics", name: "Logistics", host: "sb-logistics-prod.servicebus.windows.net", auth: { kind: "connection-string" }, emulator: false, production: true, color: null, transport: "amqp", proxy: null, pinned_entities: [] },
  { id: "p-logistics-dev", name: "Logistics dev", host: "sb-logistics-dev.servicebus.windows.net", auth: { kind: "entra", sign_in_id: "oid-1", tenant: null }, emulator: false, production: false, color: null, transport: "amqp", proxy: null, pinned_entities: [] },
  { id: "p-partner", name: "Partner gateway", host: "sb-partner-gateway.servicebus.windows.net", auth: { kind: "connection-string" }, emulator: false, production: false, color: null, transport: "websockets", proxy: null, pinned_entities: ["orders"] },
];

/** An Entra profile in the guest tenant, saved before the user signed in to that tenant itself: it is bound
 *  to the home sign-in and asks for the tenant, which wants its own (multi-factor) sign-in. */
export const guestProfile: StoredProfile = {
  id: "p-northwind", name: "Northwind orders", host: "nw-orders.servicebus.windows.net",
  auth: { kind: "entra", sign_in_id: "oid-1", tenant: "tenant-guest" },
  emulator: false, production: false, color: null, transport: "amqp", proxy: null, pinned_entities: [],
};

/** The mock's two accounts: the default one and the one "Use another account" switches to. */
export const accounts = {
  "oid-1": { id: "oid-1", user_principal: "ops@contoso.example", display_name: "Ops", tenant_id: "tenant-1" },
  "oid-2": { id: "oid-2", user_principal: "dev@fabrikam.example", display_name: "Dev", tenant_id: "tenant-2" },
} as const;

const q = (name: string, c: EntityCounts | null, extra: Partial<QueueInfo> = {}): QueueInfo => ({
  name, status: "Active", counts: c, size_in_bytes: 81_920, requires_session: false, max_delivery_count: 10, lock_duration: "PT30S", forward_to: null, ...extra,
});

export const queues: QueueInfo[] = [
  q("customs-docs", counts(112, 0)),
  q("invoice-generate", counts(1284, 37)),
  q("label-print", counts(6, 0)),
  q("returns-intake", counts(48, 211), { requires_session: true }),
];

export const topics: TopicInfo[] = [
  { name: "archive-events", status: "Active", size_in_bytes: 0, subscription_count: 0 },
  { name: "carrier-updates", status: "Active", size_in_bytes: 20_480, subscription_count: 2 },
  { name: "shipment-events", status: "Active", size_in_bytes: 4_194_304, subscription_count: 3 },
  { name: "stock-levels", status: "Active", size_in_bytes: 4_096, subscription_count: 1 },
];

export const manyTopics: TopicInfo[] = Array.from({ length: 60 }, (_, i) => ({
  name: `topic-${String(i).padStart(2, "0")}`, status: "Active", size_in_bytes: 0, subscription_count: 0,
}));

const s = (topic: string, name: string, c: EntityCounts | null): SubscriptionInfo => ({
  topic, name, status: "Active", counts: c, requires_session: false, max_delivery_count: 10, forward_to: null,
});

export const subscriptions: Record<string, SubscriptionInfo[]> = {
  "archive-events": [],
  "carrier-updates": [s("carrier-updates", "audit-log", counts(0, 0)), s("carrier-updates", "eta-recalc", counts(58, 9))],
  "shipment-events": [s("shipment-events", "billing", counts(0, 0)), s("shipment-events", "tracking-sync", counts(2906, 3417)), s("shipment-events", "warehouse-notify", counts(14, 1))],
  "stock-levels": [s("stock-levels", "replenish", null)],
};

export const manyQueues: QueueInfo[] = Array.from({ length: 2000 }, (_, i) =>
  q(`queue-${String(i).padStart(4, "0")}`, counts((i * 37) % 900, i % 17 === 0 ? (i % 50) + 1 : 0)),
);

/** The tenant a guest account (not the signed-in one's home) belongs to; it asks for multi-factor sign-in. */
export const GUEST_TENANT = "tenant-guest";

/** The signed-in account's tenants, home first. The mock accounts are `tenant-1` and `tenant-2`. */
export const tenantsFor = (homeTenantId: string): TenantView[] => [
  homeTenantId === "tenant-2"
    ? { tenant_id: homeTenantId, display_name: "Fabrikam Dev", default_domain: "fabrikamdev.onmicrosoft.com", home: true, sign_in_upn: null }
    : { tenant_id: homeTenantId, display_name: "Horizon Tech", default_domain: "horizontech.onmicrosoft.com", home: true, sign_in_upn: null },
  { tenant_id: GUEST_TENANT, display_name: "Northwind Traders", default_domain: "northwindtraders.onmicrosoft.com", home: false, sign_in_upn: null },
];

const ns = (
  name: string, tenantId: string, subscription: string, location: string, tier: string, extra: Partial<DiscoveredNamespace> = {},
): DiscoveredNamespace => ({
  name,
  host: `${name}.servicebus.windows.net`,
  resource_id: `/subscriptions/${subscription.toLowerCase().replace(/\W+/g, "-")}/resourceGroups/rg-${name}/providers/Microsoft.ServiceBus/namespaces/${name}`,
  subscription_id: subscription.toLowerCase().replace(/\W+/g, "-"),
  subscription_name: subscription,
  tenant_id: tenantId,
  location,
  tier,
  local_auth_disabled: false,
  ...extra,
});

const skippedLegacy: SkippedSubscription = {
  subscription_id: "legacy-sandbox",
  subscription_name: "Legacy sandbox",
  reason: "access denied: you need Reader on this subscription to list its namespaces",
};

export const discoveryFor = (homeTenantId: string): DiscoveryView => ({
  namespaces: [
    ns("sb-finance-events", homeTenantId, "Finance shared", "northeurope", "Standard", { local_auth_disabled: true }),
    ns("sb-logistics-dev", homeTenantId, "Logistics dev", "westeurope", "Standard"),
    ns("sb-logistics-prod", homeTenantId, "Logistics prod", "westeurope", "Premium"),
    ns("sb-partner-gateway", homeTenantId, "Partner integrations", "northeurope", "Standard"),
  ],
  skipped: [skippedLegacy],
});

/** Enough namespaces to need the filter. */
export const manyDiscoveryFor = (homeTenantId: string): DiscoveryView => ({
  namespaces: [
    ...discoveryFor(homeTenantId).namespaces,
    ...Array.from({ length: 8 }, (_, i) => ns(`sb-team-${String(i + 1).padStart(2, "0")}`, homeTenantId, `Team ${i + 1}`, i % 2 ? "westeurope" : "northeurope", "Standard")),
  ].sort((a, b) => a.name.localeCompare(b.name)),
  skipped: [],
});

/** What the guest tenant lists once the user has signed in to it. */
export const guestDiscovery: DiscoveryView = {
  namespaces: [
    ns("nw-orders", GUEST_TENANT, "Northwind production", "westeurope", "Premium"),
    ns("nw-telemetry", GUEST_TENANT, "Northwind shared", "uksouth", "Standard"),
  ],
  skipped: [],
};

/** The sessions of `returns-intake`'s 48 active messages. */
export const RMA = Array.from({ length: 12 }, (_, i) => `RMA-${1001 + i}`);

/** Messages per entity. Counts match the listing (invoice-generate: 1,288 generated = 1,284 active + 3 scheduled
 *  + 1 deferred; the listing says 1,284 active). `many-messages` gives customs-docs 12,000. */
const specs = (scenario: string): Record<string, Spec[]> => ({
  "customs-docs": [{ entity: "customs-docs", sub: "main", count: scenario === "many-messages" ? 12_000 : 112, firstSeq: 1 }],
  "invoice-generate": [
    { entity: "invoice-generate", sub: "main", count: 1288, firstSeq: 20_000, scheduledEvery: 400, deferredEvery: 600 },
    { entity: "invoice-generate", sub: "dlq", count: 37, firstSeq: 41_000 },
    { entity: "invoice-generate", sub: "tdlq", count: 2, firstSeq: 41_500 },
  ],
  "label-print": [{ entity: "label-print", sub: "main", count: 6, firstSeq: 300 }],
  "returns-intake": [
    { entity: "returns-intake", sub: "main", count: 48, firstSeq: 9_000, sessions: RMA },
    { entity: "returns-intake", sub: "dlq", count: 211, firstSeq: 8_000 },
  ],
  "shipment-events/subscriptions/tracking-sync": [
    { entity: "tracking-sync", sub: "main", count: 2906, firstSeq: 48_300 },
    { entity: "tracking-sync", sub: "dlq", count: 3417, firstSeq: 48_213 - 3416 },
  ],
  "carrier-updates/subscriptions/eta-recalc": [
    { entity: "eta-recalc", sub: "main", count: 58, firstSeq: 700 },
    { entity: "eta-recalc", sub: "dlq", count: 9, firstSeq: 650 },
  ],
  "shipment-events/subscriptions/warehouse-notify": [
    { entity: "warehouse-notify", sub: "main", count: 14, firstSeq: 1_000 },
    { entity: "warehouse-notify", sub: "dlq", count: 1, firstSeq: 990 },
  ],
});

const cache = new Map<string, MockMessage[]>();
/** Every message of an entity (all sub-queues, by sequence number), generated once per scenario. */
export function messagesOf(entity: string, scenario: string): MockMessage[] {
  const key = `${scenario === "many-messages" ? "many" : "default"}|${entity}`;
  let all = cache.get(key);
  if (!all) {
    all = (specs(scenario)[entity] ?? []).flatMap(generate);
    // The newest tracking-sync dead letters: an empty body, a 6 MiB log that arrives cut, a 5 MiB binary that arrives cut
    // (both cut to a few thousand rows: the viewer's work is the same).
    const at = (seq: number) => all!.find((m) => m.sub === "dlq" && m.sequence_number === seq);
    const empty = at(48_211);
    if (empty) Object.assign(empty, { body: { kind: "empty" }, body_size: 0, content_type: null });
    const log = at(48_212);
    if (log) {
      const text = Array.from({ length: 600 }, (_, i) => `2026-10-15T10:58:${String(i % 60).padStart(2, "0")}Z worker-${i % 7} INFO shipment ${48_212 + i} step ${i % 23} ok`).join("\n");
      Object.assign(log, { body: { kind: "text", value: text }, content_type: "text/plain", body_size: 6 * 1024 * 1024, body_truncated: true });
    }
    const big = at(48_213);
    if (big) {
      // Bytes that are not UTF-8, so the viewer shows hex.
      const bytes = Array.from({ length: 4096 }, (_, i) => String.fromCharCode((i * 31 + 0xc3) & 0xff)).join("");
      Object.assign(big, { body: { kind: "binary", value: btoa(bytes) }, content_type: "application/octet-stream", body_size: 5 * 1024 * 1024, body_truncated: true });
    }
    cache.set(key, all);
  }
  return all;
}

/** The properties an entity's description lists, in the service's order. */
export const details = (kind: EntityKindName, c: EntityCounts | null, session: boolean): EntityProperty[] => [
  { name: "LockDuration", value: "PT30S" },
  { name: "MaxSizeInMegabytes", value: "1024" },
  { name: "RequiresDuplicateDetection", value: "false" },
  { name: "RequiresSession", value: String(session) },
  { name: "DefaultMessageTimeToLive", value: "P14D" },
  { name: "DeadLetteringOnMessageExpiration", value: "true" },
  { name: "MaxDeliveryCount", value: "10" },
  { name: "EnableBatchedOperations", value: "true" },
  ...(kind === "queue" ? [{ name: "EnablePartitioning", value: "false" }] : []),
  { name: "AutoDeleteOnIdle", value: "P10675199DT2H48M5.4775807S" },
  { name: "CreatedAt", value: "2026-09-01T08:00:00Z" },
  { name: "UpdatedAt", value: "2026-10-01T08:00:00Z" },
  ...(c ? [{ name: "MessageCount", value: String(c.active + c.dead_letter) }] : []),
];

const defaultRule: RuleInfo = { name: "$Default", filter: { kind: "sql", expression: "1=1" }, action: null, created_at: "2026-09-01T08:00:00Z" };

/** Subscription rules; any other subscription has only `$Default`. */
export const rules: Record<string, RuleInfo[]> = {
  "shipment-events/subscriptions/tracking-sync": [
    defaultRule,
    { name: "eu-only", filter: { kind: "sql", expression: "region = 'eu' AND sys.Label LIKE 'shipment%'" }, action: "SET priority = 'high'", created_at: "2026-09-12T10:30:00Z" },
    {
      name: "priority",
      filter: {
        kind: "correlation", correlation_id: "batch-7", message_id: null, to: null, reply_to: null, subject: "priority", session_id: null,
        reply_to_session_id: null, content_type: null, properties: [{ name: "region", value: "eu" }, { name: "tier", value: "2" }],
      },
      action: null,
      created_at: null,
    },
  ],
  // A filter the reader does not understand (the engine's `unknown`).
  "carrier-updates/subscriptions/eta-recalc": [defaultRule, { name: "legacy-route", filter: { kind: "unknown" }, action: null, created_at: "2026-09-20T14:05:00Z" }],
};
