import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useAppInfo, useProfiles, useSignIns, useSignOut } from "@/engine";
import { Badge, Button, Problem, Segmented, Sheet } from "@/ui";
import { useShell } from "../context";
import type { AutoRefresh, PeekSize } from "../state";
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
  const profiles = useProfiles();
  // Hook-level: the shell's connection state follows even when this sheet closes before the sign-out returns.
  const signOut = useSignOut(conn.closedByEngine);
  const flow = useSignInFlow();
  const [flowFor, setFlowFor] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const uid = useId();
  const cancelButton = useRef<HTMLButtonElement>(null);
  const actionButtons = useRef(new Map<string, HTMLButtonElement | null>());
  const restoreFocus = useRef<string | null>(null);
  // Focus follows the confirmation: onto Cancel when it opens, back onto the row's button when it is declined.
  useEffect(() => {
    if (confirm !== null) cancelButton.current?.focus();
    else if (restoreFocus.current) actionButtons.current.get(restoreFocus.current)?.focus();
    restoreFocus.current = null;
  }, [confirm]);

  if (q.isPending) return <p className="m-0 text-12 text-muted">Loading sign-ins…</p>;
  if (q.isError)
    return (
      <Problem
        title="Couldn't list the sign-ins"
        body={sentence(q.error.message)}
        action={<Button size="sm" onClick={() => void q.refetch()}>Try again</Button>}
      />
    );
  if (!q.data.length) return <p className="m-0 text-12 text-muted">No Entra ID sign-ins. Add an Entra ID connection to sign in.</p>;
  const connectedName = conn.status === "connected" ? profiles.data?.find((p) => p.id === conn.profileId)?.name : undefined;
  return (
    <>
      <ul aria-label="Accounts" className="m-0 flex list-none flex-col divide-y divide-line border-y border-line p-0">
        {q.data.map((s) => {
          const upnId = `${uid}-${s.id}`;
          const asking = confirm === s.id;
          const action = s.signed_in ? "Sign out" : "Forget sign-in";
          return (
            <li key={s.id} aria-label={s.user_principal} className="flex flex-col gap-1 py-2">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span id={upnId} className="min-w-0 break-all text-12" data-selectable>{s.user_principal}</span>
                {s.signed_in ? s.current && <Badge>Current</Badge> : <span className="shrink-0 text-11 text-muted">Sign-in expired</span>}
                {!asking && (
                  <div className="ml-auto flex shrink-0 gap-2">
                    {!s.signed_in && s.same_account && (
                      <Button
                        size="sm"
                        aria-describedby={upnId}
                        disabled={flow.pending}
                        onClick={() => {
                          setFlowFor(s.id);
                          flow.start(false, s.tenant_id);
                        }}
                      >
                        Sign in again
                      </Button>
                    )}
                    <Button
                      size="sm"
                      aria-describedby={upnId}
                      ref={(el) => void actionButtons.current.set(s.id, el)}
                      onClick={() => setConfirm(s.id)}
                    >
                      {action}
                    </Button>
                  </div>
                )}
              </div>
              <div className="flex flex-wrap items-baseline gap-x-3 text-11 text-muted">
                <span className="truncate font-mono" data-selectable>{s.tenant_id}</span>
                <span>{s.profiles.length > 0 ? `Used by ${s.profiles.join(", ")}` : "Not used by any connection"}</span>
              </div>
              {asking && (
                <div className="mt-1 flex flex-col gap-2 border border-line px-2 py-1.5">
                  <span className="text-12">
                    {s.profiles.length ? `${s.profiles.join(", ")} will need a new sign-in.` : "This sign-in will be forgotten."}
                    {connectedName !== undefined && s.profiles.includes(connectedName) && ` ${connectedName} is connected and will disconnect.`}
                  </span>
                  <div className="flex flex-wrap justify-end gap-2">
                    <Button
                      size="sm"
                      variant="quiet"
                      ref={cancelButton}
                      onClick={() => {
                        restoreFocus.current = s.id;
                        setConfirm(null);
                      }}
                    >
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={signOut.isPending}
                      onClick={() => signOut.mutate(s.id, { onSettled: () => setConfirm(null) })}
                    >
                      {action} {s.user_principal}
                    </Button>
                  </div>
                </div>
              )}
              {flowFor === s.id && flow.pending && <SignInProgress flow={flow} />}
              {flowFor === s.id && flow.failure && <Problem title="Sign-in failed" body={sentence(flow.failure.message)} />}
            </li>
          );
        })}
      </ul>
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
        <Row label="Peek size" hint="Messages each view reads at first. Peek more reads the same number again.">
          <Segmented
            label="Peek size"
            value={String(prefs.peekSize)}
            onChange={(v) => setPrefs({ peekSize: Number(v) as PeekSize })}
            options={[{ value: "50", label: "50" }, { value: "100", label: "100" }, { value: "500", label: "500" }, { value: "1000", label: "1,000" }]}
          />
        </Row>
        <Row label="Version">
          <div className="flex items-center gap-3">
            <span className="text-12 tabular-nums text-muted">{info?.version}</span>
            {info?.updates ? (
              <Button size="sm" onClick={() => openSheet({ kind: "update" })}>Check for updates</Button>
            ) : (
              info && <span className="text-12 text-muted">Updates come from the App Store.</span>
            )}
          </div>
        </Row>
        <Row label="Accounts" hint="Entra ID sign-ins on this computer, one per tenant you signed in to." stacked>
          <Accounts />
        </Row>
      </div>
    </Sheet>
  );
}
