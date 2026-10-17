import type { QueryClient } from "@tanstack/react-query";
import { normalizeFilter } from "@/engine";
import { defaultViewState, setViewState, viewKey } from "./viewStore";

/** Open an entity's active messages filtered to one session ("Peek messages" in Sessions). It sets the view's session
 *  condition the way the filter bar's Apply does: the filter goes through `normalizeFilter`, the filter bar opens so
 *  the condition shows, and earlier unobserved peeks of the view are dropped, since a recent peek of the same session
 *  must not come back from the cache. Other conditions of the filter are replaced: the session is what was asked for. */
export function peekSession(qc: QueryClient, profileId: string | null, entity: string, sessionId: string, peekSize: number) {
  void qc.removeQueries({ queryKey: ["peek", profileId, entity, "active"], type: "inactive" });
  setViewState(viewKey(profileId, entity, "active"), defaultViewState(peekSize), { filter: normalizeFilter({ session_id: sessionId }), filterOpen: true });
}
