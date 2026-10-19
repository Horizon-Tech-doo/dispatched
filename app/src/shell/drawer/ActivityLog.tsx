import { CircleAlert } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { commands, type ActivityOrigin, type ActivityRecord } from "@/engine";
import { Badge, Button, EmptyState, Segmented, ToggleGroup, formatClock, formatDuration, formatNumber } from "@/ui";
import { useShell } from "../context";
import { useActivity, useRecords, type LoggedEntry } from "./activity";

type Origin = "all" | ActivityOrigin;
type Toggle = "connection" | "failed";

/** A line of the log: this window's own call (not persisted), or an entry of the shared log. */
type Line = { key: string; at: string; session: LoggedEntry } | { key: string; at: string; record: ActivityRecord };

const ORIGIN_TAG: Record<ActivityOrigin, string> = { gui: "GUI", cli: "CLI", mcp: "MCP" };
/** How a change ended, when it is not plain success. */
const ENDED: Partial<Record<ActivityRecord["outcome"], string>> = {
  pending: "running", "partly-failed": "partly failed", failed: "failed", cancelled: "cancelled", denied: "not confirmed",
};

/** The shared entries worth a line: every finished entry, and a started one only until its finish is known. */
function shown(records: ActivityRecord[]): ActivityRecord[] {
  const finished = new Set(records.filter((r) => r.phase === "finished").map((r) => r.op_id));
  return records.filter((r) => r.phase === "finished" || !finished.has(r.op_id));
}

const failedSession = (e: LoggedEntry) => e.error != null && e.error.kind !== "cancelled" && e.error.kind !== "stopped";
const failedRecord = (r: ActivityRecord) => r.outcome === "failed" || r.outcome === "partly-failed";

function SessionLine({ e }: { e: LoggedEntry }) {
  // A cancelled operation is the user's choice, not a failure: neutral line, no alert mark. So is a job stopped
  // because its connection closed.
  const cancelled = e.error?.kind === "cancelled";
  const failed = failedSession(e);
  return (
    <>
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
    </>
  );
}

function RecordLine({ r }: { r: ActivityRecord }) {
  const failed = failedRecord(r);
  const ended = ENDED[r.outcome];
  const extra = [r.failed ? `${formatNumber(r.failed)} failed` : null, r.skipped ? `${formatNumber(r.skipped)} skipped` : null].filter(Boolean);
  return (
    <>
      <span className="min-w-0">
        <span className="flex min-w-0 items-start gap-1.5">
          <Badge className="mt-px shrink-0 font-sans">{ORIGIN_TAG[r.origin]}</Badge>
          <span className="min-w-0">
            <span className={failed ? "text-fg" : "text-muted"}>{r.summary}</span>
            {extra.length > 0 && r.outcome !== "failed" && <span className="text-faint"> · {extra.join(", ")}</span>}
            {ended && <span className="text-faint"> {ended}</span>}
            <span className="text-faint"> · {r.profile}</span>
          </span>
        </span>
        {failed && r.error && (
          <span className="flex items-start gap-1.5 text-fg">
            <CircleAlert aria-hidden="true" className="mt-0.5 size-3 shrink-0 text-accent" />
            <span>{r.error}</span>
          </span>
        )}
        {r.note && <span className="block text-faint">{r.note}</span>}
      </span>
      <span />
    </>
  );
}

/**
 * The Activity tab: this window's calls to Service Bus (reads are not persisted), merged by time with the shared log of
 * changes (`activity.db`) that this app, the CLI and agents write. Persisted lines carry their origin. Filters: origin,
 * failures only, this connection only. Export… writes the shared log's matching entries to a file.
 */
