import { Button, EmptyState, Sheet } from "@/ui";
import { useShell } from "../context";
import { useJobs } from "../drawer/activity";
import { OperationSheet } from "./OperationSheet";
import { ReportView } from "./ReportView";
import { useOperation } from "./useOperation";
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

/** A running job opened from the Jobs tab: its waiting confirmation, its progress and then its report, in the same frame
 *  its own sheet uses. */
export function JobSheet({ jobId }: { jobId: string }) {
  const { closeSheet } = useShell();
  const op = useOperation(jobId);
  return <OperationSheet title={op.job?.title ?? "Job"} op={op} action="" canSubmit={false} onSubmit={() => {}} onClose={closeSheet} />;
}
