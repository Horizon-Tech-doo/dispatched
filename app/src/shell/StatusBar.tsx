import { CircleAlert } from "lucide-react";
import { keys, useAppInfo } from "@/engine";
import { Button, formatClock } from "@/ui";
import { useShell } from "./context";
import { useUpdate } from "./sheets/useUpdate";
import { useJobs } from "./drawer/activity";
import { useIsFetching, useQueryState } from "./queryState";
import { troubleLabel, useConnectionTrouble } from "./trouble";

export function StatusBar() {
  const { conn, shell, live, dispatch, openSheet, notice } = useShell();
  const update = useUpdate().data;
  const info = useAppInfo().data;
  const running = useJobs().filter((j) => !j.finished).length;
  const entities = useQueryState(keys.entities(conn.status === "connected" ? conn.profileId : null));
  const fetching = useIsFetching({ queryKey: ["entities"] }) > 0;
  const trouble = useConnectionTrouble();
  const state =
    conn.status === "connecting" ? "Connecting…"
    : trouble ? troubleLabel[trouble]
    : conn.status === "connected" ? "Peek only, nothing locked"
    : "Not connected";
  const editConnection = () => conn.profileId && openSheet({ kind: "profile", profileId: conn.profileId });
  return (
    <footer className="flex h-[22px] shrink-0 items-center gap-4 border-t border-line bg-chrome px-3 text-11 text-muted">
      {conn.view && <span className="shrink-0">{conn.view.transport === "websockets" ? "AMQP over WebSockets" : "AMQP"}</span>}
      <span className="flex shrink-0 items-center gap-1.5">
        {trouble && <CircleAlert aria-hidden="true" className="size-3 text-accent" />}
        <span role="status" className={trouble ? "text-fg" : undefined}>{state}</span>
        {trouble === "lost" && <Button size="sm" className="h-[18px]" onClick={conn.retry}>Reconnect</Button>}
        {trouble && trouble !== "lost" && <Button size="sm" className="h-[18px]" onClick={editConnection}>Edit connection</Button>}
      </span>
      {notice && (
        <span role="status" className="flex min-w-0 items-center gap-1.5 text-fg" title={notice.text}>
          {notice.error && <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />}
          <span className="truncate">{notice.text}</span>
          {notice.details && (
            <Button size="sm" className="h-[18px] shrink-0" onClick={() => dispatch({ type: "layout", patch: { drawerOpen: true, drawerTab: "activity" } })}>
              Details
            </Button>
          )}
        </span>
      )}
      {live && !shell.layout.drawerOpen && (
        <Button size="sm" variant="quiet" className="h-[18px] shrink-0" onClick={() => dispatch({ type: "layout", patch: { drawerOpen: true } })}>
          Show activity{running > 0 && <span className="tabular-nums"> · {running} running</span>}
        </Button>
      )}
      {info?.startup_error && (
        <span className="flex min-w-0 items-center gap-1.5 text-fg" title={info.startup_error}>
          <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />
          <span className="truncate">{info.startup_error}</span>
        </span>
      )}
      <div className="flex-1" />
      {update && (
        <Button size="sm" variant="quiet" className="h-[18px] shrink-0" onClick={() => openSheet({ kind: "update" })}>Update {update.version} available</Button>
      )}
      {conn.status === "connected" && entities.dataUpdatedAt > 0 && (
        <span className="shrink-0 tabular-nums">{fetching ? "Refreshing…" : `Refreshed ${formatClock(new Date(entities.dataUpdatedAt).toISOString())}`}</span>
      )}
    </footer>
  );
}
