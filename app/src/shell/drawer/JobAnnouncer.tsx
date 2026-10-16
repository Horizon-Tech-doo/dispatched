import { useEffect, useRef, useState } from "react";
import { useJobs, type Job } from "./activity";
import { plural } from "./plural";

/** What a screen reader hears when a job starts. */
function startedMessage(j: Job): string {
  return `${j.title} started.`;
}

/** What a screen reader hears when a job ends. */
function finishedMessage(j: Job): string {
  if (j.error?.kind === "cancelled") return `${j.title} was cancelled.`;
  if (j.error) return `${j.title} failed.`;
  return j.written != null ? `${j.title} finished: ${plural(j.written, "message")} written.` : `${j.title} finished.`;
}

interface Said {
  key: number;
  text: string;
}

/**
 * A polite live region that says each job's start and ending once, and nothing in between (progress is not
 * announced). It lives in the shell, not the drawer, so a job that ends while the drawer is hidden is still heard.
 * Jobs already running or finished when it mounts are not announced. Every announcement is its own node with a
 * new key, so two jobs with the same title (or one job saying the same thing again) are each heard.
 */
export function JobAnnouncer() {
  const jobs = useJobs();
  const [said, setSaid] = useState<Said[]>([]);
  const started = useRef<Set<string> | null>(null);
  const ended = useRef<Set<string> | null>(null);
  const next = useRef(1);
  if (started.current === null) started.current = new Set(jobs.map((j) => j.id));
  if (ended.current === null) ended.current = new Set(jobs.filter((j) => j.finished).map((j) => j.id));
  useEffect(() => {
    const lines: string[] = [];
    for (const j of jobs) {
      if (!j.finished) {
        // A job that starts again under the same id is a new job.
        ended.current!.delete(j.id);
        if (!started.current!.has(j.id)) {
          started.current!.add(j.id);
          lines.push(startedMessage(j));
        }
      } else if (!ended.current!.has(j.id)) {
        started.current!.add(j.id);
        ended.current!.add(j.id);
        lines.push(finishedMessage(j));
      }
    }
    if (lines.length) setSaid(lines.map((text) => ({ key: next.current++, text })));
  }, [jobs]);
  return (
    <div role="status" aria-label="Job updates" className="sr-only">
      {said.map((s) => (
        <p key={s.key}>{s.text}</p>
      ))}
    </div>
  );
}
