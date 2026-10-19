import type { MessageDetail, MessageSummary, PeekView, PropValue } from "../types";

/** A mock message: everything the engine returns, with the full body, and the sub-queue it lives in. */
export type MockMessage = MessageDetail & { sub: "main" | "dlq" | "tdlq" };

const REASONS: [string, string][] = [
  ["MaxDeliveryCountExceeded", "Message could not be consumed after 10 delivery attempts."],
  ["TTLExpiredException", "The message expired and was dead lettered."],
  ["SchemaValidationFailed", "Field 'invoiceId' is required."],
];
const SUBJECTS = ["shipment.updated", "shipment.created", "label.requested", "invoice.ready", "customs.cleared"];
const T0 = Date.parse("2026-10-15T09:00:00Z");

/** A small deterministic hash so every run sees the same messages (no `Math.random`). */
const pick = (i: number, n: number) => ((i * 2654435761) >>> 0) % n;

function body(i: number, kind: number): Pick<MessageDetail, "body" | "content_type" | "body_size"> {
  switch (kind) {
    case 0: {
      // Written by hand so the body carries an integer too large for a JS number (the viewer must keep it exact).
      const json = `{"shipmentId":"SHP-${100000 + i}","status":"${["in_transit", "delivered", "held"][pick(i, 3)]}","weightKg":${1 + pick(i, 40)},"carrier":{"code":"DHL","hub":"LEJ"},"trackingNumber":12345678901234567890}`;
      return { body: { kind: "binary", value: btoa(json) }, content_type: "application/json", body_size: json.length };
    }
    case 1: {
      const xml = `<?xml version="1.0"?><invoice id="INV-${i}"><line sku="A-${pick(i, 90)}">2</line><note><![CDATA[fragile & urgent]]></note></invoice>`;
      return { body: { kind: "binary", value: btoa(xml) }, content_type: "application/xml", body_size: xml.length };
    }
    case 2: {
      const text = `Label ${i} printed at station ${pick(i, 12)}`;
      return { body: { kind: "text", value: text }, content_type: "text/plain", body_size: text.length };
    }
    default: {
      const bytes = String.fromCharCode(0xff, 0xfe, 0x00, 0x01, i & 0xff, 0x7f, 0x80, 0x41);
      return { body: { kind: "binary", value: btoa(bytes) }, content_type: "application/octet-stream", body_size: 8 };
    }
  }
}

export interface Spec {
  entity: string;
  sub: "main" | "dlq" | "tdlq";
  count: number;
  firstSeq: number;
  /** Every n-th message is scheduled / deferred (the rest are active). */
  scheduledEvery?: number;
  deferredEvery?: number;
  /** Session ids, assigned round-robin. */
  sessions?: string[];
}

export function generate(s: Spec): MockMessage[] {
  return Array.from({ length: s.count }, (_, i) => {
    const seq = s.firstSeq + i;
    const state =
      s.scheduledEvery && i % s.scheduledEvery === s.scheduledEvery - 1
        ? "scheduled"
        : s.deferredEvery && i % s.deferredEvery === s.deferredEvery - 1
          ? "deferred"
          : "active";
    const dl = s.sub !== "main" ? REASONS[pick(i, REASONS.length)] : null;
    const props: Record<string, PropValue> = {
      tenant: { type: "string", value: ["contoso", "fabrikam", "northwind"][pick(i, 3)] },
      attempt: { type: "int", value: 1 + pick(i, 5) },
      priority: { type: "bool", value: pick(i, 4) === 0 },
    };
    if (dl) {
      props.DeadLetterReason = { type: "string", value: dl[0] };
      props.DeadLetterErrorDescription = { type: "string", value: dl[1] };
    }
    const enq = new Date(T0 - (s.count - i) * 37_000).toISOString();
    // Every fourth message carries a 14-day time to live.
    const ttl = i % 4 === 0 ? 14 * 86_400_000 : null;
    return {
      sub: s.sub,
      sequence_number: seq,
      state,
      message_id: `msg-${s.entity.replace(/\W+/g, "-")}-${seq}`,
      correlation_id: pick(i, 3) === 0 ? `corr-${pick(i, 999)}` : null,
      session_id: s.sessions ? s.sessions[i % s.sessions.length] : null,
      subject: SUBJECTS[pick(i, SUBJECTS.length)],
      to: null,
      reply_to: null,
      reply_to_session_id: null,
      partition_key: pick(i, 5) === 0 ? `pk-${pick(i, 8)}` : null,
      expires_at: ttl === null ? null : new Date(Date.parse(enq) + ttl).toISOString(),
      enqueued_time: enq,
      scheduled_enqueue_time: state === "scheduled" ? new Date(T0 + 3_600_000 + i * 60_000).toISOString() : null,
      locked_until: null,
      delivery_count: dl ? 10 : pick(i, 3),
      time_to_live_ms: ttl,
      dead_letter_reason: dl?.[0] ?? null,
      dead_letter_description: dl?.[1] ?? null,
      dead_letter_source: null,
      application_properties: props,
      body_truncated: false,
      ...body(i, pick(i + 7, 4)),
    };
  });
}

/** The body as text, as the engine's filters see it ("" for an empty body). */
export function textOf(m: MockMessage): string {
  const b = m.body;
  return b.kind === "text" ? b.value : b.kind === "binary" ? atob(b.value) : b.kind === "empty" ? "" : JSON.stringify(b.value);
}

/** A copy of `o` without the keys `ks`. */
export function without<T extends object, K extends keyof T>(o: T, ...ks: K[]): Omit<T, K> {
  const copy = { ...o };
  for (const k of ks) delete copy[k];
  return copy;
}

/** Tab, line breaks and printable ASCII: the mock's bodies are text when they are only these. */
const printable = (s: string) => [...s].every((ch) => {
  const c = ch.charCodeAt(0);
  return c === 9 || c === 10 || c === 13 || (c >= 32 && c <= 126);
});

/** As core's `MessageSummary::from`: no body, the first 160 characters of its text with whitespace collapsed. */
export function summarize(m: MockMessage): MessageSummary {
  const rest = without(m, "body", "body_truncated", "locked_until", "sub");
  const b = m.body;
  let text: string | null = null;
  if (b.kind === "text") text = b.value;
  else if (b.kind === "binary") {
    const raw = atob(b.value);
    text = printable(raw) ? raw : null;
  } else if (b.kind !== "empty") text = JSON.stringify(b.value);
  const preview = text ? text.split(/\s+/).join(" ").trim().slice(0, 160).trimEnd() || null : null;
  return { ...rest, body_kind: b.kind, body_size: m.body_size, body_preview: preview };
}

/** The message belongs to `view`: the dead-letter queues hold every state, the entity splits by state. */
export function accepts(view: PeekView, m: MockMessage): boolean {
  if (view === "dead-letter") return m.sub === "dlq";
  if (view === "transfer-dead-letter") return m.sub === "tdlq";
  return m.sub === "main" && m.state === view;
}
