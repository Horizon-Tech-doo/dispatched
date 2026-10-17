import { splitSubscriptionId } from "./entityId";
import type { EntityKind } from "./state";

/** The entity's path under the namespace host, as core's `EntityPath::base`: a queue or topic name, or
 *  `${topic}/Subscriptions/${name}`. */
export function entityBase(id: string, kind: EntityKind): string {
  if (kind !== "subscription") return id;
  const { topic, name } = splitSubscriptionId(id);
  return `${topic}/Subscriptions/${name}`;
}
export const entityUrl = (host: string, id: string, kind: EntityKind) => `sb://${host}/${entityBase(id, kind)}`;
/** The dead-letter queue's address; a topic has none. */
export const deadLetterUrl = (host: string, id: string, kind: EntityKind) => (kind === "topic" ? null : `${entityUrl(host, id, kind)}/$DeadLetterQueue`);
