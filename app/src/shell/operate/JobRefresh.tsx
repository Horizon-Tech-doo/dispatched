import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { useShell } from "../context";
import { useJobs } from "../drawer/activity";
import { invalidateEntity } from "./invalidate";
import { takeRename } from "./renames";

/** When an operate job ends, every entity it may have changed is read again: its source, its target and the entities
 *  its items were in. A job that was not confirmed changed nothing. Jobs already ended when this mounts are left. */
export function JobRefresh() {
  const jobs = useJobs();
  const { conn, dispatch } = useShell();
  const qc = useQueryClient();
  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null) seen.current = new Set(jobs.filter((j) => j.finished).map((j) => j.id));
  useEffect(() => {
    for (const j of jobs) {
      if (!j.finished || seen.current!.has(j.id)) continue;
      seen.current!.add(j.id);
      const renamed = takeRename(j.id);
      if (j.kind === "export" || j.error?.kind === "denied") continue;
      // A rename that went through: the tab of the old name now shows the new one.
      if (renamed && (j.report?.succeeded ?? 0) > 0) dispatch({ type: "retarget", from: renamed.from, to: renamed.to });
      const touched = new Set([...j.entities, ...(j.report ? [j.report.entity, j.report.target ?? ""] : [])].filter(Boolean));
      for (const e of touched) invalidateEntity(qc, conn.profileId, e);
    }
  }, [jobs, qc, conn.profileId, dispatch]);
  return null;
}
