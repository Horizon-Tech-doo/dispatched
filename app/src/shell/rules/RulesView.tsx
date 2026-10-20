import { CircleAlert } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import { useRules, type RuleFilter, type RuleInfo } from "@/engine";
import { Button, EmptyState, RuleGlyph, SkeletonRows } from "@/ui";
import { accessAdvice, isForbidden, rightsOf, useAuthKind } from "../access";
import { useShell } from "../context";
import type { EntityTab } from "../state";

const norm = (s: string) => s.replace(/\s+/g, "");

type Correlation = Extract<RuleFilter, { kind: "correlation" }>;

const FIELDS: [keyof Correlation, string][] = [
  ["correlation_id", "Correlation ID"],
  ["message_id", "Message ID"],
  ["to", "To"],
  ["reply_to", "Reply to"],
  ["subject", "Subject"],
  ["session_id", "Session ID"],
  ["reply_to_session_id", "Reply to session ID"],
  ["content_type", "Content type"],
];

/** The fields a correlation filter sets, in the order the service lists them. */
const correlationFields = (f: Correlation): (readonly [string, string])[] => FIELDS.filter(([k]) => f[k]).map(([k, label]) => [label, String(f[k])] as const);

/** A correlation filter that sets nothing matches no message (spec #794). */
const matchesNothing = (f: Correlation) => correlationFields(f).length === 0 && f.properties.length === 0;

export function ruleKind(f: RuleFilter): string {
  if (f.kind === "unknown") return "Unknown filter";
  if (f.kind === "true") return "Matches every message (default rule)";
  if (f.kind === "correlation") return matchesNothing(f) ? "Matches no message" : "Correlation filter";
  if (norm(f.expression) === "1=1") return "Matches every message";
  if (norm(f.expression) === "1=0") return "Matches no message";
  return "SQL filter";
}

const LABEL = "text-12 text-muted";

/** One labelled row of a rule's body. The label column has one width in every rule, so the bodies line up down the list. */
function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className={LABEL}>{label}</dt>
      <dd className="m-0 min-w-0">{children}</dd>
    </>
  );
}

const code = "block whitespace-pre-wrap break-all font-mono text-11 text-fg";

function Body({ f, action }: { f: RuleFilter; action: string | null }) {
  const fields = f.kind === "correlation" ? correlationFields(f) : [];
  return (
    <dl className="m-0 grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-x-4 gap-y-0.5">
      {f.kind === "sql" && (
        <Row label="Filter">
          <code className={code} data-selectable>{f.expression}</code>
        </Row>
      )}
      {f.kind === "unknown" && (
        <Row label="Filter">
          <span className="text-12 text-muted">This kind of filter can't be shown here.</span>
        </Row>
      )}
      {f.kind === "correlation" && matchesNothing(f) && (
        <Row label="Filter">
          <span className="flex items-center gap-1.5 text-12 text-fg">
            <CircleAlert aria-hidden="true" className="size-3.5 shrink-0 text-icon" />
            Sets no fields, so no message matches it.
          </span>
        </Row>
      )}
      {fields.map(([label, value], i) => (
        <Row key={`${i}-${label}`} label={label}>
          <code className={code} data-selectable>{value}</code>
        </Row>
      ))}
      {f.kind === "correlation" && f.properties.length > 0 && (
        <>
          <dt className="col-span-2 mt-1 text-11 text-muted">Properties</dt>
          {f.properties.map((p, i) => (
            <Fragment key={`${i}-${p.name}`}>
              <dt className="break-all font-mono text-11 text-muted">{p.name}</dt>
              <dd className="m-0 min-w-0"><code className={code} data-selectable>{p.value}</code></dd>
            </Fragment>
          ))}
        </>
      )}
      {action && (
        <Row label="Action">
          <code className={code} data-selectable>{action}</code>
        </Row>
      )}
    </dl>
  );
}

function Rule({ r }: { r: RuleInfo }) {
  return (
    <li aria-label={r.name} className="flex flex-col gap-1.5 border-b border-line px-3.5 py-2.5">
      <div className="flex items-baseline gap-2">
        <RuleGlyph aria-hidden="true" className="size-3.5 self-center text-icon" />
        <span className="text-12 font-semibold">{r.name}</span>
        <span className="text-11 text-muted">{ruleKind(r.filter)}</span>
      </div>
      <Body f={r.filter} action={r.action} />
    </li>
  );
}

export function RulesView({ tab }: { tab: EntityTab }) {
  const { conn, openSheet } = useShell();
  const auth = useAuthKind();
  const q = useRules(conn.profileId, tab.id);
  if (q.isPending) return <SkeletonRows rows={6} />;
  if (q.isError) {
    const denied = isForbidden(q.error);
    return (
      <EmptyState
        tone="error"
        title="Couldn't list the rules"
        body={denied ? accessAdvice(rightsOf(q.error.message, "Manage"), auth) : "Try again. If it keeps failing, the detail below says why."}
        detail={q.error.message}
        action={
          denied && conn.profileId ? (
            <Button onClick={() => openSheet({ kind: "profile", profileId: conn.profileId! })}>Edit connection</Button>
          ) : (
            <Button onClick={() => void q.refetch()}>Try again</Button>
          )
        }
      />
    );
  }
  if (q.data.length === 0) return <EmptyState title="No rules" body="A subscription without rules receives no messages." />;
  return (
    <div className="h-full overflow-y-auto" data-shot-scroller>
      <ul aria-label="Rules" className="m-0 list-none p-0">
        {q.data.map((r) => (
          <Rule key={r.name} r={r} />
        ))}
      </ul>
      <p className="m-0 px-3.5 py-2 text-11 text-muted">Rules are read-only here.</p>
    </div>
  );
}
