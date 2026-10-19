import type { EngineError } from "@/engine";
import { accessAdvice, type AuthKind } from "../access";

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

/** What to do about a failed check of a connection test, from the engine's error kind. */
export function checkAdvice(label: "Management" | "Messaging", kind: string, auth: AuthKind): string {
  switch (kind) {
    case "forbidden":
      return accessAdvice([label === "Management" ? "Manage" : "Listen"], auth);
    case "auth":
    case "not_signed_in":
      return "The credentials were refused. Check them, or sign in again.";
    case "network":
    case "timeout":
      return "Couldn't reach the namespace. Check the network, the proxy and the transport.";
    case "not_found":
      return "The namespace or entity wasn't found. Check the namespace name.";
    default:
      return "Check the connection settings and test again.";
  }
}

/** A skipped check's reason in the app's words: Rust's two pinned reasons are worded for the command line. */
export function skippedReason(reason: string): string {
  if (reason.includes("management check failed"))
    return "No queue could be listed because the Management check failed. Pin an entity to test receive rights.";
  if (reason.includes("no entity to probe")) return "No entity to test. Pin one to test receive rights.";
  return sentence(reason);
}
