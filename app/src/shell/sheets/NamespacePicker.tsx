import { ChevronRight } from "lucide-react";
import { useEffect, useId, useMemo, useState, type KeyboardEvent } from "react";
import { useAccount, useDiscoveredNamespaces, useTenants, type DiscoveredNamespace, type EngineError, type TenantView } from "@/engine";
import { Button, EmptyState, Field, Input, Problem, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Skeleton, cn } from "@/ui";
import { sentence } from "./profileErrors";
import { OtherAccountNotice, SignInProgress, useSignInFlow } from "./SignInPanel";

/** More namespaces than this get a filter box. */
export const FILTER_THRESHOLD = 8;

/** The Select value for "the account's home tenant" when the tenant list does not name it. */
const HOME_VALUE = "__home__";

export const tenantLabel = (t: TenantView) => {
  const name = t.display_name ?? t.default_domain ?? t.tenant_id;
  return t.home ? `${name} (home)` : name;
};

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

const matches = (ns: DiscoveredNamespace, q: string) =>
  [ns.name, ns.subscription_name, ns.location, ns.tier].some((v) => v?.toLowerCase().includes(q));

const label = (ns: DiscoveredNamespace) => [ns.name, ns.subscription_name, ns.location, ns.tier].filter(Boolean).join(", ");

/** The list's columns: name, subscription (takes what is left and truncates), region, tier. Regions and tiers
 *  are short and line up in their own columns. */
const COLUMNS = "grid-cols-[minmax(0,38%)_minmax(0,1fr)_68px_56px]";

/** A single-select list: arrow keys move, Enter or Space picks, a click picks. */
function NamespaceList({
  items, selectedHost, labelledBy, onPick,
}: {
  items: DiscoveredNamespace[];
  selectedHost: string;
  labelledBy: string;
  onPick: (ns: DiscoveredNamespace) => void;
}) {
  const uid = useId();
  const [active, setActive] = useState(0);
  const [focused, setFocused] = useState(false);
  const last = items.length - 1;
  const at = Math.min(active, last);
  const optionId = (i: number) => `${uid}-opt-${i}`;

  // Keep the keyboard position visible in a list that scrolls.
  useEffect(() => {
    document.getElementById(`${uid}-opt-${at}`)?.scrollIntoView?.({ block: "nearest" });
  }, [at, uid]);

  const onKeyDown = (e: KeyboardEvent) => {
    const move = (i: number) => {
      e.preventDefault();
      setActive(Math.max(0, Math.min(last, i)));
    };
    if (e.key === "ArrowDown") move(at + 1);
    else if (e.key === "ArrowUp") move(at - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(last);
    else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault(); // Enter must not submit the form
      onPick(items[at]);
    }
  };

  // The row under the keyboard carries the 1px ring, and only for keyboard focus (`:focus-visible`), so a
  // click does not leave a ring behind; the list itself shows none.
  return (
    <div
      role="listbox"
      tabIndex={0}
      aria-labelledby={labelledBy}
      aria-activedescendant={focused ? optionId(at) : undefined}
      onKeyDown={onKeyDown}
      onFocus={() => {
        setFocused(true);
        const i = items.findIndex((n) => n.host === selectedHost);
        if (i >= 0) setActive(i);
      }}
      onBlur={() => setFocused(false)}
      className="group max-h-[216px] overflow-y-auto rounded-ctl border border-line bg-field outline-none"
    >
      {items.map((ns, i) => {
        const selected = ns.host === selectedHost;
        return (
          <div
            key={ns.resource_id}
            id={optionId(i)}
            role="option"
            aria-selected={selected}
            aria-label={label(ns)}
            onClick={() => {
              setActive(i);
              onPick(ns);
            }}
            // 26px is the comfortable row (DESIGN.md spacing.row-comfortable); `h-row` follows the compact density.
            className={cn(
              "grid h-[26px] cursor-default items-center gap-2 px-2",
              COLUMNS,
              selected ? "bg-selection" : "hover:bg-hover",
              i === at && "group-focus-visible:outline group-focus-visible:outline-1 group-focus-visible:-outline-offset-1 group-focus-visible:outline-fg",
            )}
          >
            <span className="truncate text-12 text-fg">{ns.name}</span>
            <span className="truncate text-11 text-muted">{ns.subscription_name}</span>
            <span className="truncate text-right text-11 tabular-nums text-muted">{ns.location}</span>
            <span className="truncate text-right text-11 tabular-nums text-muted">{ns.tier}</span>
          </div>
        );
      })}
    </div>
  );
}

