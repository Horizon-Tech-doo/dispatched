import { ChevronDown, CircleAlert } from "lucide-react";
import { useState } from "react";
import { commands } from "@/engine";
import { Button, EmptyState, Hint, IconButton, Tabs, TabsContent, TabsList, TabsTrigger, fileManagerName, formatNumber } from "@/ui";
import { useShell } from "../context";
import { jobStore, useJobs, type Job } from "./activity";
import { ActivityLog } from "./ActivityLog";
import { plural } from "./plural";
import { endedText, endKind, failedTitle, finishedState, neutralEnd, outcomeText, runningText } from "../operate/words";
import { adviceFor, rightFor } from "../operate/advice";
import { useAuthKind } from "../access";

/** Where a job stands, in numbers: "5 of 37", or, with no total, "5 written" and how many were looked at. */
function counts(j: Job): string {
  if (j.total != null) return `${formatNumber(j.done)} of ${formatNumber(j.total)}`;
  return `${formatNumber(j.done)} written${j.scanned > j.done ? ` · ${formatNumber(j.scanned)} scanned` : ""}`;
}

/** What to do about a failed export, by what failed. The engine's own words go on the detail line. */
function nextStep(kind: string): string {
  switch (kind) {
    case "network":
    case "timeout":
    case "throttled":
    case "protocol":
      return "Check the connection, then try again.";
    case "auth":
    case "not_signed_in":
    case "forbidden":
      return "Sign in again or check the access, then try again.";
    case "not_found":
      return "The entity may have been deleted. Refresh the tree, then try again.";
    default:
      return "Try again, or choose another folder.";
  }
}

/** One job: what it is, the file it writes, how far it got, and what can be done with it (Cancel while it runs,
 *  Show in the file manager once an export has written its file, Try again after a failure). Wide: title, bar,
 *  numbers and action in a row; narrow (the drawer's own width, by container query): stacked. */
