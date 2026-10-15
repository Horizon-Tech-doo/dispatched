import type { EngineError } from "@/engine";

export type Errors = Partial<Record<"name" | "connectionString" | "namespace" | "proxy" | "pins" | "signIn" | "general", string>>;

/** A message for people: no code quotes, first letter capitalised, ends with a full stop. */
export function sentence(message: string): string {
  const t = message.replace(/`/g, "").trim();
  if (!t) return "Something went wrong.";
  const cap = t[0].toUpperCase() + t.slice(1);
  return /[.!?]$/.test(cap) ? cap : `${cap}.`;
}

/** Put a server error under the field it is about; anything else is a general error. */
export function placeError(e: Pick<EngineError, "kind" | "message">): Errors {
  const m = sentence(e.message);
  if (e.kind === "not_signed_in") return { signIn: m };
  if (e.kind === "invalid") {
    if (/already exists|needs a name/i.test(m)) return { name: m };
    if (/connection string|\bEndpoint\b|SharedAccess/i.test(m)) return { connectionString: m };
    if (/namespace/i.test(m)) return { namespace: m };
    if (/proxy/i.test(m)) return { proxy: m };
    if (/pinned|entity/i.test(m)) return { pins: m };
  }
  return { general: m };
}
