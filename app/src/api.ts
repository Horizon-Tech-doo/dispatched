import { invoke } from "@tauri-apps/api/core";

export type SubQueue = "active" | "dead-letter" | "transfer-dead-letter";

export interface CommandError { kind: string; message: string }
export interface ConnectionInfo { host: string; emulator: boolean; auth: string }
export interface EntityCounts { active: number; dead_letter: number; scheduled: number; transfer: number; transfer_dead_letter: number; approximate: boolean; truncated: boolean }
export interface QueueInfo { name: string; status: string | null; counts: EntityCounts | null; requires_session: boolean }
export type MessageBody =
  | { kind: "binary"; value: string }
  | { kind: "text"; value: string }
  | { kind: "value" | "sequence"; value: unknown }
  | { kind: "empty" };
export interface PeekedMessage {
  sequence_number: number;
  state: "active" | "deferred" | "scheduled";
  message_id: string | null;
  subject: string | null;
  enqueued_time: string | null;
  delivery_count: number;
  dead_letter_reason: string | null;
  dead_letter_description: string | null;
  body: MessageBody;
}

export const connectConnectionString = (connectionString: string) =>
  invoke<ConnectionInfo>("connect_connection_string", { connectionString });
export const disconnect = () => invoke<void>("disconnect");
export const listQueues = () => invoke<QueueInfo[]>("list_queues");
export const peek = (entity: string, subQueue: SubQueue, fromSequence = 0, max = 50) =>
  invoke<PeekedMessage[]>("peek", { entity, subQueue, fromSequence, max });

export function bodyPreview(body: MessageBody): string {
  switch (body.kind) {
    case "empty":
      return "";
    case "text":
      return body.value;
    case "value":
    case "sequence":
      return JSON.stringify(body.value) ?? "";
    case "binary": {
      const bytes = Uint8Array.from(atob(body.value), (c) => c.charCodeAt(0));
      try {
        return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      } catch {
        return `<${bytes.length} bytes>`;
      }
    }
  }
}
