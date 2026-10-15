import { CircleAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { commands, onSignInPrompt, useAccount, useSignIn, type EngineError, type SignInPrompt } from "@/engine";
import { Button } from "@/ui";

/** Entra ID sign-in: browser or device code. A cancelled sign-in returns to idle without an error. */
export function SignInPanel({ onDone }: { onDone?: () => void }) {
  const account = useAccount();
  const signIn = useSignIn();
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null);
  const pending = useRef(false);
  pending.current = signIn.isPending;

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

  const start = (deviceCode: boolean) => {
    setPrompt(null);
    signIn.mutate(
      { tenant: null, deviceCode },
      { onSuccess: () => onDone?.(), onSettled: () => setPrompt(null) },
    );
  };
  const cancel = () => void commands.signInCancel().catch(() => {});

  if (signIn.isPending) {
    return (
      <div className="flex flex-col gap-2 border border-line p-3">
        {prompt?.kind === "device-code" ? (
          <>
            <p className="m-0 text-12">
              Open <span className="font-mono text-11" data-selectable>{prompt.url}</span> and enter this code:
            </p>
            <p className="m-0 font-mono text-14 tracking-[0.08em]" data-selectable>{prompt.user_code}</p>
            <div className="flex gap-2">
              <Button size="sm" onClick={() => void navigator.clipboard?.writeText(prompt.user_code ?? "")}>Copy code</Button>
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

  if (account.data) {
    return (
      <div className="flex items-center justify-between gap-2 text-12">
        <span>Signed in as {account.data.user_principal}</span>
        <Button size="sm" variant="quiet" onClick={() => start(false)}>Use another account</Button>
      </div>
    );
  }

  const failure = signIn.error && (signIn.error as EngineError).kind !== "cancelled" ? signIn.error : null;
  return (
    <div className="flex flex-col items-start gap-1.5">
      <Button onClick={() => start(false)}>Sign in with Entra ID</Button>
      <Button size="sm" variant="quiet" onClick={() => start(true)}>Use a device code instead</Button>
      {failure && (
        <p className="m-0 flex items-start gap-1.5 text-12">
          <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
          <span>{failure.message}</span>
        </p>
      )}
    </div>
  );
}
