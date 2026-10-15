import { CircleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { commands, onSignInPrompt, useAccount, useSignIn, type EngineError, type SignInPrompt, type SignInView } from "@/engine";
import { Button, Problem } from "@/ui";
import { sentence } from "./profileErrors";

/** One Entra sign-in, browser or device code, for the home tenant or one named tenant. A cancelled sign-in
 *  returns to idle without an error, and closing whatever shows it abandons a sign-in that is still waiting. */
export function useSignInFlow(onDone?: (result: SignInView) => void) {
  const signIn = useSignIn();
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null);
  const pending = useRef(false);
  pending.current = signIn.isPending;
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(copiedTimer.current), []);
  const copy = (code: string) => {
    void navigator.clipboard?.writeText(code);
    setCopied(true);
    clearTimeout(copiedTimer.current);
    copiedTimer.current = setTimeout(() => setCopied(false), 1500);
  };

  useEffect(() => {
    let off: (() => void) | undefined;
    let live = true;
    void onSignInPrompt((p) => setPrompt(p)).then((f) => (live ? (off = f) : f()));
    return () => {
      live = false;
      off?.();
      // Closing the sheet abandons a sign-in that is still waiting.
      if (pending.current) void commands.signInCancel().catch(() => {});
    };
  }, []);

  const start = (deviceCode: boolean, tenant: string | null = null) => {
    setPrompt(null);
    signIn.mutate({ tenant, deviceCode }, { onSuccess: (r) => onDone?.(r), onSettled: () => setPrompt(null) });
  };
  const cancel = () => void commands.signInCancel().catch(() => {});
  const failure = signIn.error && (signIn.error as EngineError).kind !== "cancelled" ? (signIn.error as EngineError) : null;
  // A tenant sign-in made with another account than the current one: kept, but never used for this account.
  const tenantSignIn = signIn.data?.tenant_sign_in ?? null;
  const otherAccount = tenantSignIn && !tenantSignIn.same_account ? { ...tenantSignIn, account: signIn.data!.account.user_principal } : null;
  return { pending: signIn.isPending, prompt, copied, copy, start, cancel, failure, otherAccount };
}

/** After a tenant sign-in with another account: say which account was used and which one is needed. */
export function OtherAccountNotice({ flow, name }: { flow: SignInFlow; name: string }) {
  const o = flow.otherAccount;
  if (!o) return null;
  return (
    <Problem
      title={`Signed in to ${name} as ${o.user_principal}`}
      body={`Connections here use ${o.account}. Sign in to ${name} with that account.`}
    />
  );
}

/** "Sign in to <tenant>": that tenant's own sign-in (MFA, Conditional Access), browser or device code,
 *  with the progress, any failure, and a notice when another account was used. */
export function TenantSignInAction({ tenantId, name, onSignedIn, primary }: {
  tenantId: string;
  name: string;
  onSignedIn?: (result: SignInView) => void;
  primary?: boolean;
}) {
  const flow = useSignInFlow(onSignedIn);
  if (flow.pending) return <SignInProgress flow={flow} />;
  return (
    <div className="flex flex-col items-start gap-1.5">
      <div className="flex flex-wrap gap-2">
        <Button variant={primary ? "primary" : undefined} onClick={() => flow.start(false, tenantId)}>Sign in to {name}</Button>
        <Button variant="quiet" onClick={() => flow.start(true, tenantId)}>Use a device code instead</Button>
      </div>
      {flow.failure && <Problem title="Sign-in failed" body={sentence(flow.failure.message)} />}
      <OtherAccountNotice flow={flow} name={name} />
    </div>
  );
}

export type SignInFlow = ReturnType<typeof useSignInFlow>;

/** What shows while a sign-in waits: the browser notice, or the device code with its page. */
export function SignInProgress({ flow }: { flow: SignInFlow }) {
  const { prompt, copied, copy, cancel } = flow;
  return (
    <div className="flex flex-col gap-2 border border-line p-3">
      {prompt?.kind === "device-code" ? (
        <>
          <p className="m-0 text-12">
            Open <span className="font-mono text-11" data-selectable>{prompt.url}</span> and enter this code:
          </p>
          <p className="m-0 font-mono text-14 tracking-[0.08em]" data-selectable>{prompt.user_code}</p>
          <p className="m-0 text-11 text-muted">Waiting for you to finish signing in…</p>
          <div className="flex gap-2">
            <Button size="sm" onClick={() => copy(prompt.user_code ?? "")}>{copied ? "Copied" : "Copy code"}</Button>
            <Button size="sm" variant="quiet" onClick={cancel}>Cancel sign-in</Button>
          </div>
        </>
      ) : (
        <>
          <p className="m-0 text-12">{prompt?.message ?? "Opening your browser…"}</p>
          <div className="flex gap-2">
            <Button size="sm" variant="quiet" onClick={cancel}>Cancel sign-in</Button>
          </div>
        </>
      )}
    </div>
  );
}

/** Entra ID sign-in: browser or device code. */
export function SignInPanel({ onDone }: { onDone?: () => void }) {
  const account = useAccount();
  const flow = useSignInFlow(onDone);

  if (flow.pending) return <SignInProgress flow={flow} />;

  const failureLine = flow.failure && (
    <p className="m-0 flex items-start gap-1.5 text-12">
      <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
      <span>{flow.failure.message}</span>
    </p>
  );

  if (account.data) {
    return (
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between gap-2 text-12">
          <span>Signed in as {account.data.user_principal}</span>
          <Button size="sm" variant="quiet" onClick={() => flow.start(false)}>Use another account</Button>
        </div>
        {failureLine}
      </div>
    );
  }

  return (
    <div className="flex flex-col items-start gap-1.5">
      <Button onClick={() => flow.start(false)}>Sign in with Entra ID</Button>
      <Button size="sm" variant="quiet" onClick={() => flow.start(true)}>Use a device code instead</Button>
      {failureLine}
    </div>
  );
}
