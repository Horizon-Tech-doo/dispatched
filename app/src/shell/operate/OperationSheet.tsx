import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button, Problem, Sheet, formatNumber } from "@/ui";
import { useShell } from "../context";
import type { Job } from "../drawer/activity";
import { ConfirmPanel, answerFor, canConfirm } from "./ConfirmPanel";
import { ReportView } from "./ReportView";
import type { Operation } from "./useOperation";
import { useAuthKind, type Right } from "../access";
import { adviceFor, rightFor } from "./advice";
import { confirmLabel, endedText, failedTitle, itemsSoFar, neutralEnd, runningText } from "./words";

/** How far a running job is: the job's own progress bar and numbers. */
function Running({ job }: { job: Job | null }) {
  if (!job) return <p className="m-0 text-12 text-muted">Starting…</p>;
  const n = Math.max(job.done, itemsSoFar(job));
  return (
    <div className="flex flex-col gap-2">
      <p className="m-0 text-12 text-fg">{job.title}</p>
      {job.total != null ? (
        <span role="progressbar" aria-label={job.title} aria-valuemin={0} aria-valuemax={job.total} aria-valuenow={Math.min(n, job.total)} className="block h-[3px] bg-line">
          <span className="block h-[3px] bg-fg" style={{ width: `${job.total ? Math.min(100, (n / job.total) * 100) : 0}%` }} />
        </span>
      ) : (
        <span aria-hidden="true" className="block h-[3px] bg-line" />
      )}
      <p className="m-0 text-11 tabular-nums text-muted">{runningText(job)}</p>
      {job.notes.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0 text-11 text-muted">
          {job.notes.map((t, i) => (
            <li key={i}>{t}</li>
          ))}
        </ul>
      )}
      {job.received > 0 && <p className="m-0 text-11 tabular-nums text-muted">{formatNumber(job.received)} received</p>}
    </div>
  );
}

/** How a finished job went: a neutral line when it was not confirmed, cancelled or stopped, the error form when it
 *  failed, and the report whenever there is one. */
function Done({ job }: { job: Job }) {
  const auth = useAuthKind();
  const ended = endedText(job);
  const failed = job.error && !neutralEnd(job.error.kind);
  return (
    <div className="flex flex-col gap-3">
      {ended && <p className="m-0 text-12 text-muted">{ended}</p>}
      {failed && job.error && (
        <Problem
          title={failedTitle(job.kind)}
          body={`${job.report ? "The report lists what was done before it stopped." : "Nothing was changed."} ${adviceFor(job.error.kind, job.error.message, rightFor(job.kind), auth)}`}
          detail={job.error.message}
        />
      )}
      <ReportView job={job} />
    </div>
  );
}

/**
 * The frame every operation sheet uses: its form, then the core's confirmations (one at a time, in this sheet), then
 * the run, then the report. One primary button per stage: the action, the confirmation, Cancel job, Close. While a
 * confirmation waits or the job runs the sheet cannot be dismissed; Cancel (while confirming) closes it and answers no.
 */
