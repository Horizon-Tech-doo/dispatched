import { useProfiles } from "@/engine";
import { useShell } from "./context";

/** How the connection signs in: the advice for a missing right differs (a role, or a key's policy). */
export type AuthKind = "entra" | "connection-string";

/** A right the engine can say is missing. `Manage` stands for the management rights (`Manage`, `EntityRead`). */
export type Right = "Listen" | "Manage";

/** The rights an engine message names ("access denied (needs Listen): ...", "(needs Manage,EntityRead)").
 *  Both Listen and Manage may be named; with nothing named, the surface's own `fallback` applies. */
export function rightsOf(message: string, fallback: Right): Right[] {
  const named = /needs ([A-Za-z,]+)/.exec(message)?.[1]?.split(",") ?? [];
  const rights = new Set<Right>();
  for (const n of named) {
    if (n === "Listen") rights.add("Listen");
    else if (n === "Manage" || n === "EntityRead") rights.add("Manage");
  }
  return rights.size > 0 ? [...rights] : [fallback];
}

/** What to do about a missing right, by how the connection signs in. Management needs Manage (Data Owner with
 *  Entra) because Service Bus's management API has no reader role; Listen alone is the Data Receiver role. */
export function accessAdvice(rights: readonly Right[], auth: AuthKind): string {
  const manage = rights.includes("Manage");
  if (auth === "entra")
    return manage
      ? "Your account needs the Azure Service Bus Data Owner role on this namespace. Data Receiver and Data Sender don't include listing entities or reading their properties and counts. After it's assigned, try again."
      : "Your account needs the Azure Service Bus Data Receiver role on this namespace or entity. Owner and Contributor don't include reading messages. After it's assigned, peek again.";
  return manage
    ? "This key has no Manage right. Use a key whose policy includes Manage."
    : "This key has no Listen right. Use a key whose policy includes Listen (or Manage).";
}

/** The auth kind of the connected profile; until the profile list is known, a connection string. */
export function useAuthKind(): AuthKind {
  const { conn } = useShell();
  const profile = useProfiles().data?.find((p) => p.id === conn.profileId);
  return profile?.auth.kind === "entra" ? "entra" : "connection-string";
}

/** Whether a query error is the engine's `forbidden` (its errors carry a `kind`). */
export const isForbidden = (e: unknown): boolean => (e as { kind?: string } | null | undefined)?.kind === "forbidden";
