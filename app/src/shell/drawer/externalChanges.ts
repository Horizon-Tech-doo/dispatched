import type { QueryClient } from "@tanstack/react-query";
import { commands, type ActivityRecord } from "@/engine";
import { invalidateEntity } from "../operate/invalidate";
import { recordStore } from "./activity";

/** How often the shared log is read for changes made elsewhere. */
export const POLL_MS = 3000;

/**
 * Follows the shared activity log for changes the CLI or an agent made: every few seconds while the window has focus,
 * the entries written since the last look go to the Activity tab, and each finished change on the connected profile
 * by anyone but this app has its entity's views read again.
 */
export function createExternalChanges(o: {
  qc: QueryClient;
  /** The connected profile, or null. Entries name profiles by name. */
  profile: () => { id: string; name: string } | null;
  focused?: () => boolean;
  intervalMs?: number;
}) {
  // Null until the first look, which only finds where the log ends now.
  let after: number | null = null;
  let busy = false;
  const focused = o.focused ?? (() => typeof document === "undefined" || document.hasFocus());

  async function poll(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const first = after === null;
      const records: ActivityRecord[] = await commands.activitySince(after);
      if (records.length) after = Math.max(after ?? 0, ...records.map((r) => r.id));
      else if (first) after = 0;
      if (first) return;
      recordStore.merge(records);
      const p = o.profile();
      if (!p) return;
      const touched = new Set(records.filter((r) => r.phase === "finished" && r.origin !== "gui" && r.profile === p.name && r.entity).map((r) => r.entity!));
      for (const entity of touched) invalidateEntity(o.qc, p.id, entity);
    } catch {
      // The log could not be read this time; the next look tries again.
    } finally {
      busy = false;
    }
  }

  return {
    poll,
    /** Starts following; returns the stop. */
    start(): () => void {
      const t = setInterval(() => {
        if (focused()) void poll();
      }, o.intervalMs ?? POLL_MS);
      return () => clearInterval(t);
    },
  };
}
