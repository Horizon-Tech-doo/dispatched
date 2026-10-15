import type { EntityKind } from "./state";

const SEP = "/subscriptions/";

/** The tab id of an entity, which is also a valid `EntityPath`: a queue or topic name, or
 *  `${topic}/subscriptions/${name}` for a subscription. */
export function entityIdOf(kind: EntityKind, name: string, topic?: string): string {
  return kind === "subscription" ? `${topic}${SEP}${name}` : name;
}

/** The topic and name of a subscription id. Topic names may contain `/` and subscription names may not,
 *  so the id splits on its last `/subscriptions/` (any case), as the engine's `EntityPath::parse` does. */
export function splitSubscriptionId(id: string): { topic: string; name: string } {
  const at = id.toLowerCase().lastIndexOf(SEP);
  if (at < 0) return { topic: "", name: id };
  return { topic: id.slice(0, at), name: id.slice(at + SEP.length) };
}
