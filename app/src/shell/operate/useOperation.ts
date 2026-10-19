import { useCallback, useEffect, useRef, useState } from "react";
import { EngineError, commands, type ConfirmAnswer, type JobStarted } from "@/engine";
import { jobStore, useJobs, type Job } from "../drawer/activity";

export type Stage = "form" | "confirming" | "running" | "done";

export interface Operation {
  stage: Stage;
  /** The job this sheet started, once its `started` event arrived. */
  job: Job | null;
  jobId: string | null;
  /** The `op_*` call is on its way. */
  starting: boolean;
  /** The engine refused to start the job. */
  failure: { kind: string; message: string } | null;
  /** Starts a job; its confirmations, items and finish then arrive as events. */
  start(run: () => Promise<JobStarted>): Promise<void>;
  /** Answers the job's waiting confirmation. */
  answer(a: ConfirmAnswer): Promise<void>;
  /** Cancels the job (a waiting confirmation is answered no). */
  cancel(): void;
  /** Back to the form, forgetting the job (it stays in the Jobs tab). */
  reset(): void;
  /** The sheet is closing: a job that has not begun its work (it waits for, or between, confirmations) is cancelled. */
  leave(): void;
}

/** Starts one operate job and follows it: form, then the confirmations it asks for, then its run, then its report.
 *  Leaving (the sheet closes or is replaced) before the job began its work answers its confirmation no, so nothing
 *  waits on a question nobody can see. */
export function useOperation(): Operation {
  const [jobId, setJobId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [failure, setFailure] = useState<Operation["failure"]>(null);
  const jobs = useJobs();
  const job = jobId ? (jobs.find((j) => j.id === jobId) ?? null) : null;
  const stage: Stage = !jobId ? "form" : job?.finished ? "done" : job?.confirm ? "confirming" : "running";

  const live = useRef<{ id: string | null; job: Job | null }>({ id: null, job: null });
  live.current = { id: jobId, job };
  const leave = useCallback(() => {
    const { id, job: j } = live.current;
    if (id && !j?.finished && !j?.progressed) void commands.jobCancel(id).catch(() => {});
  }, []);
  // Replaced by another sheet, or the shell went away.
  useEffect(() => leave, [leave]);

  const start = useCallback(async (run: () => Promise<JobStarted>) => {
    setFailure(null);
    setStarting(true);
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
      await commands.confirmAnswer(confirmId, a).catch(() => false);
    },
    [jobId, confirmId],
  );
  const cancel = useCallback(() => {
    if (jobId) void commands.jobCancel(jobId).catch(() => {});
  }, [jobId]);
  const reset = useCallback(() => setJobId(null), []);

  return { stage, job, jobId, starting, failure, start, answer, cancel, reset, leave };
}
