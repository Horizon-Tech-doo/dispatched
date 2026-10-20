import type { JobKind } from "@/engine";
import { accessAdvice, rightsOf, type AuthKind, type Right } from "../access";

/** What to do about a failure, by its kind. The engine's own words go on the detail line. */
export function nextStep(kind: string): string {
  switch (kind) {
    case "network":
    case "timeout":
    case "throttled":
    case "protocol":
      return "Check the connection, then try again.";
    case "auth":
    case "not_signed_in":
      return "Sign in again, then try again.";
    case "not_found":
      return "The entity may have been deleted. Refresh the tree, then try again.";
    case "not_connected":
      return "The connection has closed. Connect again, then try again.";
    case "invalid":
      return "Check the fields, then try again.";
    case "conflict":
      return "Something else changed it meanwhile. Read it again, then try again.";
    case "activity_unavailable":
      return "Nothing that changes messages can run without the activity log. Restart Dispatched.";
    default:
      return "Try again. If it keeps failing, the detail below says why.";
  }
}

/** The right a job kind needs when the engine does not name it. */
export const rightFor = (kind: JobKind | "count"): Right =>
  kind === "send" || kind === "import"
    ? "Send"
    : kind === "purge" || kind === "session-state" || MANAGE.has(kind)
      ? "Manage"
      : "Listen";

/** Job kinds that change entities: they need Manage. */
const MANAGE = new Set<JobKind | "count">(["create-entities", "update-entities", "delete-entities", "duplicate", "rename", "import-entities"]);

/** Cause and next step for an operate failure: a missing right in the connection's own sign-in terms, else the kind's step. */
export function adviceFor(kind: string, message: string, fallback: Right, auth: AuthKind): string {
  return kind === "forbidden" ? accessAdvice(rightsOf(message, fallback), auth) : nextStep(kind);
}
