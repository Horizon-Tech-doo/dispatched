import { Button, EmptyState, Sheet } from "@/ui";
import { useShell } from "../context";
import { useJobs } from "../drawer/activity";
import { ReportView } from "./ReportView";
import { endedText } from "./words";

/** A job's report, opened from the Jobs tab. */
export function ReportSheet({ jobId }: { jobId: string }) {
  const { closeSheet } = useShell();
  const job = useJobs().find((j) => j.id === jobId);
  const ended = job && endedText(job);
  return (
    <Sheet open onOpenChange={(o) => !o && closeSheet()} title={job?.title ?? "Report"} footer={<Button variant="primary" onClick={closeSheet}>Close</Button>}>
      {job ? (
        <div className="flex flex-col gap-3">
          {ended && <p className="m-0 text-12 text-muted">{ended}</p>}
          <ReportView job={job} />
        </div>
      ) : (
        <EmptyState className="px-0 py-0" title="This job is no longer listed" body="Its report was cleared from the Jobs tab." />
      )}
    </Sheet>
  );
}
