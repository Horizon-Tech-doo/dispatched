import type { MessageBody, MessageDetail } from "@/engine";
import { base64ToBytes, utf8OrNull } from "@/ui";

/** The body decoded once: its bytes and its text. `bytes` is null for value and sequence bodies (AMQP values with no
 *  wire bytes here), for an empty body, and for base64 that does not decode. `text` is the UTF-8 text of a binary body
 *  (null when it is not UTF-8), the text body itself, or the compact JSON of a value or sequence body (an empty body is "").
 *  A body cut at the display cap (`truncated`) may end inside a multi-byte character: up to 3 trailing bytes are
 *  ignored then, so a UTF-8 body that was only cut is still text. */
export function decodeBody(b: MessageBody, truncated = false): { bytes: Uint8Array | null; text: string | null } {
  switch (b.kind) {
    case "binary": {
      const bytes = base64ToBytes(b.value);
      if (!bytes) return { bytes: null, text: null };
      let text = utf8OrNull(bytes);
      for (let cut = 1; text === null && truncated && cut <= 3 && cut < bytes.length; cut++) text = utf8OrNull(bytes.subarray(0, bytes.length - cut));
      return { bytes, text };
    }
    case "text":
      return { bytes: new TextEncoder().encode(b.value), text: b.value };
    case "value":
    case "sequence":
      return { bytes: null, text: JSON.stringify(b.value) };
    default:
      return { bytes: null, text: "" };
  }
}

/** The body's bytes (see `decodeBody`). */
export const bodyBytes = (b: MessageBody): Uint8Array | null => decodeBody(b).bytes;

/** The body as text (see `decodeBody`); the one decode path copy and export use. */
export const bodyText = (b: MessageBody): string | null => decodeBody(b).text;

/** One message as the export file writes it (`dispatched.message/v1`). `body_text` only for binary bodies that are
 *  UTF-8. The display-only fields (`body_size`, `body_truncated`) are left out. */
export function toExportJson(d: MessageDetail): string {
  const message: Partial<MessageDetail> = { ...d };
  delete message.body_size;
  delete message.body_truncated;
  const text = d.body.kind === "binary" ? bodyText(d.body) : null;
  return JSON.stringify({ schema: "dispatched.message/v1", ...message, ...(text !== null ? { body_text: text } : {}) }, null, 2);
}
