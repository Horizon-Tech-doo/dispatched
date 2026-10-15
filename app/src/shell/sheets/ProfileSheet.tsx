import { CircleAlert, CircleCheck } from "lucide-react";
import { useEffect, useId, useState } from "react";
import {
  useAccount, useDeleteProfile, useProfiles, useSaveProfile, useTestProfile,
  type Check, type EngineError, type Profile, type ProfileDraft, type Transport,
} from "@/engine";
import {
  Button, Field, Input, LabelSwatch, Segmented, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, Textarea,
  labelColors,
} from "@/ui";
import { useShell } from "../context";
import { placeError, sentence, type Errors } from "./profileErrors";
import { SignInPanel } from "./SignInPanel";

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

function initial(p: Profile | undefined, auth: AuthKind | undefined): Form {
  return {
    name: p?.name ?? "",
    auth: p ? (p.auth.kind === "entra" ? "entra" : "connection-string") : (auth ?? "connection-string"),
    connectionString: "",
    replaceSecret: false,
    namespace: p ? namespaceField(p.host) : "",
    tenant: p?.auth.kind === "entra" ? (p.auth.tenant ?? "") : "",
    production: p?.production ?? false,
    color: p?.color ?? "none",
    transport: p?.transport ?? "amqp",
    proxy: p?.proxy ?? "",
    pins: (p?.pinned_entities ?? []).join("\n"),
  };
}

function CheckLine({ label, c }: { label: string; c: Check }) {
  const headline = c.status === "ok" ? `${label} works` : c.status === "failed" ? `${label} failed: ${sentence(c.message)}` : `${label} not tested`;
  const detail = c.status === "ok" ? c.detail : c.status === "skipped" ? sentence(c.reason) : null;
  return (
    <li className="flex items-start gap-1.5 text-12">
      {c.status === "failed" ? (
        <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
      ) : (
        <CircleCheck aria-hidden="true" className="mt-px size-3.5 shrink-0 text-icon" />
      )}
      <span className="flex min-w-0 flex-col">
        <span className="font-semibold">{headline}</span>
        {detail && <span className="text-muted">{detail}</span>}
      </span>
    </li>
  );
}

/** The fields a test would not see: anything that differs from the saved profile. */
const changed = (a: Form, b: Form) => (Object.keys(a) as (keyof Form)[]).some((k) => k !== "replaceSecret" && a[k] !== b[k]);

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
  const [saved] = useState<Form>(() => initial(profile, initialAuth));
  const [f, setF] = useState<Form>(saved);
  const [errors, setErrors] = useState<Errors>({});
  const [confirmDelete, setConfirmDelete] = useState(false);
  const save = useSaveProfile();
  const del = useDeleteProfile();
  const test = useTestProfile();
  const account = useAccount();
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setF((x) => ({ ...x, [k]: v }));
  const dirty = editing && changed(f, saved);
  const hasProxy = f.proxy.trim() !== "";
  const transport: Transport = hasProxy ? "websockets" : f.transport; // a proxy carries AMQP over WebSockets
  const secretMissing =
    editing && profile.auth.kind === "connection-string" && conn.status === "failed" && conn.profileId === profile.id &&
    conn.error?.kind === "invalid" && /no stored connection string/.test(conn.error.message);
  const otherAccount =
    editing && profile.auth.kind === "entra" && account.data != null && account.data.id !== profile.auth.sign_in_id;

  function validate(): Errors {
    const e: Errors = {};
    if (!f.name.trim()) e.name = "Enter a name for this connection.";
    if (f.auth === "connection-string" && (!editing || f.replaceSecret) && !f.connectionString.trim())
      e.connectionString = "Paste the namespace's connection string.";
    if (f.auth === "entra" && !f.namespace.trim()) e.namespace = "Enter the namespace name, like sb-orders, or its full host.";
    if (f.auth === "entra" && !editing && !account.data) e.signIn = "Sign in with Entra ID first.";
    return e;
  }

  async function onSave() {
    const e = validate();
    setErrors(e);
    if (Object.keys(e).length) return;
    const draft: ProfileDraft = {
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
    try {
      const result = await save.mutateAsync(draft);
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
              {dirty && <p id={`${formId}-test-hint`} className="m-0 text-11 text-muted">Save to test these changes.</p>}
              <div className="flex gap-2">
                {editing && <Button variant="quiet" onClick={() => setConfirmDelete(true)}>Delete…</Button>}
                {editing && (
                  <Button
                    onClick={() => test.mutate(profile.id)}
                    disabled={test.isPending || dirty}
                    aria-describedby={dirty ? `${formId}-test-hint` : undefined}
                  >
                    {test.isPending ? "Testing…" : "Test connection"}
                  </Button>
                )}
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
              {otherAccount && (
                <p className="m-0 text-11 text-muted">This connection uses a different account. Sign in with that account to use it.</p>
              )}
            </div>
            <Field label="Namespace" hint="The namespace name, like sb-orders, or its full host." error={errors.namespace}>
              {(ids) => <Input {...ids} value={f.namespace} onChange={(e) => set("namespace", e.target.value)} placeholder="sb-orders" />}
            </Field>
            <Field label="Tenant" hint="Only if the namespace is in another tenant than your account's home tenant.">
              {(ids) => <Input {...ids} value={f.tenant} onChange={(e) => set("tenant", e.target.value)} placeholder="contoso.onmicrosoft.com" />}
            </Field>
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

        {test.data && (
          <div className="flex flex-col gap-1.5 border-t border-line pt-3">
            <p className="m-0 text-11 text-muted">Tested with saved settings.</p>
            <ul aria-label="Connection test" className="m-0 flex list-none flex-col gap-1 p-0">
              <CheckLine label="Management" c={test.data.management} />
              <CheckLine label="Messaging" c={test.data.messaging} />
            </ul>
          </div>
        )}
        {test.error && (
          <p className="m-0 flex items-start gap-1.5 border-t border-line pt-3 text-12">
            <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
            <span>{sentence(test.error.message)}</span>
          </p>
        )}
      </form>
    </Sheet>
  );
}
