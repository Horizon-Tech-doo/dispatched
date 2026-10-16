import { ChevronRight, CircleAlert, CircleCheck, CircleMinus } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import {
  useAccount, useDeleteProfile, useProfiles, useSaveProfile, useTenants, useTestDraft,
  type Check, type ConnectionReport, type DiscoveredNamespace, type EngineError, type Profile, type ProfileDraft, type Transport,
} from "@/engine";
import {
  Button, Field, Input, LabelSwatch, Problem, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, Textarea,
  cn, labelColors,
} from "@/ui";
import { useShell } from "../context";
import { checkAdvice, placeError, sentence, skippedReason, type Errors } from "./profileErrors";
import { NamespacePicker } from "./NamespacePicker";
import { SignInPanel, TenantSignInAction, useSigningIn } from "./SignInPanel";

type AuthKind = "connection-string" | "entra";

interface Form {
  name: string;
  auth: AuthKind;
  connectionString: string;
  replaceSecret: boolean;
  namespace: string;
  tenant: string;
  production: boolean;
  color: string;
  transport: Transport;
  proxy: string;
  pins: string;
}

const PUBLIC_SUFFIX = ".servicebus.windows.net";

/** What the Namespace field shows for a saved host: the short name only when adding the public suffix back
 *  gives the same host (`sb-orders`); a private link or another cloud keeps its full host. */
export function namespaceField(host: string): string {
  const short = host.endsWith(PUBLIC_SUFFIX) ? host.slice(0, -PUBLIC_SUFFIX.length) : null;
  return short && !short.includes(".") ? short : host;
}

/** The tenant an Entra profile lists and connects in: its stored tenant, or, when it is bound to a tenant's own
 *  sign-in (stored without a tenant, since that is the sign-in's home), that sign-in's tenant. `null`: home. */
export function profileTenant(p: Profile): string | null {
  if (p.auth.kind !== "entra") return null;
  return p.auth.tenant ?? (p.bound_sign_in?.own_tenant ? p.bound_sign_in.tenant_id : null);
}

function initial(p: Profile | undefined, auth: AuthKind | undefined): Form {
  return {
    name: p?.name ?? "",
    auth: p ? (p.auth.kind === "entra" ? "entra" : "connection-string") : (auth ?? "connection-string"),
    connectionString: "",
    replaceSecret: false,
    namespace: p ? namespaceField(p.host) : "",
    tenant: p?.auth.kind === "entra" ? profileTenant(p) ?? "" : "",
    production: p?.production ?? false,
    color: p?.color ?? "none",
    transport: p?.transport ?? "amqp",
    proxy: p?.proxy ?? "",
    pins: (p?.pinned_entities ?? []).join("\n"),
  };
}

function CheckLine({ label, c }: { label: "Management" | "Messaging"; c: Check }) {
  if (c.status === "failed")
    return (
      <li>
        <Problem role="alert" title={`${label} failed`} body={checkAdvice(label, c.kind)} detail={c.message} />
      </li>
    );
  const skipped = c.status === "skipped";
  return (
    <li className="flex items-start gap-1.5 text-12">
      {skipped ? (
        <CircleMinus aria-hidden="true" className="mt-px size-3.5 shrink-0 text-icon" />
      ) : (
        <CircleCheck aria-hidden="true" className="mt-px size-3.5 shrink-0 text-icon" />
      )}
      <span className="flex min-w-0 flex-col">
        <span className="font-semibold">{skipped ? `${label} not tested` : `${label} works`}</span>
        <span className="text-muted">{skipped ? skippedReason(c.reason) : c.detail}</span>
      </span>
    </li>
  );
}

const TRANSPORT_NAME: Record<Transport, string> = { amqp: "AMQP", websockets: "AMQP over WebSockets" };

/** Add or edit a connection. An edit waits for the saved list so it never starts from an empty form. */
export function ProfileSheet({ profileId, initialAuth }: { profileId: string | null; initialAuth?: AuthKind }) {
  const { closeSheet } = useShell();
  const profiles = useProfiles();
  const profile = profiles.data?.find((p) => p.id === profileId);
  const gone = profileId !== null && !profiles.isPending && !profile;
  useEffect(() => {
    if (gone) closeSheet();
  }, [gone, closeSheet]);
  if (profileId !== null && !profile) return null;
  return <ProfileForm profile={profile} initialAuth={initialAuth} />;
}