const tenantName = (t: TenantView | undefined, id: string) => t?.display_name ?? t?.default_domain ?? id;

/** A tenant that asks for multi-factor sign-in: sign in to it here, then list it again. This is not a
 *  failure, so it is a neutral message, not an error. */
function TenantSignIn({ name, tenantId, message, onSignedIn }: {
  name: string;
  tenantId: string;
  message: string;
  onSignedIn: () => void;
}) {
  const flow = useSignInFlow(onSignedIn);
  if (flow.pending) return <SignInProgress flow={flow} />;
  return (
    <>
      <EmptyState
        className="px-0 py-1"
        title={`${name} needs its own sign-in`}
        body={sentence(message)}
        action={
          <>
            <Button onClick={() => flow.start(false, tenantId)}>Sign in to {name}</Button>
            <Button variant="quiet" onClick={() => flow.start(true, tenantId)}>Use a device code instead</Button>
          </>
        }
      />
      {flow.failure && <Problem title="Sign-in failed" body={sentence(flow.failure.message)} />}
      <OtherAccountNotice flow={flow} name={name} />
    </>
  );
}

/** The tenant and the Service Bus namespaces the signed-in account can see, to pick one instead of typing it.
 *  It stays mounted (`hidden`) while a pick is shown as a summary, so the tenant and list are as they were. */
