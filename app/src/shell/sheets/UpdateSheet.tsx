import { useEffect } from "react";
import { useAppInfo } from "@/engine";
import { Button, Problem, Sheet, formatClock } from "@/ui";
import { useShell } from "../context";
import { useJobs } from "../drawer/activity";
import { installStore, reasonOf, useInstall } from "./installState";
import { useUpdate } from "./useUpdate";

/** The updater reports dates like "2026-10-20 10:00:00.0 +00:00:00"; the day is all a reader needs. */
const day = (date: string) => (/^\d{4}-\d{2}-\d{2}/.test(date) ? date.slice(0, 10) : date);

/** A 2px bar. `value` null: size unknown, so a full-width static bar at half strength. */
function Bar({ value }: { value: number | null }) {
  return (
    <div
      role="progressbar"
      aria-label="Download progress"
      aria-valuemin={0}
      aria-valuemax={100}
      {...(value == null ? {} : { "aria-valuenow": value })}
      className="h-0.5 w-full bg-line"
    >
      <div className={value == null ? "h-full w-full bg-fg opacity-40" : "h-full bg-fg"} style={value == null ? undefined : { width: `${value}%` }} />
    </div>
  );
}

export function UpdateSheet() {
  const { closeSheet } = useShell();
  const update = useUpdate({ onDemand: true });
  const install = useInstall();
  const version = useAppInfo().data?.version;
  const running = useJobs().filter((j) => !j.finished).length;
  const u = update.data;
  const checking = update.isFetching;
  const installing = install.phase !== "idle";
  const checkError = !checking && update.error ? reasonOf(update.error) : null;

  // Opening the sheet checks now (it must not wait for the start-up delay) and forgets an older install failure.
  // An install in progress is left alone.
  useEffect(() => {
    if (installStore.busy()) return;
    installStore.reset();
    void update.refetch();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const status =
    install.phase === "downloading" ? (install.progress == null ? "Downloading…" : `Downloading ${install.progress}%`)
    : install.phase === "installing" ? "Installing…"
    : install.phase === "restarting" ? "Restarting…"
    : null;
  const label = install.phase === "idle" ? (install.error ? "Try again" : "Install and restart") : install.phase === "downloading" ? "Downloading…" : status;

  return (
    <Sheet
      open
      onOpenChange={(o) => !o && !installing && closeSheet()}
      dismissible={!installing}
      title="Updates"
      footer={
        <div className="flex w-full flex-col gap-2">
          {u && running > 0 && (
            <p className="m-0 text-11 text-muted">
              {running} {running === 1 ? "job" : "jobs"} running will stop when Dispatched restarts.
            </p>
          )}
          <div className="flex justify-end gap-2">
            {u ? (
              <>
                {checkError && <Button onClick={() => void update.refetch()} disabled={installing}>Check again</Button>}
                <Button variant="quiet" onClick={closeSheet} disabled={installing}>Later</Button>
                {/* Retrying a failed install is a recovery action: bordered, not primary. */}
                <Button variant={install.error ? "default" : "primary"} onClick={() => void installStore.start()} disabled={installing}>{label}</Button>
              </>
            ) : (
              <Button onClick={() => void update.refetch()} disabled={checking}>Check again</Button>
            )}
          </div>
        </div>
      }
    >
      <div className="flex flex-col gap-3">
        {checking && !u && <p role="status" className="m-0 text-12 text-muted">Checking for updates…</p>}
        {checkError && (
          <Problem role="alert" title="Couldn't check for updates" body="Check your network connection, then check again." detail={checkError} />
        )}
        {u && (
          <div className="flex flex-col gap-2">
            <p className="m-0 text-13">
              <span className="font-semibold">Version {u.version}</span>
              {u.date && <span className="text-muted">, released {day(u.date)}</span>}
            </p>
            {version && <p className="m-0 text-12 text-muted">You have {version}.</p>}
            {u.notes && <p className="m-0 whitespace-pre-wrap text-12">{u.notes}</p>}
          </div>
        )}
        {installing && (
          <div className="flex flex-col gap-1.5">
            {install.phase === "downloading" && <Bar value={install.progress} />}
            <p role="status" className="m-0 text-12 text-muted">{status}</p>
          </div>
        )}
        {install.error && (
          <Problem role="alert" title="Couldn't install the update" body="Try again now, or later from Settings." detail={install.error} />
        )}
        {!u && !checking && !update.error && (
          <p className="m-0 text-12 text-muted">
            Dispatched{version && ` ${version}`}.{update.dataUpdatedAt > 0 && ` Checked ${formatClock(new Date(update.dataUpdatedAt).toISOString())}.`}
          </p>
        )}
      </div>
    </Sheet>
  );
}