function ProfileForm({ profile, initialAuth }: { profile: Profile | undefined; initialAuth?: AuthKind }) {
  const { closeSheet, conn } = useShell();
  const editing = !!profile;
  const formId = useId();
  const [f, setF] = useState<Form>(() => initial(profile, initialAuth));
  const [errors, setErrors] = useState<Errors>({});
  const [confirmDelete, setConfirmDelete] = useState(false);
  const save = useSaveProfile();
  const del = useDeleteProfile();
  const test = useTestDraft();
  // The result lives here, not in the mutation: the mutation is reset as soon as it settles, so the draft (and a
  // typed connection string in it) does not stay in its `variables`.
  const [tested, setTested] = useState<{ report: ConnectionReport } | { error: EngineError } | null>(null);
  // Bumped by every edit: a result that arrives for an older form is dropped.
  const formVersion = useRef(0);
  const signingIn = useSigningIn();
  const resultBox = useRef<HTMLDivElement>(null);
  // A result that has just arrived is brought into view, as little as it takes.
  useEffect(() => {
    if (tested) resultBox.current?.scrollIntoView({ block: "nearest" });
  }, [tested]);
  const account = useAccount();
  // Signed in, a new Entra connection picks its namespace from a list; the typed fields stay one click away.
  const picker = !editing && f.auth === "entra" && !!account.data;
  // The typed Namespace and Tenant fields: always there without a picker, else behind a toggle.
  const [manual, setManual] = useState(false);
  const showManual = !picker || manual;
  // What came from the list (not from typing), and whether the list shows instead of the summary of it.
  const [picked, setPicked] = useState<{ ns: DiscoveredNamespace; tenantId: string | null; tenantLabel: string; signInUpn: string | null } | null>(null);
  const [choosing, setChoosing] = useState(true);
  const [browsedTenant, setBrowsedTenant] = useState<string | null>(null);
  // Whether the Tenant ID holds text the user typed; until then it follows the tenant being browsed.
  const [tenantTyped, setTenantTyped] = useState(false);
  const tenantTypedNow = useRef(false);
  tenantTypedNow.current = tenantTyped;
  // Where focus goes once the summary or the list has rendered: a pick hides the list, Change hides the summary.
  const focusNext = useRef<"change" | "list" | null>(null);
  const changeButton = useRef<HTMLButtonElement>(null);
  const pickerBox = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const target = focusNext.current;
    focusNext.current = null;
    if (target === "change") changeButton.current?.focus();
    // The list puts its keyboard position on the picked row when it gains focus.
    else if (target === "list") pickerBox.current?.querySelector<HTMLElement>('[role="listbox"]')?.focus();
  }, [picked, choosing]);
  useEffect(() => {
    formVersion.current++;
    setTested(null); // a result never describes settings the form no longer has
  }, [f]);
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setF((x) => ({ ...x, [k]: v }));
  // Typing in the Namespace or Tenant field makes the values typed ones: they are no longer "what was picked".
  const typed = (k: "namespace" | "tenant", v: string) => {
    set(k, v);
    if (k === "tenant") setTenantTyped(true);
    setManual(true);
    setPicked(null);
    setChoosing(true);
  };
  /** Forget a pick (never typed text): the Tenant select and the saved tenant must agree. */
  const dropPick = () => {
    setF((x) => ({ ...x, namespace: "", tenant: "" }));
    setPicked(null);
    setChoosing(true);
  };
  const browse = (tenantId: string | null) => {
    setBrowsedTenant(tenantId);
    if (picked && picked.tenantId !== tenantId) dropPick();
    // An open Tenant ID field the user has not typed in follows the browsed tenant (empty for home).
    if (manual && !tenantTyped && !(picked && picked.tenantId === tenantId)) set("tenant", tenantId ?? "");
  };
  // Another account starts a new pick.
  const accountId = account.data?.id;
  const lastAccount = useRef(accountId);
  const hasPick = useRef(false);
  hasPick.current = picked !== null;
  useEffect(() => {
    if (lastAccount.current === accountId) return;
    lastAccount.current = accountId;
    // A test result is about the account it ran with; one still running is dropped too.
    formVersion.current++;
    setTested(null);
    setBrowsedTenant(null);
    setChoosing(true);
    if (hasPick.current) dropPick();
    // A Tenant ID taken from the old account's tenants is not this account's: clear it unless it was typed.
    // (Editing keeps the profile's own tenant.)
    else if (!editing && !tenantTypedNow.current) setF((x) => ({ ...x, tenant: "" }));
  }, [accountId, editing]);
  const hasProxy = f.proxy.trim() !== "";
  const transport: Transport = hasProxy ? "websockets" : f.transport; // a proxy carries AMQP over WebSockets
  const secretMissing =
    editing && profile.auth.kind === "connection-string" && conn.status === "failed" && conn.profileId === profile.id &&
    conn.error?.kind === "invalid" && /no stored connection string/.test(conn.error.message);
  // The sign-in an edited Entra profile uses, as the engine reports it: warn only when it no longer works or is
  // someone else's. A guest-tenant profile on the current account's own tenant sign-in is fine.
  const bound = editing && profile.auth.kind === "entra" ? profile.bound_sign_in : null;
  const signInIssue =
    !editing || profile.auth.kind !== "entra" || account.data == null
      ? null
      : bound == null
        ? `The account this connection used is no longer signed in on this computer. Save to use ${account.data.user_principal}.`
        : !bound.live
          ? `The sign-in this connection uses (${bound.user_principal}) has expired. Sign in again to use it.`
          : !bound.same_account
            ? `This connection uses a different account, ${bound.user_principal}. Sign in with that account to use it.`
            : null;
  // Signing in again with the same account flips the bound sign-in's `live` flag: a result from before it is stale.
  const boundLive = bound?.live;
  const lastLive = useRef(boundLive);
  useEffect(() => {
    if (lastLive.current === boundLive) return;
    lastLive.current = boundLive;
    formVersion.current++;
    setTested(null);
  }, [boundLive]);
  // An edited Entra profile whose sign-in record is gone: a test has no sign-in to use until it is saved.
  const signInGone = editing && profile.auth.kind === "entra" && profile.bound_sign_in == null;
  // The tenant an edited Entra profile lives in, and its name when the tenant list has it.
  const savedTenant = editing ? profileTenant(profile) : null;
  const tenants = useTenants(savedTenant !== null);
  const savedTenantName = savedTenant
    ? (tenants.data?.find((t) => t.tenant_id.toLowerCase() === savedTenant.toLowerCase())?.display_name ?? savedTenant)
    : null;

  function validate(): Errors {
    const e: Errors = {};
    if (!f.name.trim()) e.name = "Enter a name for this connection.";
    if (f.auth === "connection-string" && (!editing || f.replaceSecret) && !f.connectionString.trim())
      e.connectionString = "Paste the namespace's connection string.";
    if (f.auth === "entra" && !f.namespace.trim())
      e.namespace = picker ? "Pick a namespace from the list, or enter one manually." : "Enter the namespace name, like sb-orders, or its full host.";
    if (f.auth === "entra" && !editing && !account.data) e.signIn = "Sign in with Entra ID first.";
    return e;
  }

  function pick(ns: DiscoveredNamespace, tenantId: string | null, tenantLabel: string, signInUpn: string | null) {
    setF((x) => ({ ...x, namespace: ns.host, tenant: tenantId ?? "", name: x.name.trim() ? x.name : ns.name }));
    setErrors((e) => ({ ...e, namespace: undefined, name: undefined }));
    setPicked({ ns, tenantId, tenantLabel, signInUpn });
    setTenantTyped(false); // the tenant is the picked one now
    setChoosing(false);
    focusNext.current = "change";
  }

  /** What the form says now: Save and Test send the same draft. A stored connection string stays `null`. */
  function toDraft(): ProfileDraft {
    return {
      id: profile?.id ?? null,
      name: f.name.trim(),
      auth:
        f.auth === "entra"
          ? { kind: "entra", namespace: f.namespace.trim(), tenant: f.tenant.trim() || null }
          : { kind: "connection-string", connection_string: f.replaceSecret || !editing ? f.connectionString.trim() : null },
      production: f.production,
      color: f.color === "none" ? null : f.color,
      transport,
      proxy: f.proxy.trim() || null,
      pinned_entities: f.pins.split("\n").map((s) => s.trim()).filter(Boolean),
    };
  }

  function runTest() {
    // As Save: a form that can't make a draft says why, under the fields.
    const e = validate();
    setErrors(e);
    if (Object.keys(e).length) return;
    const version = formVersion.current;
    const settle = (r: { report: ConnectionReport } | { error: EngineError }) => {
      if (version === formVersion.current) setTested(r);
    };
    test.mutate(toDraft(), {
      onSuccess: (report) => settle({ report }),
      onError: (error) => settle({ error: error as EngineError }),
      onSettled: () => test.reset(),
    });
  }

  async function onSave() {
    const e = validate();
    setErrors(e);
    if (Object.keys(e).length) return;
    try {
      const result = await save.mutateAsync(toDraft());
      setF((x) => ({ ...x, connectionString: "" }));
      save.reset(); // drops the draft, and the connection string in it, from the mutation state
      closeSheet();
      if (!editing || result.id === conn.profileId) void conn.connect(result.id);
    } catch (err) {
      setErrors(placeError(err as EngineError));
    }
  }

  async function onDelete() {
    if (!profile) return;
    try {
      // Leave the connection first, so a failed delete is visible and the sheet closes on its own terms.
      if (conn.profileId === profile.id) await conn.disconnect();
      await del.mutateAsync(profile.id);
      closeSheet();
    } catch (err) {
      setConfirmDelete(false);
      setErrors({ general: sentence((err as EngineError).message) });
    }
  }

  const title = editing ? `Edit ${profile.name}` : "Add connection";
  return (
    <Sheet
      open
      onOpenChange={(o) => !o && closeSheet()}
      title={title}
      description={editing ? undefined : "Saved on this computer. Secrets go to the system keychain."}
      initialFocus="field"
      footer={
        <div className="flex w-full flex-col gap-2">
          {confirmDelete && profile ? (
            <>
              <p className="m-0 text-12">
                Delete {profile.name}?{" "}
                {profile.auth.kind === "connection-string" ? "Its saved connection string is removed from the keychain too." : "Your Entra sign-in stays."}
              </p>
              <div className="flex justify-end gap-2">
                <Button variant="quiet" onClick={() => setConfirmDelete(false)}>Keep</Button>
                <Button variant="danger" onClick={() => void onDelete()} disabled={del.isPending}>Delete {profile.name}</Button>
              </div>
            </>
          ) : (
            <>
              {signingIn && <p id={`${formId}-test-hint`} className="m-0 text-11 text-muted">Finish signing in to test.</p>}
              <div className="flex gap-2">
                {editing && <Button variant="quiet" onClick={() => setConfirmDelete(true)}>Delete…</Button>}
                <Button onClick={runTest} disabled={test.isPending || signingIn} aria-describedby={signingIn ? `${formId}-test-hint` : undefined}>
                  {test.isPending ? "Testing…" : "Test connection"}
                </Button>
                <div className="flex-1" />
                <Button variant="quiet" onClick={closeSheet}>Cancel</Button>
                <Button variant="primary" type="submit" form={formId} disabled={save.isPending}>{save.isPending ? "Saving…" : "Save"}</Button>
              </div>
            </>
          )}
        </div>
      }
    >
      <form
        id={formId}
        className="flex flex-col gap-4"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          void onSave();
        }}
      >
        {errors.general && (
          <p className="m-0 flex items-start gap-1.5 text-12">
            <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
            <span>{errors.general}</span>
          </p>
        )}
        <Field label="Name" error={errors.name}>
          {(ids) => <Input {...ids} value={f.name} onChange={(e) => set("name", e.target.value)} placeholder="Logistics production" />}
        </Field>

        <div className="flex flex-col gap-1">
          <span className="text-12 font-semibold text-muted">Sign-in method</span>
          {editing ? (
            <span className="text-12 text-fg">{f.auth === "entra" ? "Entra ID" : "Connection string"}</span>
          ) : (
            <div>
              <Segmented
                label="Sign-in method"
                value={f.auth}
                onChange={(v) => set("auth", v)}
                options={[{ value: "connection-string", label: "Connection string" }, { value: "entra", label: "Entra ID" }]}
              />
            </div>
          )}
        </div>

        {f.auth === "connection-string" ? (
          editing && !f.replaceSecret ? (
            <div className="flex flex-col gap-1">
              <span className="text-12 font-semibold text-muted">Connection string</span>
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 flex-col">
                  <span className="truncate font-mono text-11 text-fg">{profile.host}</span>
                  <span className="text-11 text-muted">
                    {secretMissing ? "No connection string is stored. Paste it again." : "Stored in the system keychain."}
                  </span>
                </div>
                <Button size="sm" onClick={() => set("replaceSecret", true)}>{secretMissing ? "Paste" : "Replace"}</Button>
              </div>
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              <Field
                label="Connection string"
                hint="From Shared access policies in the Azure Portal. It is stored in the system keychain and never shown again."
                error={errors.connectionString}
              >
                {(ids) => (
                  <Input
                    {...ids}
                    type="password"
                    autoComplete="new-password"
                    mono
                    value={f.connectionString}
                    onChange={(e) => set("connectionString", e.target.value)}
                    placeholder="Endpoint=sb://…"
                  />
                )}
              </Field>
              {editing && (
                <div>
                  <Button size="sm" variant="quiet" onClick={() => setF((x) => ({ ...x, replaceSecret: false, connectionString: "" }))}>
                    Keep the stored one
                  </Button>
                </div>
              )}
            </div>
          )
        ) : (
          <>
            <div className="flex flex-col gap-1.5">
              <SignInPanel />
              {errors.signIn && (
                <p className="m-0 flex items-start gap-1.5 text-12">
                  <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
                  <span>{errors.signIn}</span>
                </p>
              )}
              {signInIssue && <p className="m-0 text-11 text-muted">{signInIssue}</p>}
              {savedTenant && savedTenantName && account.data && (
                <TenantSignInAction tenantId={savedTenant} name={savedTenantName} />
              )}
            </div>
            {picker && (
              <>
                <div ref={pickerBox} className="contents">
                  <NamespacePicker
                    key={accountId}
                    hidden={!!picked && !choosing}
                    selectedHost={f.namespace.trim().toLowerCase()}
                    onPick={pick}
                    onTenant={browse}
                  />
                </div>
                {picked && !choosing && (
                  <div role="group" aria-label="Selected namespace" className="flex flex-col gap-1">
                    <div className="flex h-[26px] items-center gap-2 rounded-ctl border border-line bg-field pl-2 pr-0.5">
                      <span className="min-w-0 flex-1 truncate font-mono text-11 text-fg" title={picked.ns.host} data-selectable>{picked.ns.host}</span>
                      <span className="shrink-0 text-11 text-muted">{picked.tenantLabel}</span>
                      <Button
                        ref={changeButton}
                        size="sm"
                        variant="quiet"
                        onClick={() => {
                          focusNext.current = "list";
                          setChoosing(true);
                        }}
                      >
                        Change
                      </Button>
                    </div>
                    {picked.signInUpn && <p className="m-0 text-11 text-muted">Listed and connected with your own sign-in to {picked.tenantLabel}, as {picked.signInUpn}.</p>}
                    {picked.ns.local_auth_disabled && <p className="m-0 text-11 text-muted">Shared access keys are off for this namespace. Entra ID only.</p>}
                  </div>
                )}
              </>
            )}
            {showManual && (
              <>
                <Field label="Namespace" hint="The namespace name, like sb-orders, or its full host." error={errors.namespace}>
                  {(ids) => <Input {...ids} mono value={f.namespace} onChange={(e) => typed("namespace", e.target.value)} placeholder="sb-orders" />}
                </Field>
                <Field label={picker ? "Tenant ID" : "Tenant"} hint="Leave empty for your home tenant. Otherwise the ID of the namespace's tenant.">
                  {(ids) => <Input {...ids} mono value={f.tenant} onChange={(e) => typed("tenant", e.target.value)} placeholder="Home tenant" />}
                </Field>
              </>
            )}
            {picker && (
              <div className="flex flex-col items-start gap-1">
                {!showManual && errors.namespace && (
                  <p className="m-0 flex items-start gap-1.5 text-12">
                    <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
                    <span>{errors.namespace}</span>
                  </p>
                )}
                <Button
                  size="sm"
                  variant="quiet"
                  className="-ml-1.5"
                  aria-expanded={manual}
                  onClick={() => {
                    if (!manual) {
                      // The tenant being browsed is the one a typed namespace most likely belongs to.
                      setF((x) => ({ ...x, tenant: x.tenant || (browsedTenant ?? "") }));
                    }
                    setManual(!manual);
                  }}
                >
                  <ChevronRight aria-hidden="true" className={cn("size-3!", manual && "rotate-90")} />
                  Enter a namespace manually
                </Button>
              </div>
            )}
          </>
        )}

        <label className="flex items-start gap-2 text-12">
          <input
            type="checkbox"
            className="mt-0.5 size-3.5 accent-[var(--c-fg)]"
            checked={f.production}
            onChange={(e) => set("production", e.target.checked)}
          />
          <span>
            <span className="font-semibold">Production</span>
            <span className="block text-muted">Marked PROD everywhere. Destructive actions will ask you to type the entity name.</span>
          </span>
        </label>

        <div className="grid grid-cols-2 gap-3">
          <div className="flex flex-col gap-1">
            <span className="text-12 font-semibold text-muted">Label</span>
            <Select value={f.color} onValueChange={(v) => set("color", v)}>
              <SelectTrigger aria-label="Label"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                {Object.keys(labelColors).map((c) => (
                  <SelectItem key={c} value={c}>
                    <span className="flex items-center gap-2"><LabelSwatch color={c} />{c[0].toUpperCase() + c.slice(1)}</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-12 font-semibold text-muted">Transport</span>
            <Select value={transport} onValueChange={(v) => set("transport", v as Transport)} disabled={hasProxy}>
              <SelectTrigger aria-label="Transport" aria-describedby={hasProxy ? `${formId}-transport-hint` : undefined}><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="amqp">AMQP</SelectItem>
                <SelectItem value="websockets">AMQP over WebSockets</SelectItem>
              </SelectContent>
            </Select>
            {hasProxy && <p id={`${formId}-transport-hint`} className="m-0 text-11 text-muted">A proxy needs AMQP over WebSockets.</p>}
          </div>
        </div>

        <Field label="Proxy" hint="Format http://host:port. Leave empty to use HTTPS_PROXY from the environment." error={errors.proxy}>
          {(ids) => <Input {...ids} mono value={f.proxy} onChange={(e) => set("proxy", e.target.value)} placeholder="http://proxy.corp:8080" />}
        </Field>
        <Field
          label="Pinned entities"
          hint="One per line, like orders or events/subscriptions/audit. Shown when this identity can't list the namespace."
          error={errors.pins}
        >
          {(ids) => <Textarea {...ids} mono value={f.pins} onChange={(e) => set("pins", e.target.value)} />}
        </Field>

        {/* Always there, so a result that lands in it is announced; empty it takes no room (it cancels the form's gap). */}
        <div ref={resultBox} role="status" className={tested ? "flex flex-col gap-2 border-t border-line pt-3" : "-mt-4"}>
          {tested &&
            ("report" in tested ? (
              <>
                <p className="m-0 text-11 text-muted">
                  Tested <span className="font-mono" data-selectable>{tested.report.host}</span> as {tested.report.auth}, {TRANSPORT_NAME[tested.report.transport]}.
                </p>
                <ul aria-label="Connection test" className="m-0 flex list-none flex-col gap-2 p-0">
                  <CheckLine label="Management" c={tested.report.management} />
                  <CheckLine label="Messaging" c={tested.report.messaging} />
                </ul>
              </>
            ) : (
              <Problem
                role="alert"
                title="Couldn't test the connection"
                body={
                  <>
                    {sentence(tested.error.message)}
                    {signInGone && account.data != null && tested.error.kind === "not_signed_in" && " Save the connection to use your current sign-in, then test again."}
                  </>
                }
              />
            ))}
        </div>
      </form>
    </Sheet>
  );
}
