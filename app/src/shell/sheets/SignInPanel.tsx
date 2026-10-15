import { CircleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { commands, onSignInPrompt, useAccount, useSignIn, type EngineError, type SignInPrompt } from "@/engine";
import { Button } from "@/ui";

/** One Entra sign-in, browser or device code, for the home tenant or one named tenant. A cancelled sign-in
 *  returns to idle without an error, and closing whatever shows it abandons a sign-in that is still waiting. */
export function useSignInFlow(onDone?: () => void) {
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
    signIn.mutate({ tenant, deviceCode }, { onSuccess: () => onDone?.(), onSettled: () => setPrompt(null) });
  };
  const cancel = () => void commands.signInCancel().catch(() => {});
  const failure = signIn.error && (signIn.error as EngineError).kind !== "cancelled" ? (signIn.error as EngineError) : null;
  return { pending: signIn.isPending, prompt, copied, copy, start, cancel, failure };
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
