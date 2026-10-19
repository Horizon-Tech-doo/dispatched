import type { Query, QueryClient } from "@tanstack/react-query";

/** The topic of a subscription id (`topic/subscriptions/name`), else null. */
const topicOf = (entity: string) => (entity.includes("/subscriptions/") ? entity.split("/subscriptions/")[0] : null);

/**
 * Everything that shows one entity's messages or counts is read again: its message views (reset, so a view starts a
 * fresh round), the open message, its Overview, sessions and dead-letter summary, and the tree and Dashboard counts.
 */
export function invalidateEntity(qc: QueryClient, profileId: string | null, entity: string): void {
  if (!profileId) return;
  const about = (q: Query) => q.queryKey[1] === profileId && q.queryKey.includes(entity);
  void qc.resetQueries({ queryKey: ["peek", profileId, entity] });
  for (const root of ["message", "sessions", "dlq-summary", "session-state"]) void qc.invalidateQueries({ queryKey: [root, profileId, entity] });
  void qc.invalidateQueries({ queryKey: ["entity-details"], predicate: about });
  void qc.invalidateQueries({ queryKey: ["dashboard", profileId] });
  void qc.invalidateQueries({ queryKey: ["entities", profileId] });
  const topic = topicOf(entity);
  if (topic) void qc.invalidateQueries({ queryKey: ["subscriptions", profileId, topic] });
}
