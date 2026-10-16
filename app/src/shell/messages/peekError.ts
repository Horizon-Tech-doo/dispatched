import { accessAdvice, rightsOf, type AuthKind } from "../access";

export interface PeekErrorText {
  title: string;
  body: string;
  /** The way out: try again, open the Sessions sub-tab, or edit the connection. */
  fix: "retry" | "sessions" | "edit";
}

/** What a failed peek means for the user, by engine error kind; the raw message goes on the detail line. */
export function describePeekError(kind: string, message: string, viewLabel: string, auth: AuthKind): PeekErrorText {
  if (/session/i.test(message) && /(requires|require|enabled|sessionful)/i.test(message))
    return { title: "This entity requires sessions", body: "Open Sessions and peek one session's messages.", fix: "sessions" };
  switch (kind) {
    case "forbidden": {
      // Trying again can't grant a right: the way out is the connection (another key, or a role assigned in Azure).
      return { title: `Can't read the ${viewLabel}`, body: accessAdvice(rightsOf(message, "Listen"), auth, "peek again"), fix: "edit" };
    }
    case "not_found":
      return { title: "This entity wasn't found", body: "It may have been deleted or renamed. Refresh the tree to see what is there now.", fix: "retry" };
    case "not_signed_in":
      return { title: "Sign-in expired", body: "Sign in again from the connection, then try again.", fix: "edit" };
    case "auth":
      return { title: "Credentials rejected", body: "Replace the connection string or sign in again, then try again.", fix: "edit" };
    case "network":
    case "timeout":
      return { title: "Couldn't reach the namespace", body: "Check your network connection, then try again.", fix: "retry" };
    case "throttled":
      return { title: "The namespace is busy", body: "Service Bus is throttling requests. Wait a moment, then try again.", fix: "retry" };
    default:
      return { title: "Couldn't peek messages", body: "Try again. If it keeps failing, the detail below says why.", fix: "retry" };
  }
}
