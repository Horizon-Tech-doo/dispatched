import { CircleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { installUpdate, type EngineError } from "@/engine";
import { Button, EmptyState, Sheet } from "@/ui";
import { useShell } from "../context";
import { useUpdate } from "./useUpdate";

/** The updater reports dates like "2026-10-20 10:00:00.0 +00:00:00"; the day is all a reader needs. */
const day = (date: string) => (/^\d{4}-\d{2}-\d{2}/.test(date) ? date.slice(0, 10) : date);

export function UpdateSheet() {
  const { closeSheet } = useShell();
  const update = useUpdate({ onDemand: true });
  // undefined: not installing, null: downloading with no known size, number: percent.
  const [progress, setProgress] = useState<number | null | undefined>(undefined);
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const u = update.data;
  const installing = progress !== undefined;

  // Opening the sheet checks now; it must not wait for the start-up delay.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { void update.refetch(); }, []);

  async function install() {
    setError(null);
    setProgress(null);
    try {
      await installUpdate(setProgress);
      setRestarting(true);
    } catch (e) {
      setProgress(undefined);
      setError((e as EngineError).message ?? String(e));
    }
  }

  const title = u
    ? "Update available"
    : update.isFetching ? "Checking for updates"
    : update.error ? "Update check failed"
    : "Dispatched is up to date";
  const installLabel = restarting ? "Restarting…" : installing ? (progress == null ? "Downloading…" : `Downloading ${progress}%`) : "Install and restart";
  return (
    <Sheet
      open
      onOpenChange={(o) => !o && !installing && closeSheet()}
      title={title}
      footer={
        u ? (
          <>
            <Button variant="quiet" onClick={closeSheet} disabled={installing}>Later</Button>
            <Button variant="primary" onClick={() => void install()} disabled={installing}>{installLabel}</Button>
          </>
        ) : (
          <Button onClick={() => void update.refetch()} disabled={update.isFetching}>Check again</Button>
        )
      }
    >
      {u ? (
        <div className="flex flex-col gap-2">
          <p className="m-0 text-13">
            <span className="font-semibold">Version {u.version}</span>
            {u.date && <span className="text-muted">, released {day(u.date)}</span>}
          </p>
          {u.notes && <p className="m-0 whitespace-pre-wrap text-12">{u.notes}</p>}
          {installing && <p className="m-0 text-12 text-muted">Dispatched restarts when the download finishes.</p>}
          {error && (
            <div className="flex items-start gap-1.5 text-12" role="alert">
              <CircleAlert aria-hidden="true" className="mt-px size-3.5 shrink-0 text-accent" />
              <div className="flex flex-col">
                <span className="font-semibold">Couldn't install the update.</span>
                <span>{error}</span>
              </div>
            </div>
          )}
        </div>
      ) : update.error ? (
        <EmptyState tone="error" title="Couldn't check for updates" body={(update.error as Error).message} />
      ) : (
        !update.isFetching && <p className="m-0 text-12 text-muted">You have the latest version.</p>
      )}
    </Sheet>
  );
}
