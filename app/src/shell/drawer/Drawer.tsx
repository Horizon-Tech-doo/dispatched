import { ChevronDown, CircleAlert } from "lucide-react";
import { useLayoutEffect, useRef } from "react";
import { EmptyState, Hint, IconButton, Tabs, TabsContent, TabsList, TabsTrigger, formatClock, formatDuration, formatNumber } from "@/ui";
import { useShell } from "../context";
import { useActivity, useJobs } from "./activity";

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

function JobsList() {
  const jobs = useJobs();
  if (!jobs.length)
    return <EmptyState title="No jobs" body="Exports and bulk operations show their progress here, and keep running while you work." />;
  return (
    <ul className="m-0 list-none p-0">
      {jobs.map((j) => {
        const cancelled = j.error?.kind === "cancelled";
        const failed = j.error != null && !cancelled;
        // Full only when it finished without an error, even if it ended short of its total; otherwise what was done.
        const pct = j.finished && !j.error ? 100 : j.total ? Math.min(100, (j.done / j.total) * 100) : 0;
        const counts = `${formatNumber(j.done)}${j.total ? ` / ${formatNumber(j.total)}` : ""}`;
        return (
          <li key={j.id} className="grid grid-cols-[minmax(200px,1fr)_200px_max-content] items-start gap-x-4 px-3.5 py-2 text-12">
            <span className="min-w-0">
              <span className="block truncate">{j.title}</span>
              {failed && j.error && (
                <span className="mt-0.5 flex items-start gap-1.5 font-mono text-11 text-fg">
                  <CircleAlert aria-hidden="true" className="mt-0.5 size-3 shrink-0 text-accent" />
                  <span className="min-w-0 break-words">{j.error.message}</span>
                </span>
              )}
            </span>
            <span className="mt-[7px] block h-[3px] bg-line" aria-hidden="true">
              <span className="block h-[3px] bg-fg" style={{ width: `${pct}%` }} />
            </span>
            <span className="min-w-[96px] whitespace-nowrap text-right font-mono text-11 tabular-nums text-muted">
              {cancelled ? "Cancelled" : failed ? counts : j.finished ? "Done" : counts}
            </span>
          </li>
        );
      })}
    </ul>
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
          <TabsList variant="underline" aria-label="Drawer" className="h-full px-1.5">
            <TabsTrigger value="jobs">Jobs {running > 0 && <span className="tabular-nums text-muted">{running}</span>}</TabsTrigger>
            <TabsTrigger value="activity">Activity</TabsTrigger>
          </TabsList>
          <div className="flex-1" />
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