function ExportRow({ j }: { j: Job }) {
  const { openSheet } = useShell();
  const [cancelling, setCancelling] = useState(false);
  const [revealFailed, setRevealFailed] = useState(false);
  const cancelled = j.error?.kind === "cancelled";
  // Stopped because its connection closed (disconnect, another connection, sign-out): not the user's cancel, not a failure.
  const stopped = j.error?.kind === "stopped";
  const failed = j.error != null && !cancelled && !stopped;
  const running = !j.finished;
  const manager = fileManagerName();
  const pct = j.total ? Math.min(100, (j.done / j.total) * 100) : 0;
  const cancel = () => {
    setCancelling(true);
    // False: the job had already ended, and its finish event is on its way. An error: it is still running.
    commands.jobCancel(j.id).catch(() => setCancelling(false));
  };
  const reveal = () => {
    setRevealFailed(false);
    commands.jobReveal(j.id).catch(() => setRevealFailed(true));
  };
  const retry = j.retry;
  const state = running ? counts(j) : cancelled ? "Cancelled" : stopped ? "Stopped" : failed ? counts(j) : "Done";
  const sofar = j.total != null ? counts(j) : `${formatNumber(j.done)} written`;
  const missing = j.missing ?? 0;
  return (
    <li className="grid grid-cols-1 gap-y-1.5 px-3.5 py-2 text-12 @[560px]:grid-cols-[minmax(200px,1fr)_minmax(120px,200px)_max-content] @[560px]:items-start @[560px]:gap-x-4">
      <span className="min-w-0">
        <span className="block truncate">{j.title}</span>
        {j.fileName && (
          <span className="block truncate font-mono text-11 text-muted" title={j.fileName} data-selectable>
            {j.fileName}
          </span>
        )}
        {j.finished && !j.error && j.written != null && <span className="block text-11 text-muted">{plural(j.written, "message")} written</span>}
        {j.finished && !j.error && missing > 0 && (
          <span className="block text-11 text-muted">
            {missing === 1 ? "1 selected message was" : `${formatNumber(missing)} selected messages were`} no longer there
          </span>
        )}
        {cancelled && (
          <span className="block text-11 text-muted">
            {j.done > 0 ? `Cancelled after ${sofar}.` : "Cancelled."} No file was written.
          </span>
        )}
        {stopped && (
          <span className="block text-11 text-muted">
            {j.done > 0 ? `Stopped: the connection closed after ${sofar}.` : "Stopped: the connection closed."} No file was written.
          </span>
        )}
        {failed && j.error && (
          <span className="mt-1 flex flex-col gap-0.5">
            <span className="flex items-center gap-1.5 text-12 font-semibold text-fg">
              <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />
              Export failed
            </span>
            <span className="text-11 text-muted">{nextStep(j.error.kind)}</span>
            <span className="min-w-0 break-words font-mono text-11 text-fg" data-selectable>
              {j.error.message}
            </span>
          </span>
        )}
        {revealFailed && (
          <span role="alert" className="mt-0.5 flex items-start gap-1.5 text-11 text-fg">
            <CircleAlert aria-hidden="true" className="mt-0.5 size-3 shrink-0 text-accent" />
            <span className="min-w-0">Couldn&apos;t show the file. It may have been moved or deleted.</span>
          </span>
        )}
      </span>
      {running && j.total != null ? (
        <span
          role="progressbar"
          aria-label={j.title}
          aria-valuemin={0}
          aria-valuemax={j.total}
          aria-valuenow={Math.min(j.done, j.total)}
          className="block h-[3px] bg-line @[560px]:mt-[7px]"
        >
          <span className="block h-[3px] bg-fg" style={{ width: `${pct}%` }} />
        </span>
      ) : running ? (
        // No total, so nothing to fill: the numbers say how far it is.
        <span aria-hidden="true" className="block h-[3px] bg-line @[560px]:mt-[7px]" />
      ) : (
        // A finished row has no bar; its column stays so the rows line up.
        <span aria-hidden="true" className="hidden @[560px]:block" />
      )}
      <span className="flex items-center justify-between gap-3 @[560px]:justify-end">
        <span className="min-w-[96px] whitespace-nowrap text-left text-11 tabular-nums text-muted @[560px]:text-right">{state}</span>
        {running && (
          <Button size="sm" className="h-[18px]" disabled={cancelling} onClick={cancel} aria-label={`${cancelling ? "Cancelling" : "Cancel"} ${j.title}`}>
            {cancelling ? "Cancelling" : "Cancel"}
          </Button>
        )}
        {j.finished && !j.error && j.output && (
          <Button size="sm" className="h-[18px]" onClick={reveal} aria-label={`Show in ${manager}${j.fileName ? `: ${j.fileName}` : ""}`}>
            Show in {manager}
          </Button>
        )}
        {failed && retry && (
          <Button
            size="sm"
            className="h-[18px]"
            onClick={() =>
              openSheet({ kind: "export", entity: retry.entity, view: retry.view, filter: retry.filter, selected: retry.selected, total: retry.total, scope: retry.scope, format: retry.format })
            }
          >
            Try again
          </Button>
        )}
      </span>
    </li>
  );
}

/** One operate job: what it is, how far it got or how it went ("2 of 3 deleted, 1 failed"), Cancel while it runs and
 *  View report once it has one. Laid out like an export row. */
