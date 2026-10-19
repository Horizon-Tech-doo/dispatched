import { useCallback, useEffect, useRef, useState } from "react";
import { EngineError, commands, type ConfirmAnswer, type JobStarted } from "@/engine";
import { jobStore, useJobs, type Job } from "../drawer/activity";

export type Stage = "form" | "confirming" | "running" | "done";

export interface Operation {
  stage: Stage;
  /** The job this sheet started (or was opened on), once its `started` event arrived. */
  job: Job | null;
  jobId: string | null;
  /** The `op_*` call is on its way. */
  starting: boolean;
  /** The engine refused to start the job. */
  failure: { kind: string; message: string } | null;
  /** This sheet has answered at least one of the job's confirmations "go ahead". */
  answered: boolean;
  /** Starts a job; its confirmations, items and finish then arrive as events. */
  start(run: () => Promise<JobStarted>): Promise<void>;
  /** Answers the job's waiting confirmation. */
  answer(a: ConfirmAnswer): Promise<void>;
  /** Cancels the job (a waiting confirmation is answered no). */
  cancel(): void;
  /** Back to the form, forgetting the job (it stays in the Jobs tab). */
  reset(): void;
  /** The sheet is closing: a job that waits for a confirmation, or has not begun its work, is cancelled. */
  leave(): void;
  /** The sheet closes and the job carries on in the Jobs tab: leaving does not cancel it. */
  detach(): void;
}

/**
 * Starts one operate job (or follows `jobId`) and follows it: form, then the confirmations it asks for, then its run,
 * then its report. Leaving (the sheet closes or is replaced) while a confirmation waits, or before the job began its
 * work, cancels the job, so nothing waits on a question nobody can see. This holds at any step, a purge's second
 * entity included.
 */
export function useOperation(jobIdToFollow: string | null = null): Operation {
  const [jobId, setJobId] = useState<string | null>(jobIdToFollow);
  const [starting, setStarting] = useState(false);
  const [answered, setAnswered] = useState(false);
  const [failure, setFailure] = useState<Operation["failure"]>(null);
  const jobs = useJobs();
  const job = jobId ? (jobs.find((j) => j.id === jobId) ?? null) : null;
  const stage: Stage = !jobId ? "form" : job?.finished ? "done" : job?.confirm ? "confirming" : "running";

  const live = useRef<{ id: string | null; job: Job | null; detached: boolean }>({ id: null, job: null, detached: false });
  live.current = { ...live.current, id: jobId, job };
  const leave = useCallback(() => {
    const { id, job: j, detached } = live.current;
    if (!id || detached || j?.finished) return;
    if (!j || j.confirm || !j.progressed) void commands.jobCancel(id).catch(() => {});
  }, []);
  const detach = useCallback(() => {
    live.current.detached = true;
  }, []);
  // Replaced by another sheet, or the shell went away.
  useEffect(() => leave, [leave]);

  const start = useCallback(async (run: () => Promise<JobStarted>) => {
    setFailure(null);
    setStarting(true);
    setAnswered(false);
    live.current.detached = false;
    try {
      const { id } = await run();
      setJobId(id);
    } catch (e) {
      setFailure({ kind: e instanceof EngineError ? e.kind : "unknown", message: e instanceof Error ? e.message : String(e) });
    } finally {
      setStarting(false);
    }
  }, []);

  const confirmId = job?.confirm?.id;
  const answer = useCallback(
    async (a: ConfirmAnswer) => {
      if (!jobId || !confirmId) return;
      jobStore.answered(jobId);
      if (a.type !== "deny") setAnswered(true);
      await commands.confirmAnswer(confirmId, a).catch(() => false);
    },
    [jobId, confirmId],
  );
  const cancel = useCallback(() => {
    if (jobId) void commands.jobCancel(jobId).catch(() => {});
  }, [jobId]);
  const reset = useCallback(() => {
    setJobId(null);
    setAnswered(false);
  }, []);

  return { stage, job, jobId, starting, failure, answered: answered || (jobIdToFollow !== null && jobId === jobIdToFollow), start, answer, cancel, reset, leave, detach };
}
