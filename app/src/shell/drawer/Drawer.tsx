import { ChevronDown, CircleAlert } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { commands } from "@/engine";
import { Button, EmptyState, Hint, IconButton, Tabs, TabsContent, TabsList, TabsTrigger, fileManagerName, formatClock, formatDuration, formatNumber } from "@/ui";
import { useShell } from "../context";
import { jobStore, useActivity, useJobs, type Job } from "./activity";

function ActivityLog() {
  const entries = useActivity();
  const box = useRef<HTMLDivElement>(null);
  // Whether the user was at the bottom before the latest rows rendered; the scroll handler keeps it current.
  const atBottom = useRef(true);
  const newest = entries.at(-1)?.id;
  const toEnd = () => {
    const el = box.current;
    if (el) el.scrollTop = el.scrollHeight;
  };
  // Opening the tab (or the drawer) starts at the newest entry.
  useLayoutEffect(toEnd, []);
  // New rows are followed only when the user was already at the bottom. Keyed on the newest id, not the
  // length, which stops changing once the cap trims old rows.
  useLayoutEffect(() => {
    if (newest === undefined) atBottom.current = true; // a cleared log starts following again
    else if (atBottom.current) toEnd();
  }, [newest]);
  if (!entries.length)
    return <EmptyState title="No activity yet" body="Every call to Service Bus appears here: what ran, how long it took, and why anything failed." />;
  return (
    <div
      ref={box}
      onScroll={(e) => {
        const el = e.currentTarget;
        atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
      className="h-full overflow-y-auto py-1 font-mono text-11 tabular-nums"
      data-selectable
    >
      {entries.map((e) => {
        // A cancelled operation is the user's choice, not a failure: neutral line, no alert mark.
        const cancelled = e.error?.kind === "cancelled";
        const failed = e.error != null && !cancelled;
        return (
          <div key={e.id} className="grid grid-cols-[64px_minmax(0,1fr)_72px] gap-x-3 px-3.5 leading-[18px]">
            <span className="text-faint">{formatClock(e.at)}</span>
            <span className="min-w-0">
              <span className={failed ? "text-fg" : "text-muted"}>{e.action}</span>
              {cancelled && <span className="text-faint"> cancelled</span>}
              {failed && e.error && (
                <span className="flex items-start gap-1.5 text-fg">
                  <CircleAlert aria-hidden="true" className="mt-0.5 size-3 shrink-0 text-accent" />
                  <span>{e.error.message}</span>
                </span>
              )}
            </span>
            <span className="text-right text-faint">{formatDuration(e.duration_ms)}</span>
          </div>
        );
      })}
    </div>
  );
}

const plural = (n: number, one: string) => `${formatNumber(n)} ${n === 1 ? one : `${one}s`}`;

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
function JobRow({ j }: { j: Job }) {
  const { openSheet } = useShell();
  const [cancelling, setCancelling] = useState(false);
  const [revealFailed, setRevealFailed] = useState(false);
  const cancelled = j.error?.kind === "cancelled";
  const failed = j.error != null && !cancelled;
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
  const state = running ? counts(j) : cancelled ? "Cancelled" : failed ? counts(j) : "Done";
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
            {j.done > 0 ? `Cancelled after ${j.total != null ? counts(j) : `${formatNumber(j.done)} written`}.` : "Cancelled."} No file was written.
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
        <span
          {...(running && j.total == null ? { role: "status" } : {})}
          className="min-w-[96px] whitespace-nowrap text-left text-11 tabular-nums text-muted @[560px]:text-right"
        >
          {state}
        </span>
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

/** What a screen reader hears when a job ends. */
function finishedMessage(j: Job): string {
  if (j.error?.kind === "cancelled") return `${j.title} was cancelled.`;
  if (j.error) return `${j.title} failed.`;
  return j.written != null ? `${j.title} finished: ${plural(j.written, "message")} written.` : `${j.title} finished.`;
}

/** A polite live region that says each job's ending once; jobs already finished when the drawer opened are not announced. */
function JobAnnouncer() {
  const jobs = useJobs();
  const [said, setSaid] = useState("");
  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null) seen.current = new Set(jobs.filter((j) => j.finished).map((j) => j.id));
  useEffect(() => {
    const ended = jobs.filter((j) => j.finished && !seen.current!.has(j.id));
    if (!ended.length) return;
    for (const j of ended) seen.current!.add(j.id);
    setSaid(finishedMessage(ended[ended.length - 1]));
  }, [jobs]);
  return (
    <p role="status" aria-label="Job updates" className="sr-only">
      {said}
    </p>
  );
}

export function Drawer() {
  const { shell, dispatch } = useShell();
  const jobs = useJobs();
  const running = jobs.filter((j) => !j.finished).length;
  return (
    <section aria-label="Jobs and activity" className="flex h-full min-h-0 flex-col">
      <JobAnnouncer />
      <Tabs value={shell.layout.drawerTab} onValueChange={(v) => dispatch({ type: "layout", patch: { drawerTab: v as "jobs" | "activity" } })} className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-[26px] shrink-0 items-stretch border-b border-line pr-1">
          <TabsList variant="underline" aria-label="Drawer" className="h-full px-1.5">
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