function OperateRow({ j }: { j: Job }) {
  const { openSheet } = useShell();
  const auth = useAuthKind();
  const [cancelling, setCancelling] = useState(false);
  const running = !j.finished;
  const failed = j.error != null && !neutralEnd(j.error.kind);
  const ended = j.finished ? endedText(j) : null;
  const outcome = j.finished && !endKind(j) ? outcomeText(j) : null;
  const pct = running && j.total ? Math.min(100, (j.done / j.total) * 100) : 0;
  const cancel = () => {
    setCancelling(true);
    commands.jobCancel(j.id).catch(() => setCancelling(false));
  };
  return (
    <li className="grid grid-cols-1 gap-y-1.5 px-3.5 py-2 text-12 @[560px]:grid-cols-[minmax(200px,1fr)_minmax(120px,200px)_max-content] @[560px]:items-start @[560px]:gap-x-4">
      <span className="min-w-0">
        <span className="block truncate">{j.title}</span>
        {outcome && (
          <span className="flex items-center gap-1.5 text-11 text-muted">
            {outcome.failed && <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />}
            <span>{outcome.text}</span>
          </span>
        )}
        {ended && <span className="block text-11 text-muted">{ended}</span>}
        {failed && j.error && (
          <span className="mt-1 flex flex-col gap-0.5">
            <span className="flex items-center gap-1.5 text-12 font-semibold text-fg">
              <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />
              {failedTitle(j.kind)}
            </span>
            <span className="text-11 text-muted">{adviceFor(j.error.kind, j.error.message, rightFor(j.kind), auth)}</span>
            <span className="min-w-0 break-words font-mono text-11 text-fg" data-selectable>
              {j.error.message}
            </span>
          </span>
        )}
      </span>
      {running && j.total != null ? (
        <span role="progressbar" aria-label={j.title} aria-valuemin={0} aria-valuemax={j.total} aria-valuenow={Math.min(j.done, j.total)} className="block h-[3px] bg-line @[560px]:mt-[7px]">
          <span className="block h-[3px] bg-fg" style={{ width: `${pct}%` }} />
        </span>
      ) : running ? (
        <span aria-hidden="true" className="block h-[3px] bg-line @[560px]:mt-[7px]" />
      ) : (
        <span aria-hidden="true" className="hidden @[560px]:block" />
      )}
      <span className="flex items-center justify-between gap-3 @[560px]:justify-end">
        <span className="min-w-[96px] whitespace-nowrap text-left text-11 tabular-nums text-muted @[560px]:text-right">{running ? runningText(j) : finishedState(j)}</span>
        {running && j.confirm && (
          <Button size="sm" className="h-[18px]" onClick={() => openSheet({ kind: "job", jobId: j.id })} aria-label={`Confirm…: ${j.title}`}>
            Confirm…
          </Button>
        )}
        {running && (
          <Button size="sm" className="h-[18px]" disabled={cancelling} onClick={cancel} aria-label={`${cancelling ? "Cancelling" : "Cancel"} ${j.title}`}>
            {cancelling ? "Cancelling" : "Cancel"}
          </Button>
        )}
        {j.finished && j.report && (
          <Button size="sm" className="h-[18px]" onClick={() => openSheet({ kind: "report", jobId: j.id })} aria-label={`View report: ${j.title}`}>
            View report
          </Button>
        )}
      </span>
    </li>
  );
}

function JobRow({ j }: { j: Job }) {
  return j.kind === "export" ? <ExportRow j={j} /> : <OperateRow j={j} />;
}

function JobsList() {
  const jobs = useJobs();
  if (!jobs.length)
    return <EmptyState title="No jobs" body="Exports and bulk operations show their progress here, and keep running while you work." />;
  return (
    <div className="@container">
      <ul className="m-0 list-none p-0">
        {jobs.map((j) => (
          <JobRow key={j.id} j={j} />
        ))}
      </ul>
    </div>
  );
}

export function Drawer() {
  const { shell, dispatch } = useShell();
  const jobs = useJobs();
  const running = jobs.filter((j) => !j.finished).length;
  return (
    <section aria-label="Jobs and activity" className="flex h-full min-h-0 flex-col">
      <Tabs value={shell.layout.drawerTab} onValueChange={(v) => dispatch({ type: "layout", patch: { drawerTab: v as "jobs" | "activity" } })} className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-[26px] shrink-0 items-stretch border-b border-line pr-1">
          <TabsList variant="underline" aria-label="Drawer" className="h-full px-1.5 [--tabs-fade:var(--c-side)]">
            <TabsTrigger value="jobs">Jobs {running > 0 && <span className="tabular-nums text-muted">{running}</span>}</TabsTrigger>
            <TabsTrigger value="activity">Activity</TabsTrigger>
          </TabsList>
          <div className="flex-1" />
          {shell.layout.drawerTab === "jobs" && jobs.some((j) => j.finished) && (
            <Button size="sm" variant="quiet" className="my-auto mr-1" onClick={() => jobStore.clearFinished()}>
              Clear finished
            </Button>
          )}
          <Hint label="Hide drawer" shortcut="mod+j">
            <IconButton label="Hide drawer" className="my-auto size-5" onClick={() => dispatch({ type: "layout", patch: { drawerOpen: false } })}>
              <ChevronDown />
            </IconButton>
          </Hint>
        </div>
        <TabsContent value="jobs" className="overflow-y-auto"><JobsList /></TabsContent>
        <TabsContent value="activity"><ActivityLog /></TabsContent>
      </Tabs>
    </section>
  );
}
