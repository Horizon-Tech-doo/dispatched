import { useEffect } from "react";
import { commands } from "@/engine";
import { useShell } from "../context";
import { tabKey, tabListener } from "./listenerStore";

/** A listener belongs to its tab: closing the tab stops it, so nothing goes on receiving out of sight. (A closed
 *  connection stops it in the engine.) */
export function ListenerGuard() {
  const { shell, conn } = useShell();
  const open = new Set(shell.tabs.map((t) => tabKey(conn.profileId, t.id)));
  const key = [...open].sort().join("\n");
  useEffect(() => {
    const prefix = `${conn.profileId ?? ""}|`;
    for (const k of tabListener.keys()) {
      if (!k.startsWith(prefix) || open.has(k)) continue;
      const l = tabListener.get(k);
      tabListener.set(k, null);
      if (l) void commands.jobCancel(l.jobId).catch(() => {});
    }
    // `key` is the set of open tabs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, conn.profileId]);
  return null;
}
