import { CircleAlert } from "lucide-react";
import { keys, useAppInfo } from "@/engine";
import { Button, formatClock } from "@/ui";
import { useShell } from "./context";
import { useIsFetching, useQueryState } from "./queryState";

const LOST = new Set(["network", "timeout", "auth", "not_signed_in"]);

export function StatusBar() {
  const { conn, shell, dispatch } = useShell();
  const info = useAppInfo().data;
  const entities = useQueryState(keys.entities(conn.status === "connected" ? conn.profileId : null));
  const fetching = useIsFetching({ queryKey: ["entities"] }) > 0;
  const lost = conn.status === "connected" && entities.error != null && LOST.has(entities.error.kind);
  const state =
    conn.status === "connecting" ? "Connecting…"
    : lost ? "Connection lost"
    : conn.status === "connected" ? "Peek only, nothing locked"
    : "Not connected";
  return (
    <footer className="flex h-[22px] shrink-0 items-center gap-4 border-t border-line bg-chrome px-3 text-11 text-muted">
      {conn.view && <span>{conn.view.transport === "websockets" ? "AMQP over WebSockets" : "AMQP"}</span>}
      <span className="flex items-center gap-1.5">
        {lost && <CircleAlert aria-hidden="true" className="size-3 text-accent" />}
        <span role="status" className={lost ? "text-fg" : undefined}>{state}</span>
        {lost && <Button size="sm" variant="quiet" onClick={conn.retry}>Reconnect</Button>}
      </span>
      {conn.status === "connected" && !shell.layout.drawerOpen && (
        <Button size="sm" variant="quiet" onClick={() => dispatch({ type: "layout", patch: { drawerOpen: true } })}>Show activity</Button>
      )}
      {info?.startup_error && (
        <span className="flex items-center gap-1.5 text-fg">
          <CircleAlert aria-hidden="true" className="size-3 text-accent" />
          {info.startup_error}
        </span>
      )}
      <div className="flex-1" />
      {conn.status === "connected" && entities.dataUpdatedAt > 0 && (
        <span className="tabular-nums">{fetching ? "Refreshing…" : `Refreshed ${formatClock(new Date(entities.dataUpdatedAt).toISOString())}`}</span>
      )}
    </footer>
  );
}
