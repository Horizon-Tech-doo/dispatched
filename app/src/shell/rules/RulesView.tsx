import { useRules, type RuleFilter, type RuleInfo } from "@/engine";
import { Button, EmptyState, Problem, RuleGlyph, SkeletonRows } from "@/ui";
import { useShell } from "../context";
import type { EntityTab } from "../state";

const norm = (s: string) => s.replace(/\s+/g, "");

export function ruleKind(f: RuleFilter): string {
  if (f.kind === "unknown") return "Unknown filter";
  if (f.kind === "correlation") return "Correlation filter";
  if (norm(f.expression) === "1=1") return "Matches every message";
  if (norm(f.expression) === "1=0") return "Matches no message";
  return "SQL filter";
}

const FIELDS: [keyof Extract<RuleFilter, { kind: "correlation" }>, string][] = [
  ["correlation_id", "Correlation ID"],
  ["message_id", "Message ID"],
  ["to", "To"],
  ["reply_to", "Reply to"],
  ["subject", "Subject"],
  ["session_id", "Session ID"],
  ["reply_to_session_id", "Reply to session ID"],
  ["content_type", "Content type"],
];

/** The fields a correlation filter sets, in the order the service lists them, then its custom properties. */
function correlationPairs(f: Extract<RuleFilter, { kind: "correlation" }>): (readonly [string, string])[] {
  return [
    ...FIELDS.filter(([k]) => f[k]).map(([k, label]) => [label, String(f[k])] as const),
    ...f.properties.map((p) => [p.name, p.value] as const),
  ];
}

function Filter({ f }: { f: RuleFilter }) {
  if (f.kind === "sql")
    return (
      <code className="block whitespace-pre-wrap break-all font-mono text-11" data-selectable>
        {f.expression}
      </code>
    );
  if (f.kind === "unknown") return <p className="m-0 text-12 text-muted">This kind of filter can't be shown here.</p>;
  const pairs = correlationPairs(f);
  // A correlation filter that sets nothing never matches (spec #794): say so where the user is reading the rule.
  if (pairs.length === 0) return <Problem title="This rule matches nothing" body="Its correlation filter sets no fields." />;
  return (
    <dl className="m-0 grid grid-cols-[max-content_1fr] gap-x-4 gap-y-0.5">
      {pairs.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="font-mono text-11 text-muted">{k}</dt>
          <dd className="m-0 break-all font-mono text-11" data-selectable>
            {v}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function Rule({ r }: { r: RuleInfo }) {
  return (
    <li aria-label={r.name} className="flex flex-col gap-1 border-b border-line px-3.5 py-2.5">
      <div className="flex items-baseline gap-2">
        <RuleGlyph aria-hidden="true" className="size-3.5 self-center text-icon" />
        <span className="text-12 font-semibold">{r.name}</span>
        <span className="text-11 text-muted">{ruleKind(r.filter)}</span>
      </div>
      <Filter f={r.filter} />
      {r.action && (
        <p className="m-0 text-11 text-muted">
          Action{" "}
          <code className="font-mono text-fg" data-selectable>
            {r.action}
          </code>
        </p>
      )}
    </li>
  );
}

export function RulesView({ tab }: { tab: EntityTab }) {
  const { conn } = useShell();
  const q = useRules(conn.profileId, tab.id);
  if (q.isPending) return <SkeletonRows rows={6} />;
  if (q.isError)
    return (
      <EmptyState
        tone="error"
        title="Couldn't list the rules"
        body="Listing rules needs Manage or Listen on this subscription."
        detail={q.error.message}
        action={<Button onClick={() => void q.refetch()}>Try again</Button>}
      />
    );
  if (q.data.length === 0) return <EmptyState title="No rules" body="A subscription without rules receives no messages." />;
  return (
    <div className="h-full overflow-y-auto" data-shot-scroller>
      <ul aria-label="Rules" className="m-0 list-none p-0">
        {q.data.map((r) => (
          <Rule key={r.name} r={r} />
        ))}
      </ul>
      <p className="m-0 px-3.5 py-2 text-11 text-muted">Rules are read-only here. Adding and editing them comes with entity management.</p>
    </div>
  );
}
