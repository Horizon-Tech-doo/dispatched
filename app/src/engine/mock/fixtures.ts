import type { EntityCounts, Profile, QueueInfo, SubscriptionInfo, TopicInfo } from "../types";

export const counts = (active: number, dead_letter: number, scheduled = 0): EntityCounts => ({
  active, dead_letter, scheduled, transfer: 0, transfer_dead_letter: 0, approximate: false, truncated: false,
});

export const profiles: Profile[] = [
  { id: "p-logistics", name: "Logistics", host: "sb-logistics-prod.servicebus.windows.net", auth: { kind: "connection-string" }, emulator: false, production: true, color: null, transport: "amqp", proxy: null, pinned_entities: [] },
  { id: "p-logistics-dev", name: "Logistics dev", host: "sb-logistics-dev.servicebus.windows.net", auth: { kind: "entra", sign_in_id: "oid-1", tenant: null }, emulator: false, production: false, color: null, transport: "amqp", proxy: null, pinned_entities: [] },
  { id: "p-partner", name: "Partner gateway", host: "sb-partner-gateway.servicebus.windows.net", auth: { kind: "connection-string" }, emulator: false, production: false, color: null, transport: "websockets", proxy: null, pinned_entities: ["orders"] },
];

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
