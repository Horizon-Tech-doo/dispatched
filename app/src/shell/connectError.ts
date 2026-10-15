import type { EngineError } from "@/engine";
import { shortHost } from "./ConnectionSelector";

export interface ConnectErrorText {
  /** Sentence for people: what happened and what to do. */
  message: string;
  /** Whether trying again unchanged can help; if not, the profile needs editing. */
  retryable: boolean;
}

const sentence = (s: string) => {
  const t = s.trim();
  if (!t) return "Something went wrong.";
  const cap = t[0].toUpperCase() + t.slice(1);
  return /[.!?]$/.test(cap) ? cap : `${cap}.`;
};

/** Plain-language text for a failed connect, keyed on the engine's stable error kind. */
export function describeConnectError(
  error: Pick<EngineError, "kind" | "message"> | null,
  profile: { name: string; host: string },
): ConnectErrorText {
  const host = shortHost(profile.host);
  switch (error?.kind) {
    case "invalid":
      if (/no stored connection string/.test(error.message)) {
        return {
          message: `The connection string for ${profile.name} isn't in the system keychain. Edit the connection and paste it again.`,
          retryable: false,
        };
      }
      return { message: sentence(error.message), retryable: false };
    case "not_signed_in":
      return { message: `Your Entra sign-in for ${profile.name} has expired. Edit the connection to sign in again.`, retryable: false };
    case "auth":
      return { message: `${host} rejected the credentials. Edit the connection to replace them.`, retryable: false };
    case "network":
    case "timeout":
      return { message: `Couldn't reach ${host}. Check your network or proxy, then try again.`, retryable: true };
    case "forbidden":
      return { message: `This identity isn't allowed to connect to ${host}.`, retryable: false };
    default:
      return { message: sentence(error?.message ?? ""), retryable: true };
  }
}
