import { keys, useProfiles } from "@/engine";
import { useShell } from "./context";
import { useQueryState } from "./queryState";
import { troubleOf, type ConnectionTrouble } from "./tree/listError";

/** Whether the connected namespace stopped answering (lost) or stopped accepting the credentials,
 *  judged from the latest listing. The status bar and the connection selector both show it. */
export function useConnectionTrouble(): ConnectionTrouble | null {
  const { conn } = useShell();
  const profile = useProfiles().data?.find((p) => p.id === conn.profileId);
  const entities = useQueryState(keys.entities(conn.status === "connected" ? conn.profileId : null));
  if (conn.status !== "connected") return null;
  return troubleOf(entities.error?.kind, profile?.auth.kind === "entra");
}

/** The status bar's words for each kind of trouble. */
export const troubleLabel: Record<ConnectionTrouble, string> = {
  lost: "Connection lost",
  "sign-in": "Sign-in expired",
  credentials: "Credentials rejected",
};
