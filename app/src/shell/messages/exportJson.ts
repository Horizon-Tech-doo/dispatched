import type { MessageBody, MessageDetail } from "@/engine";
import { base64ToBytes, utf8OrNull } from "@/ui";

/** The body's bytes: decoded for a binary body, UTF-8 for a text body. `null` for value and sequence bodies (they
 *  are AMQP values with no wire bytes here), for an empty body, and for base64 that does not decode. */
export function bodyBytes(b: MessageBody): Uint8Array | null {
  if (b.kind === "binary") return base64ToBytes(b.value);
  if (b.kind === "text") return new TextEncoder().encode(b.value);
  return null;
}

/** The body as text: the UTF-8 text of a binary body (`null` when it is not UTF-8), the text body itself, or the
 *  JSON of a value or sequence body. This is the one decode path; copy and export both use it. */
export function bodyText(b: MessageBody): string | null {
  switch (b.kind) {
    case "binary": {
      const bytes = base64ToBytes(b.value);
      return bytes ? utf8OrNull(bytes) : null;
    }
    case "text":
      return b.value;
    case "value":
    case "sequence":
      return JSON.stringify(b.value);
    default:
      return "";
  }
}

/** One message as the export file writes it (`dispatched.message/v1`). `body_text` only for binary bodies that are
 *  UTF-8. The display-only fields (`body_size`, `body_truncated`) are left out. */
export function toExportJson(d: MessageDetail): string {
  const message: Partial<MessageDetail> = { ...d };
  delete message.body_size;
  delete message.body_truncated;
  const text = d.body.kind === "binary" ? bodyText(d.body) : null;
  return JSON.stringify({ schema: "dispatched.message/v1", ...message, ...(text !== null ? { body_text: text } : {}) }, null, 2);
}
