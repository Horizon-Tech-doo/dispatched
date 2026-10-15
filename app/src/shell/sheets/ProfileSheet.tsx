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
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";
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

type Errors = Partial<Record<"name" | "connectionString" | "namespace" | "proxy" | "pins" | "general", string>>;

function initial(p: Profile | undefined, auth: AuthKind | undefined): Form {
  return {
    name: p?.name ?? "",
    auth: p ? (p.auth.kind === "entra" ? "entra" : "connection-string") : (auth ?? "connection-string"),
    connectionString: "",
    replaceSecret: false,
    namespace: p ? shortHost(p.host) : "",
    tenant: p?.auth.kind === "entra" ? (p.auth.tenant ?? "") : "",
    production: p?.production ?? false,
    color: p?.color ?? "none",
    transport: p?.transport ?? "amqp",
    proxy: p?.proxy ?? "",
    pins: (p?.pinned_entities ?? []).join("\n"),
  };
}

const sentence = (m: string) => m.charAt(0).toUpperCase() + m.slice(1);

/** Put a server error under the field it is about. */
function placeError(e: EngineError): Errors {
  const m = sentence(e.message);
  if (e.kind === "invalid") {
    if (/already exists|needs a name/i.test(m)) return { name: m };
    if (/connection string|Endpoint=|SharedAccess/i.test(m)) return { connectionString: m };
    if (/namespace/i.test(m)) return { namespace: m };
    if (/proxy/i.test(m)) return { proxy: m };
    if (/pinned|entity/i.test(m)) return { pins: m };
  }
  return { general: m };
}

function CheckLine({ label, c }: { label: string; c: Check }) {
  return (
    <li className="flex items-start gap-1.5 text-12">
      {c.status === "failed" ? (
        <CircleAlert aria-hidden="true" className="mt-px size-3.5 text-accent" />
      ) : (
        <CircleCheck aria-hidden="true" className="mt-px size-3.5 text-icon" />
      )}
      <span>
        <span className="font-semibold">{label}: </span>
        {c.status === "ok" ? c.detail : c.status === "failed" ? c.message : <span className="text-muted">{c.reason}</span>}
      </span>
    </li>
  );
}

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
  const test = useTestProfile();
  const account = useAccount();
  const set = <K extends keyof Form>(k: K, v: Form[K]) => setF((x) => ({ ...x, [k]: v }));

  function validate(): Errors {
    const e: Errors = {};
    if (!f.name.trim()) e.name = "Enter a name for this connection.";
    if (f.auth === "connection-string" && (!editing || f.replaceSecret) && !f.connectionString.trim())
      e.connectionString = "Paste the namespace's connection string.";
    if (f.auth === "entra" && !f.namespace.trim()) e.namespace = "Enter the namespace name, like sb-orders, or its full host.";
    if (f.auth === "entra" && !editing && !account.data) e.general = "Sign in with Entra ID first.";
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
      transport: f.transport,
      proxy: f.proxy.trim() || null,
      pinned_entities: f.pins.split("\n").map((s) => s.trim()).filter(Boolean),
    };
    try {
      const saved = await save.mutateAsync(draft);
      setF((x) => ({ ...x, connectionString: "" }));
      closeSheet();
      if (!editing || saved.id === conn.profileId) void conn.connect(saved.id);
    } catch (err) {
      setErrors(placeError(err as EngineError));
    }
  }

  async function onDelete() {
    if (!profile) return;
    try {
      await del.mutateAsync(profile.id);
      if (conn.profileId === profile.id) await conn.disconnect();
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
      footer={
        confirmDelete && profile ? (
          <div className="flex w-full flex-col gap-2">
            <p className="m-0 text-12">
              Delete {profile.name}?{" "}
              {profile.auth.kind === "connection-string" ? "Its saved connection string is removed from the keychain too." : "Your Entra sign-in stays."}
            </p>
            <div className="flex justify-end gap-2">
              <Button variant="quiet" onClick={() => setConfirmDelete(false)}>Keep</Button>
              <Button variant="danger" onClick={() => void onDelete()} disabled={del.isPending}>Delete {profile.name}</Button>
            </div>
          </div>
        ) : (
          <>
            {editing && <Button variant="quiet" onClick={() => setConfirmDelete(true)}>Delete…</Button>}
            {editing && (
              <Button onClick={() => test.mutate(profile.id)} disabled={test.isPending}>
                {test.isPending ? "Testing…" : "Test connection"}
              </Button>
            )}
            <div className="flex-1" />
            <Button variant="quiet" onClick={closeSheet}>Cancel</Button>
            <Button variant="primary" type="submit" form={formId} disabled={save.isPending}>{save.isPending ? "Saving…" : "Save"}</Button>
          </>
        )
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

        {!editing && (
          <div className="flex flex-col gap-1">
            <span className="text-12 font-semibold text-muted">Sign-in method</span>
            <div>
              <Segmented
                label="Sign-in method"
                value={f.auth}
                onChange={(v) => set("auth", v)}
                options={[{ value: "connection-string", label: "Connection string" }, { value: "entra", label: "Entra ID" }]}
              />
            </div>
          </div>
        )}

        {f.auth === "connection-string" ? (
          editing && !f.replaceSecret ? (
            <div className="flex items-center justify-between gap-2 text-12">
              <span className="text-muted">Stored in the system keychain.</span>
              <Button size="sm" onClick={() => set("replaceSecret", true)}>Replace</Button>
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              <Field
                label="Connection string"
                hint="From Shared access policies in the Azure Portal. It is stored in the system keychain and never shown again."
                error={errors.connectionString}
              >
                {(ids) => (
                  <Input {...ids} type="password" mono value={f.connectionString} onChange={(e) => set("connectionString", e.target.value)} placeholder="Endpoint=sb://…" />
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
            {!editing && <SignInPanel />}
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
            <Select value={f.transport} onValueChange={(v) => set("transport", v as Transport)}>
              <SelectTrigger aria-label="Transport"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="amqp">AMQP</SelectItem>
                <SelectItem value="websockets">AMQP over WebSockets</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        <Field label="Proxy" hint="http://host:port. Uses WebSockets. Leave empty to use HTTPS_PROXY from the environment." error={errors.proxy}>
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
          <ul aria-label="Connection test" className="m-0 flex list-none flex-col gap-1 border-t border-line p-0 pt-3">
            <CheckLine label="Management" c={test.data.management} />
            <CheckLine label="Messaging" c={test.data.messaging} />
          </ul>
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
