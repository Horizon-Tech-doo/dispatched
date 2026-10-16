import { useState, type ReactNode } from "react";
import { useAppInfo, useSignIns, useSignOut } from "@/engine";
import { Badge, Button, Problem, Segmented, Sheet } from "@/ui";
import { useShell } from "../context";
import type { AutoRefresh } from "../state";
import { sentence } from "./profileErrors";
import { SignInProgress, useSignInFlow } from "./SignInPanel";

/** A setting: its label and control side by side, or, `stacked`, the control full width below the label. */
function Row({ label, hint, stacked, children }: { label: string; hint?: string; stacked?: boolean; children: ReactNode }) {
  return (
    <div className={stacked ? "flex flex-col gap-2 py-3 first:pt-0 last:pb-0" : "flex items-center justify-between gap-4 py-3 first:pt-0 last:pb-0"}>
      <div className="flex min-w-0 flex-col">
        <span className="text-12 font-semibold">{label}</span>
        {hint && <span className="text-11 text-muted">{hint}</span>}
      </div>
      {children}
    </div>
  );
}

/** Every Entra sign-in kept on this computer: whose it is, which connections use it, sign in again, sign out. */
function Accounts() {
  const { conn } = useShell();
  const q = useSignIns();
  const signOut = useSignOut();
  const flow = useSignInFlow();
  const [confirm, setConfirm] = useState<string | null>(null);
  if (!q.data?.length) return <p className="m-0 text-12 text-muted">No Entra ID sign-ins. Add an Entra ID connection to sign in.</p>;
  const out = (id: string) =>
    signOut.mutate(id, {
      // Rust already closed the connection that used it; the shell's own connection state follows.
      onSuccess: (r) => {
        if (r.closed_profile_id !== null && r.closed_profile_id === conn.profileId) void conn.disconnect();
      },
      onSettled: () => setConfirm(null),
    });
  return (
    <>
      <ul aria-label="Accounts" className="m-0 flex list-none flex-col gap-3 p-0">
        {q.data.map((s) => (
          <li key={s.id} aria-label={s.user_principal} className="flex flex-col gap-1">
            <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
              <span className="min-w-0 break-all text-12" data-selectable>{s.user_principal}</span>
              {s.current && <Badge>Current</Badge>}
            </div>
            <span className="truncate font-mono text-11 text-muted" data-selectable>{s.tenant_id}</span>
            {(!s.same_account || !s.signed_in) && (
              <span className="flex gap-2 text-11 text-muted">
                {!s.same_account && <span>Another account</span>}
                {!s.signed_in && <span>Signed out</span>}
              </span>
            )}
            {s.profiles.length > 0 && <span className="text-11 text-muted">Used by {s.profiles.join(", ")}</span>}
            {confirm === s.id ? (
              <div className="mt-1 flex flex-col gap-2 border border-line px-2 py-1.5">
                <span className="text-12">
                  {s.profiles.length ? `${s.profiles.join(", ")} will need a new sign-in.` : "This sign-in will be forgotten."}
                </span>
                <div className="flex flex-wrap justify-end gap-2">
                  <Button size="sm" variant="quiet" onClick={() => setConfirm(null)}>Cancel</Button>
                  <Button size="sm" variant="danger" disabled={signOut.isPending} onClick={() => out(s.id)}>Sign out {s.user_principal}</Button>
                </div>
              </div>
            ) : (
              <div className="mt-1 flex flex-wrap gap-2">
                {!s.signed_in && (
                  <Button size="sm" disabled={flow.pending} onClick={() => flow.start(false, s.tenant_id)}>Sign in again</Button>
                )}
                <Button size="sm" onClick={() => setConfirm(s.id)}>Sign out</Button>
              </div>
            )}
          </li>
        ))}
      </ul>
      {flow.pending && <SignInProgress flow={flow} />}
      {flow.failure && <Problem title="Sign-in failed" body={sentence(flow.failure.message)} />}
      {signOut.error && <Problem title="Couldn't sign out" body={sentence(signOut.error.message)} />}
    </>
  );
}

export function SettingsSheet() {
  const { prefs, setPrefs, closeSheet, openSheet } = useShell();
  const info = useAppInfo().data;
  return (
    <Sheet open onOpenChange={(o) => !o && closeSheet()} title="Settings">
      <div className="flex flex-col divide-y divide-line">
        <Row label="Theme" hint="System follows the operating system.">
          <Segmented
            label="Theme"
            value={prefs.theme}
            onChange={(theme) => setPrefs({ theme })}
            options={[{ value: "system", label: "System" }, { value: "dark", label: "Dark" }, { value: "light", label: "Light" }]}
          />
        </Row>
        <Row label="Density" hint="Row height in the tree and grids.">
          <Segmented
            label="Density"
            value={prefs.density}
            onChange={(density) => setPrefs({ density })}
            options={[{ value: "compact", label: "Compact" }, { value: "comfortable", label: "Comfortable" }]}
          />
        </Row>
        <Row label="Auto-refresh" hint="Refreshes the connected namespace's counts.">
          <Segmented
            label="Auto-refresh"
            value={String(prefs.autoRefresh)}
            onChange={(v) => setPrefs({ autoRefresh: Number(v) as AutoRefresh })}
            options={[{ value: "0", label: "Off" }, { value: "30", label: "30 s" }, { value: "60", label: "60 s" }]}
          />
        </Row>
        <Row label="Version">
          <div className="flex items-center gap-3">
            <span className="text-12 tabular-nums text-muted">{info?.version}</span>
            <Button size="sm" onClick={() => openSheet({ kind: "update" })}>Check for updates</Button>
          </div>
        </Row>
        <Row label="Accounts" hint="Entra ID sign-ins on this computer, one per tenant you signed in to." stacked>
          <Accounts />
        </Row>
      </div>
    </Sheet>
  );
}
