import type { DiscoveredNamespace, DiscoveryView, EntityCounts, QueueInfo, SkippedSubscription, StoredProfile, SubscriptionInfo, TenantView, TopicInfo } from "../types";

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
  q("returns-intake", counts(0, 211), { requires_session: true }),
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