export function ActivityLog() {
  const { conn } = useShell();
  const entries = useActivity();
  const records = useRecords();
  const [origin, setOrigin] = useState<Origin>("all");
  const [toggles, setToggles] = useState<Toggle[]>([]);
  const [exported, setExported] = useState<{ ok: true; text: string } | { ok: false; text: string } | null>(null);
  const profileName = conn.view?.profile_name ?? null;
  const onlyConnection = toggles.includes("connection");
  const onlyFailed = toggles.includes("failed");

  const lines = useMemo<Line[]>(() => {
    const out: Line[] = [];
    // This window's own lines are GUI calls without a profile: shown unless another origin or connection is asked for.
    if ((origin === "all" || origin === "gui") && !onlyConnection)
      for (const e of entries) if (!onlyFailed || failedSession(e)) out.push({ key: `s${e.id}`, at: e.at, session: e });
    for (const r of shown(records)) {
      if (origin !== "all" && r.origin !== origin) continue;
      if (onlyConnection && r.profile !== profileName) continue;
      if (onlyFailed && !failedRecord(r)) continue;
      out.push({ key: `r${r.id}`, at: r.at, record: r });
    }
    // By time; a stable sort keeps each source's own order for equal times.
    return out.sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
  }, [entries, records, origin, onlyConnection, onlyFailed, profileName]);

  const box = useRef<HTMLDivElement>(null);
  // Whether the user was at the bottom before the latest rows rendered; the scroll handler keeps it current.
  const atBottom = useRef(true);
  const newest = lines.length ? `${entries.at(-1)?.id}|${records.at(-1)?.id}` : undefined;
  const toEnd = () => {
    const el = box.current;
    if (el) el.scrollTop = el.scrollHeight;
  };
  // Opening the tab (or the drawer) starts at the newest entry.
  useLayoutEffect(toEnd, []);
  // New rows are followed only when the user was already at the bottom. Keyed on the newest entry of each source, not
  // the length, which stops changing once the cap trims old rows.
  useLayoutEffect(() => {
    if (newest === undefined) atBottom.current = true; // a cleared log starts following again
    else if (atBottom.current) toEnd();
  }, [newest]);

  const exportLog = async () => {
    setExported(null);
    const query = { ...(origin !== "all" ? { origin } : {}), ...(onlyConnection && profileName ? { profile: profileName } : {}), ...(onlyFailed ? { outcome: "failed" as const } : {}) };
    try {
      const pick = await commands.pickActivityExportPath("json");
      if (!pick) return;
      const n = await commands.activityExport(query, pick.pick, "json");
      setExported({ ok: true, text: `Exported ${formatNumber(n)} ${n === 1 ? "entry" : "entries"} to ${pick.name}` });
    } catch (e) {
      setExported({ ok: false, text: e instanceof Error ? e.message : String(e) });
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div role="toolbar" aria-label="Activity filters" className="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-3.5 py-1">
        <Segmented
          label="Origin"
          value={origin}
          onChange={setOrigin}
          options={[
            { value: "all", label: "All" },
            { value: "gui", label: "GUI" },
            { value: "cli", label: "CLI" },
            { value: "mcp", label: "MCP" },
          ]}
        />
        <ToggleGroup
          label="Show"
          values={toggles}
          onChange={setToggles}
          options={[
            { value: "connection", label: "This connection only" },
            { value: "failed", label: "Failed only" },
          ]}
        />
        <span className="flex-1" />
        {exported && (
          <span role={exported.ok ? undefined : "alert"} className="flex min-w-0 items-center gap-1.5 text-11 text-muted">
            {!exported.ok && <CircleAlert aria-hidden="true" className="size-3 shrink-0 text-accent" />}
            <span className="truncate">{exported.ok ? exported.text : `Couldn't export: ${exported.text}`}</span>
          </span>
        )}
        <Button size="sm" variant="quiet" onClick={() => void exportLog()}>
          Export…
        </Button>
      </div>
      {!lines.length ? (
        entries.length || records.length ? (
          <EmptyState title="Nothing matches these filters" body="Every change made here, in the CLI or by an agent is kept in the shared activity log." />
        ) : (
          <EmptyState title="No activity yet" body="Every call to Service Bus appears here: what ran, how long it took, and why anything failed." />
        )
      ) : (
        <div
          ref={box}
          onScroll={(e) => {
            const el = e.currentTarget;
            atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
          }}
          className="min-h-0 flex-1 overflow-y-auto py-1 font-mono text-11 tabular-nums"
          data-selectable
        >
          {lines.map((l) => (
            <div key={l.key} data-activity-row className="grid grid-cols-[64px_minmax(0,1fr)_72px] gap-x-3 px-3.5 leading-[18px]">
              <span className="text-faint">{formatClock(l.at)}</span>
              {"session" in l ? <SessionLine e={l.session} /> : <RecordLine r={l.record} />}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