export function NamespacePicker({
  selectedHost, onPick, onTenant, hidden,
}: {
  /** The host the form holds now, to mark its row. */
  selectedHost: string;
  /** `tenantId` is the tenant to save the namespace under (`null`: the home tenant); `tenantLabel` names it;
   *  `signInUpn` is the identity of the tenant's own sign-in when the tenant is listed with one. */
  onPick: (ns: DiscoveredNamespace, tenantId: string | null, tenantLabel: string, signInUpn: string | null) => void;
  /** The tenant being browsed changed (`null` is the home tenant). */
  onTenant: (tenantId: string | null) => void;
  hidden?: boolean;
}) {
  const labelId = useId();
  const [tenantId, setTenantId] = useState<string | null>(null); // null: the home tenant
  const [filter, setFilter] = useState("");
  const account = useAccount();
  const tenants = useTenants();
  const found = useDiscoveredNamespaces(tenantId);
  // The list marks the home tenant; if it does not, the one that is the sign-in's own tenant is home, else "Home tenant".
  const home = tenants.data?.find((t) => t.home) ?? tenants.data?.find((t) => t.tenant_id === account.data?.tenant_id);
  const homeLabel = home ? tenantLabel({ ...home, home: true }) : "Home tenant";
  const q = filter.trim().toLowerCase();
  const all = found.data?.namespaces;
  const shown = useMemo(() => (all ? all.filter((n) => matches(n, q)) : []), [all, q]);
  const browsed = tenantId === null ? undefined : tenants.data?.find((t) => t.tenant_id === tenantId);
  const browsedLabel = browsed ? tenantLabel(browsed) : homeLabel;

  return (
    <div hidden={hidden} className="flex flex-col gap-3">
      {tenants.isPending ? (
        <div className="flex flex-col gap-1">
          <span className="text-12 font-semibold text-muted">Tenant</span>
          <div role="status" aria-label="Loading tenants" className="flex h-6 items-center rounded-ctl border border-line px-2">
            <Skeleton className="w-[40%]" />
          </div>
        </div>
      ) : tenants.data ? (
        <Field label="Tenant" hint={browsed?.sign_in_upn ? `Listed with your own sign-in to this tenant, as ${browsed.sign_in_upn}.` : undefined}>
          {(ids) => (
            <Select
              value={tenantId ?? home?.tenant_id ?? HOME_VALUE}
              onValueChange={(v) => {
                const next = v === HOME_VALUE || v === home?.tenant_id ? null : v;
                setTenantId(next);
                onTenant(next);
                setFilter("");
              }}
            >
              <SelectTrigger {...ids}><SelectValue /></SelectTrigger>
              <SelectContent>
                {!home && <SelectItem value={HOME_VALUE}>Home tenant</SelectItem>}
                {tenants.data.map((t) => (
                  <SelectItem key={t.tenant_id} value={t.tenant_id}>{t.tenant_id === home?.tenant_id ? homeLabel : tenantLabel(t)}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </Field>
      ) : (
        <Problem
          title="Couldn't list your tenants"
          body={`${sentence((tenants.error as EngineError).message)} Namespaces in your home tenant are listed below.`}
          action={<Button size="sm" onClick={() => void tenants.refetch()}>Try again</Button>}
        />
      )}

      <div className="flex flex-col gap-1">
        <div className="flex items-center justify-between gap-2">
          <span id={labelId} className="text-12 font-semibold text-muted">Namespaces</span>
          {all && all.length > FILTER_THRESHOLD && (
            <Input
              aria-label="Filter namespaces"
              placeholder="Filter"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.preventDefault(); // filtering is not saving
              }}
              className="w-40"
            />
          )}
        </div>

        {found.isPending ? (
          <div role="status" aria-label="Loading namespaces" className="rounded-ctl border border-line">
            {["w-[56%]", "w-[44%]", "w-[62%]", "w-[38%]"].map((w) => (
              <div key={w} data-skeleton-row className="flex h-[26px] items-center gap-2 px-2">
                <Skeleton className={w} />
              </div>
            ))}
          </div>
        ) : found.isError ? (
          (found.error as EngineError).kind === "mfa_required" ? (
            <TenantSignIn
              key={tenantId}
              name={tenantName(browsed, tenantId ?? "")}
              tenantId={tenantId ?? ""}
              message={(found.error as EngineError).message}
              onSignedIn={() => void found.refetch()}
            />
          ) : (
            <Problem
              title="Couldn't list namespaces"
              body={sentence((found.error as EngineError).message)}
              action={<Button size="sm" onClick={() => void found.refetch()}>Try again</Button>}
            />
          )
        ) : found.data.namespaces.length === 0 ? (
          <EmptyState
            className="px-0 py-1"
            title="No Service Bus namespaces in this tenant"
            body="You need Reader access on a subscription to see its namespaces."
          />
        ) : shown.length === 0 ? (
          <p className="m-0 py-1 text-12 text-muted">No namespaces match “{filter.trim()}”.</p>
        ) : (
          <NamespaceList
            key={`${tenantId}:${q}`}
            items={shown}
            selectedHost={selectedHost}
            labelledBy={labelId}
            onPick={(ns) => onPick(ns, tenantId, browsedLabel, browsed?.sign_in_upn ?? null)}
          />
        )}

        {found.data && found.data.skipped.length > 0 && (
          <details className="group text-11 text-muted">
            <summary className="flex w-fit cursor-default select-none list-none items-center gap-1 rounded-ctl focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg focus-visible:outline-offset-1 [&::-webkit-details-marker]:hidden">
              <ChevronRight aria-hidden="true" className="size-3 shrink-0 group-open:rotate-90" />
              {count(found.data.skipped.length, "subscription", "subscriptions")} couldn't be listed
            </summary>
            <ul className="m-0 flex list-none flex-col gap-0.5 pl-4 pt-1">
              {found.data.skipped.map((s) => (
                <li key={s.subscription_id}>
                  <span className="text-fg">{s.subscription_name}</span>: {sentence(s.reason)}
                </li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </div>
  );
}