export function OperationSheet({
  title,
  description,
  op,
  action,
  failureTitle,
  right = "Listen",
  canSubmit = true,
  onSubmit,
  onClose,
  children,
}: {
  title: string;
  description?: string;
  op: Operation;
  /** The form's primary button, naming what it does: "Delete 3 messages". */
  action: string;
  /** The heading of a refused start; by default "Couldn't" and the title, "Couldn't delete messages". */
  failureTitle?: string;
  /** The right the operation needs, for advice when the engine does not name it. */
  right?: Right;
  canSubmit?: boolean;
  onSubmit: () => void;
  onClose: () => void;
  /** The form. */
  children?: ReactNode;
}) {
  const { dispatch } = useShell();
  const auth = useAuthKind();
  const [typed, setTyped] = useState<{ id: string; text: string }>({ id: "", text: "" });
  const [cancelling, setCancelling] = useState<string | null>(null);
  const ask = op.stage === "confirming" ? op.job?.confirm ?? null : null;
  const text = ask && typed.id === ask.id ? typed.text : "";
  const busy = op.stage === "confirming" || op.stage === "running";
  const failedJob = op.stage === "done" && !!op.job?.error && !neutralEnd(op.job.error.kind);
  const close = () => {
    op.leave();
    onClose();
  };

  // Each stage brings its own footer (keyed, so no button carries focus from one stage into the next) and focus goes to a
  // safe place: never the confirm button. On production the typed-name field takes it.
  const settle = useRef<HTMLButtonElement>(null);
  const stageKey = ask ? `confirm-${ask.id}` : op.stage;
  useEffect(() => {
    if (op.stage === "form" || ask?.production) return;
    settle.current?.focus();
  }, [stageKey, op.stage, ask?.production]);

  const footer =
    op.stage === "form" ? (
      <FooterGroup key="form">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" disabled={!canSubmit || op.starting} aria-busy={op.starting} onClick={onSubmit}>
          {action}
        </Button>
      </FooterGroup>
    ) : ask ? (
      <FooterGroup key={`confirm-${ask.id}`}>
        <Button ref={settle} onClick={close}>
          Cancel
        </Button>
        <Button variant="primary" disabled={!canConfirm(ask, text)} onClick={() => void op.answer(answerFor(ask, text))}>
          {confirmLabel(ask.verb, ask.target, ask.step)}
        </Button>
      </FooterGroup>
    ) : op.stage === "running" ? (
      <FooterGroup key="running">
        {op.answered ? (
          <Button
            ref={settle}
            onClick={() => {
              op.detach();
              dispatch({ type: "layout", patch: { drawerOpen: true, drawerTab: "jobs" } });
              onClose();
            }}
          >
            Show in Jobs
          </Button>
        ) : (
          // Nothing has been confirmed yet: closing now cancels the job, and the button says so.
          <Button ref={settle} onClick={close}>
            Cancel and close
          </Button>
        )}
        <Button
          variant="primary"
          disabled={cancelling === op.jobId}
          onClick={() => {
            setCancelling(op.jobId);
            op.cancel();
          }}
        >
          {cancelling === op.jobId ? "Cancelling" : "Cancel job"}
        </Button>
      </FooterGroup>
    ) : (
      <FooterGroup key="done">
        {failedJob && <Button onClick={op.reset}>Edit and try again</Button>}
        <Button ref={settle} variant="primary" onClick={onClose}>
          Close
        </Button>
      </FooterGroup>
    );

  return (
    <Sheet
      open
      onOpenChange={(o) => !o && close()}
      title={title}
      description={description}
      dismissible={!busy}
      initialFocus={op.stage === "form" ? "field" : "sheet"}
      footer={footer}
    >
      {op.stage === "form" ? (
        <div className="flex flex-col gap-4">
          {/* First, so a refusal is seen without scrolling a long form. */}
          {op.failure && (
            <Problem
              role="alert"
              title={failureTitle ?? `Couldn't ${title[0].toLowerCase()}${title.slice(1)}`}
              body={adviceFor(op.failure.kind, op.failure.message, right, auth)}
              detail={op.failure.message}
            />
          )}
          {children}
        </div>
      ) : (
        // The stage is announced: the confirmation, the progress, the outcome.
        <div aria-live="polite">
          {ask ? (
            <ConfirmPanel key={ask.id} request={ask} typed={text} onTyped={(t) => setTyped({ id: ask.id, text: t })} />
          ) : op.stage === "running" ? (
            <Running job={op.job} />
          ) : (
            op.job && <Done job={op.job} />
          )}
        </div>
      )}
    </Sheet>
  );
}

/** One stage's footer buttons. */
function FooterGroup({ children }: { children: ReactNode }) {
  return <>{children}</>;
}
