import { accessAdvice } from "../access";

export interface ListErrorText {
  /** One plain sentence and the next step; the raw message goes beneath it. */
  message: string;
  /** What gets the listing back: running it again, or fixing the connection's credentials. */
  fix: "retry" | "edit";
}

/** Plain-language text for a failed namespace listing, keyed on the engine's stable error kind. */
export function describeListError(kind: string, profile: { name: string; host: string; entra: boolean }): ListErrorText {
  const { name, host } = profile;
  switch (kind) {
    case "network":
    case "timeout":
      return { message: `The connection to ${host} was interrupted. Check your network or VPN, then try again.`, fix: "retry" };
    case "forbidden":
      return { message: `${accessAdvice(["Manage"], profile.entra ? "entra" : "connection-string")} You can also pin entities on the connection.`, fix: "edit" };
    case "not_signed_in":
      return { message: `Your sign-in for ${name} has expired. Edit the connection to sign in again.`, fix: "edit" };
    case "auth":
      // A rejected token on an Entra connection is an expired or revoked sign-in; on a connection string, a changed key.
      return profile.entra
        ? { message: `Your sign-in for ${name} has expired. Edit the connection to sign in again.`, fix: "edit" }
        : { message: `${host} rejected the credentials for ${name}. Edit the connection to replace them.`, fix: "edit" };
    default:
      return { message: `Something went wrong listing ${host}. Try again, or check the activity log for details.`, fix: "retry" };
  }
}

/** Kinds that mean the connection itself is gone, not just one listing: the status bar and selector say so. */
export type ConnectionTrouble = "lost" | "sign-in" | "credentials";

export function troubleOf(kind: string | undefined, entra: boolean): ConnectionTrouble | null {
  switch (kind) {
    case "network":
    case "timeout":
      return "lost";
    case "not_signed_in":
      return "sign-in";
    case "auth":
      return entra ? "sign-in" : "credentials";
    default:
      return null;
  }
}
