import type { JobKind } from "@/engine";
import { formatNumber } from "@/ui";
import type { Job } from "../drawer/activity";

/** What a job did to each message, in the past tense: "deleted", "moved". */
export const PAST: Record<JobKind, string> = {
  export: "written", send: "sent", import: "imported", "cancel-scheduled": "unscheduled", receive: "received", "settle-deferred": "settled",
  delete: "deleted", move: "moved", resubmit: "resubmitted", purge: "removed", "session-state": "changed",
  "create-entities": "created", "update-entities": "updated", "delete-entities": "deleted", duplicate: "duplicated", rename: "renamed",
  "import-entities": "imported", listen: "received",
};

/** The confirm button: the core's verb, naming the entity this confirmation is about (`Delete from orders`, then for a
 *  move's second confirmation `Move to billing`). */
export function confirmLabel(verb: string, target: string, step: number): string {
  switch (verb) {
    case "delete":
      return `Delete from ${target}`;
    case "move":
      return step > 1 ? `Move to ${target}` : `Move from ${target}`;
    case "resubmit":
      return step > 1 ? `Resubmit to ${target}` : `Resubmit from ${target}`;
    case "send":
      return `Send to ${target}`;
    case "schedule":
      return `Schedule to ${target}`;
    case "import":
      return `Import into ${target}`;
    case "purge":
      return `Purge ${target}`;
    case "cancel-scheduled":
      return `Unschedule in ${target}`;
    case "receive-and-delete":
      return `Receive and delete from ${target}`;
    case "complete":
    case "abandon":
    case "defer":
    case "dead-letter":
      return `${verb[0].toUpperCase()}${verb.slice(1)} in ${target}`;
    case "listen-complete":
    case "listen-delete":
      return `Start listening to ${target}`;
    case "set-session-state":
      return `Replace state in ${target}`;
    case "clear-session-state":
      return `Clear state in ${target}`;
    default:
      return "Confirm";
  }
}

/** Items done so far, by any outcome. */
export const itemsSoFar = (j: Job) => Object.values(j.tally).reduce((a, b) => a + b, 0);

/** Where a running operate job stands: "5 of 37", or "5 done" without a total. */
export function runningText(j: Job): string {
  if (j.confirm) return "Waiting for confirmation";
  const n = Math.max(j.done, itemsSoFar(j));
  const failed = j.tally.failed ? ` · ${formatNumber(j.tally.failed)} failed` : "";
  return `${j.total != null ? `${formatNumber(n)} of ${formatNumber(j.total)}` : `${formatNumber(n)} done`}${failed}`;
}

/** A finished job's counts: "3 deleted", "2 of 3 deleted, 1 failed". Null when there is no report. */
export function outcomeText(j: Job): { text: string; failed: boolean } | null {
  const r = j.report;
  if (!r) return null;
  const past = PAST[j.kind];
  const others = r.incomplete - r.failed - r.missing;
  const rest = [r.failed > 0 && `${formatNumber(r.failed)} failed`, r.missing > 0 && `${formatNumber(r.missing)} missing`, others > 0 && `${formatNumber(others)} not done`].filter(Boolean);
  const text = rest.length === 0 && r.succeeded === r.items ? `${formatNumber(r.succeeded)} ${past}` : `${formatNumber(r.succeeded)} of ${formatNumber(r.items)} ${past}${rest.length ? `, ${rest.join(", ")}` : ""}`;
  return { text, failed: r.failed > 0 };
}

/** The state column of a finished operate job. */
export function finishedState(j: Job): string {
  const kind = endKind(j);
  if (kind === "cancelled") return "Cancelled";
  if (kind === "stopped") return "Stopped";
  if (kind === "denied") return "Not confirmed";
  if (j.error) return "Failed";
  if (j.report?.outcome === "partly-failed") return "Partly failed";
  if (j.report?.outcome === "failed") return "Failed";
  return "Done";
}

/** A neutral sentence for a job that ended without failing: not confirmed, cancelled, or stopped by a closing connection. */
export function endedText(j: Job): string | null {
  const kind = endKind(j);
  const n = j.report?.succeeded ?? 0;
  const sofar = `${formatNumber(n)} ${PAST[j.kind]}`;
  if (kind === "denied") return "Not confirmed. Nothing was changed.";
  if (kind === "cancelled") return j.report && n > 0 ? `Cancelled after ${sofar}.` : j.report ? "Cancelled." : "Cancelled. Nothing was changed.";
  if (kind === "stopped") return j.report && n > 0 ? `Stopped: the connection closed after ${sofar}.` : "Stopped: the connection closed.";
  return null;
}

/** How a job ended when it did not just finish: its error kind, or `cancelled` for a report that says it was
 *  cancelled even without an error. */
export const endKind = (j: Job): string | undefined => j.error?.kind ?? (j.report?.cancelled || j.report?.outcome === "cancelled" ? "cancelled" : undefined);

/** The error kinds that are not a failure: the user's choice, or a closing connection. */
export const neutralEnd = (kind: string | undefined) => kind === "cancelled" || kind === "stopped" || kind === "denied";

/** The heading of a failed operate job: "Delete failed". */
export function failedTitle(kind: JobKind): string {
  const name: Record<JobKind, string> = {
    export: "Export", send: "Send", import: "Import", "cancel-scheduled": "Unschedule", receive: "Receive", "settle-deferred": "Settle",
    delete: "Delete", move: "Move", resubmit: "Resubmit", purge: "Purge", "session-state": "Session state change",
    "create-entities": "Create", "update-entities": "Change", "delete-entities": "Delete", duplicate: "Duplicate", rename: "Rename",
    "import-entities": "Import", listen: "Listener",
  };
  return `${name[kind]} failed`;
}
