import type { Query, QueryClient } from "@tanstack/react-query";

/** The tab id of an entity as the core names it: a subscription's `topic/Subscriptions/name` becomes
 *  `topic/subscriptions/name`. */
export const entityIdOfPath = (path: string) => path.replace("/Subscriptions/", "/subscriptions/");

/** The topic of a subscription id (`topic/subscriptions/name`), else null. */
const topicOf = (entity: string) => (entity.includes("/subscriptions/") ? entity.split("/subscriptions/")[0] : null);

/**
 * Everything that shows one entity's messages or counts is read again: its message views (reset, so a view starts a
 * fresh round), the open message, its Overview, sessions, dead-letter summary, rules, settings and policies, and the tree
 * and Dashboard counts. `path` may be the core's form of a subscription (`topic/Subscriptions/name`).
 */
export function invalidateEntity(qc: QueryClient, profileId: string | null, path: string): void {
  if (!profileId) return;
  const entity = entityIdOfPath(path);
  const about = (q: Query) => q.queryKey[1] === profileId && q.queryKey.includes(entity);
  void qc.resetQueries({ queryKey: ["peek", profileId, entity] });
  for (const root of ["message", "sessions", "dlq-summary", "session-state", "rules", "settings", "policies"]) void qc.invalidateQueries({ queryKey: [root, profileId, entity] });
  void qc.invalidateQueries({ queryKey: ["entity-details"], predicate: about });
  void qc.invalidateQueries({ queryKey: ["dashboard", profileId] });
  void qc.invalidateQueries({ queryKey: ["entities", profileId] });
  const topic = topicOf(entity);
  if (topic) void qc.invalidateQueries({ queryKey: ["subscriptions", profileId, topic] });
}
