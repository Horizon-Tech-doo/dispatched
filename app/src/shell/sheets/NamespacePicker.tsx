import { Check } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useDiscoveredNamespaces, useTenants, type DiscoveredNamespace, type EngineError, type TenantView } from "@/engine";
import { Button, EmptyState, Field, Input, Problem, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Skeleton, cn } from "@/ui";
import { sentence } from "./profileErrors";
import { SignInProgress, useSignInFlow } from "./SignInPanel";

/** More namespaces than this get a filter box. */
export const FILTER_THRESHOLD = 8;

const tenantLabel = (t: TenantView) => {
  const name = t.display_name ?? t.default_domain ?? t.tenant_id;
  return t.home ? `${name} (home)` : name;
};

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

const meta = (ns: DiscoveredNamespace) => [ns.subscription_name, ns.location, ns.tier].filter(Boolean).join(" · ");

const matches = (ns: DiscoveredNamespace, q: string) => `${ns.name} ${meta(ns)}`.toLowerCase().includes(q);

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
  const box = useRef<HTMLDivElement>(null);
  const last = items.length - 1;
  const at = Math.min(active, last);
  const optionId = (i: number) => `${uid}-opt-${i}`;

  // Keep the keyboard position visible in a list that scrolls.
  useEffect(() => {
    document.getElementById(optionId(at))?.scrollIntoView?.({ block: "nearest" });
    // optionId is derived from uid
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [at]);

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

  return (
    <div
      ref={box}
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
      className="max-h-[216px] overflow-y-auto rounded-ctl border border-line bg-field focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg focus-visible:-outline-offset-1"
    >
      {items.map((ns, i) => {
        const selected = ns.host === selectedHost;
        return (
          <div
            key={ns.resource_id}
            id={optionId(i)}
            role="option"
            aria-selected={selected}
            onClick={() => {
              setActive(i);
              onPick(ns);
            }}
            className={cn(
              "flex h-[26px] cursor-default items-center gap-2 px-2",
              selected ? "bg-selection" : "hover:bg-hover",
              focused && i === at && "outline outline-1 -outline-offset-1 outline-fg",
            )}
          >
            <span className="max-w-[60%] shrink-0 truncate text-12 text-fg">{ns.name}</span>
            <span className="flex min-w-0 flex-1 text-11 text-muted">
              <span className="min-w-0 truncate">{ns.subscription_name}</span>
              <span className="shrink-0 whitespace-pre">{[ns.location, ns.tier].filter(Boolean).map((x) => ` · ${x}`).join("")}</span>
            </span>
            {ns.local_auth_disabled && (
              <span className="shrink-0 text-11 text-muted" title="Shared access keys are turned off. Sign in with Entra ID.">
                Entra only
              </span>
            )}
            <Check aria-hidden="true" className={cn("size-3.5 shrink-0", selected ? "text-fg" : "invisible")} />
          </div>
        );
      })}
    </div>
  );
}

const tenantName = (t: TenantView | undefined, id: string) => t?.display_name ?? t?.default_domain ?? id;

/** A tenant that asks for multi-factor sign-in: sign in to it here, then list it again. */
function TenantSignIn({ name, tenantId, message, onSignedIn }: {
  name: string;
  tenantId: string;
  message: string;
  onSignedIn: () => void;
}) {
  const flow = useSignInFlow(onSignedIn);
  if (flow.pending) return <SignInProgress flow={flow} />;
  return (
    <Problem
      title={`${name} needs its own sign-in`}
      body={sentence(message)}
      detail={flow.failure?.message}
      action={
        <>
          <Button onClick={() => flow.start(false, tenantId)}>Sign in to {name}</Button>
          <Button variant="quiet" onClick={() => flow.start(true, tenantId)}>Use a device code instead</Button>
        </>
      }
    />
  );
}

/** The tenant and the Service Bus namespaces the signed-in account can see, to pick one instead of typing it.
 *  `onPick` gets the namespace and the tenant to save it under (`null` is the home tenant). */
export function NamespacePicker({
  selectedHost, onPick, onTenant,
}: {
  /** The host the form holds now, to mark its row. */
  selectedHost: string;
  onPick: (ns: DiscoveredNamespace, tenantId: string | null) => void;
  /** The tenant being browsed changed (`null` is the home tenant). */
  onTenant: (tenantId: string | null) => void;
}) {
  const labelId = useId();
  const [tenantId, setTenantId] = useState<string | null>(null); // null: the home tenant
  const [filter, setFilter] = useState("");
  // A new picker (another account) starts at the home tenant.
  useEffect(() => onTenant(null), [onTenant]);
  const tenants = useTenants();
  const found = useDiscoveredNamespaces(tenantId);
  const home = tenants.data?.find((t) => t.home);
  const q = filter.trim().toLowerCase();
  const all = found.data?.namespaces;
  const shown = useMemo(() => (all ? all.filter((n) => matches(n, q)) : []), [all, q]);

  return (
    <div className="flex flex-col gap-3">
      {tenants.isPending ? (
        <div className="flex flex-col gap-1">
          <span className="text-12 font-semibold text-muted">Tenant</span>
          <div role="status" aria-label="Loading tenants" className="flex h-6 items-center rounded-ctl border border-line px-2">
            <Skeleton className="w-[40%]" />
          </div>
        </div>
      ) : tenants.data ? (
        <Field label="Tenant">
          {(ids) => (
            <Select
              value={tenantId ?? home?.tenant_id ?? tenants.data[0]?.tenant_id ?? ""}
              onValueChange={(v) => {
                const next = v === home?.tenant_id ? null : v;
                setTenantId(next);
                onTenant(next);
                setFilter("");
              }}
            >
              <SelectTrigger {...ids}><SelectValue /></SelectTrigger>
              <SelectContent>
                {tenants.data.map((t) => (
                  <SelectItem key={t.tenant_id} value={t.tenant_id}>{tenantLabel(t)}</SelectItem>
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
              name={tenantName(tenants.data?.find((t) => t.tenant_id === tenantId), tenantId ?? "")}
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
            title="No Service Bus namespaces in this tenant."
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
            onPick={(ns) => onPick(ns, tenantId)}
          />
        )}

        {found.data && found.data.skipped.length > 0 && (
          <details className="text-11 text-muted">
            <summary className="w-fit cursor-default select-none rounded-ctl focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg focus-visible:outline-offset-1">
              {count(found.data.skipped.length, "subscription", "subscriptions")} couldn't be listed
            </summary>
            <ul className="m-0 flex list-none flex-col gap-0.5 pl-0 pt-1">
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
