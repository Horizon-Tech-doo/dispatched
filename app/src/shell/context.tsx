import { useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useEffect, useRef, useState, type Dispatch, type ReactNode } from "react";
import { commands, type ExportFormat, type FilterSpec, type PeekView } from "@/engine";
import { useTheme } from "@/ui";
import { recordStore, startEventCapture } from "./drawer/activity";
import { createExternalChanges } from "./drawer/externalChanges";
import { useConnectionController, type ConnectionController } from "./connection";
import { clearViewStates } from "./messages/viewStore";
import { connectionStringScope, displayName } from "./entityId";
import { installStore } from "./sheets/installState";
import type { OperationSeed } from "./operate/seed";
import { usePrefs, useProfileShell, type Prefs, type ProfileShell, type ShellAction } from "./state";

export type SheetState =
  | { kind: "profile"; profileId: string | null; auth?: "connection-string" | "entra" }
  | { kind: "profiles" }
  | { kind: "settings" }
  | { kind: "update" }
  | { kind: "explorer-options" }
  /** Export messages of one view: `selected` are the grid's selected sequence numbers (or one message's), `total` the
   *  view's listed count (null when unknown). `scope` and `format` start the sheet on those choices (a retry). */
  | { kind: "export"; entity: string; view: PeekView; filter: FilterSpec; selected: number[]; total: number | null; scope?: "all" | "selected"; format?: ExportFormat }
  /** An operation (send, delete, move, purge, …), with what it starts from. */
  | ({ kind: "operation" } & OperationSeed)
  /** A finished job's report. */
  | { kind: "report"; jobId: string }
  /** A running job whose sheet is gone, to answer the confirmation it waits for. */
  | { kind: "job"; jobId: string }
  | null;

/** A short line in the status bar about what the user just did (a copy). `details` offers the activity log, where a failed
 *  engine call is recorded. */
export type Notice = { text: string; error?: boolean; details?: boolean } | null;
const NOTICE_MS = 3000;
const ERROR_NOTICE_MS = 8000;
/** What a connection string copy came to. The string itself never reaches the webview: only whether it worked. */
export type CopyResult = { ok: true } | { ok: false; message: string };

interface ShellContextValue {
  prefs: Prefs;
  setPrefs: (patch: Partial<Prefs>) => void;
  conn: ConnectionController;
  shell: ProfileShell;
  dispatch: Dispatch<ShellAction>;
  shellLoaded: boolean;
  /** The profile the workbench is showing; stays set while that same profile reconnects. */
  shellProfileId: string | null;
  /** The workbench is up: connected, or reconnecting the profile it already shows. Welcome otherwise. */
  live: boolean;
  sheet: SheetState;
  openSheet: (s: Exclude<SheetState, null>) => void;
  closeSheet: () => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  refresh: () => void;
  /** The one path for copying text from the shell: writes the clipboard and says `message` in the status bar, or that it
   *  could not (unless `inline`: the caller shows the failure where the action was taken). */
  copyText: (text: string, message: string, opts?: { inline?: boolean }) => Promise<boolean>;
  /** Asks the engine to put the entity's connection string on the clipboard (Rust writes it; nothing comes back). The
   *  status bar says so; a result that arrives after the connection changed is dropped. */
  copyConnectionString: (entity: string, opts?: { inline?: boolean }) => Promise<CopyResult>;
  notice: Notice;
  dismissNotice: () => void;
  /** Says `n` in the status bar (what an action came to). */
  say: (n: Exclude<Notice, null>) => void;
}

const ShellContext = createContext<ShellContextValue | null>(null);

export function ShellProvider({ children }: { children: ReactNode }) {
  useEffect(() => {
    let off: (() => void) | undefined;
    let live = true;
    startEventCapture().then(
      (f) => (live ? (off = f) : f()),
      (e) => console.error("Could not subscribe to engine events", e),
    );
    return () => {
      live = false;
      off?.();
    };
  }, []);
  const [prefs, setPrefs] = usePrefs();
  useTheme(prefs.theme, prefs.density);
  const conn = useConnectionController();
  // The profile whose shell state is live: set when a connection succeeds, cleared on disconnect or a failed
  // connect, and unchanged while the same profile reconnects, so a reconnect keeps (and does not reload) its state.
  const [shellProfileId, setShellProfileId] = useState<string | null>(null);
  if (conn.status === "connected" && conn.profileId !== shellProfileId) setShellProfileId(conn.profileId);
  else if ((conn.status === "idle" || conn.status === "failed") && shellProfileId !== null) setShellProfileId(null);
  const live = conn.status === "connected" || (conn.status === "connecting" && shellProfileId !== null && conn.profileId === shellProfileId);
  const [shell, dispatch, shellLoaded] = useProfileShell(shellProfileId);
  // Leaving a connection forgets its views' state (Top N, From, filters, sorting): it was about that namespace's
  // messages. A reconnect of the same profile keeps it, as it keeps the tabs.
  const shownProfile = useRef(shellProfileId);
  useEffect(() => {
    const was = shownProfile.current;
    shownProfile.current = shellProfileId;
    if (was !== null && was !== shellProfileId) clearViewStates(was);
  }, [shellProfileId]);
  const [sheet, setSheet] = useState<SheetState>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);
  const dismissNotice = useCallback(() => setNotice(null), []);
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(null), notice.error ? ERROR_NOTICE_MS : NOTICE_MS);
    return () => clearTimeout(t);
  }, [notice]);
  // A notice is about one connection's entity.
  useEffect(() => setNotice(null), [shellProfileId]);
  const profileNow = useRef<string | null>(null);
  profileNow.current = conn.profileId;
  const copyText = useCallback(async (text: string, message: string, opts?: { inline?: boolean }) => {
    try {
      if (!navigator.clipboard) throw new Error("no clipboard");
      await navigator.clipboard.writeText(text);
      setNotice({ text: message });
      return true;
    } catch {
      if (!opts?.inline) setNotice({ text: "Couldn't copy to the clipboard", error: true });
      return false;
    }
  }, []);
  const copyConnectionString = useCallback(async (entity: string, opts?: { inline?: boolean }): Promise<CopyResult> => {
    const profileId = profileNow.current;
    if (!profileId) return { ok: false, message: "Not connected." };
    setNotice(null);
    try {
      await commands.copyEntityConnectionString(profileId, entity);
      // Rust has already put it on the clipboard; if the connection changed meanwhile, say nothing about the old one.
      if (profileNow.current === profileId) setNotice({ text: `Copied the connection string for ${displayName(entity)}. It is scoped to ${connectionStringScope(entity, "that entity")} and was never shown.` });
      return { ok: true };
    } catch (e) {
      const message = String((e as { message?: string } | null)?.message ?? e);
      if (!opts?.inline && profileNow.current === profileId) setNotice({ text: "Couldn't copy the connection string", error: true, details: true });
      return { ok: false, message };
    }
  }, []);
  const qc = useQueryClient();
  // The shared activity log: what it holds now, then (while connected) the changes the CLI and agents make.
  useEffect(() => {
    let live = true;
    commands.activityList({ limit: 500 }).then(
      (rs) => live && recordStore.merge([...rs].reverse()),
      () => {},
    );
    return () => {
      live = false;
    };
  }, []);
  const followed = useRef<{ id: string; name: string } | null>(null);
  followed.current = conn.status === "connected" && conn.view ? { id: conn.view.profile_id, name: conn.view.profile_name } : null;
  useEffect(() => {
    const x = createExternalChanges({ qc, profile: () => followed.current });
    void x.poll();
    return x.start();
  }, [qc]);
  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ["entities"] });
    void qc.invalidateQueries({ queryKey: ["subscriptions"] });
    // Peeks are reset, never invalidated: an invalidated peek would refetch every page it had, and the stream
    // starts a fresh round at Top N itself once its pages are gone.
    void qc.resetQueries({ queryKey: ["peek"] });
    for (const queryKey of [["entity-details"], ["rules"], ["sessions"], ["dashboard"], ["message"]]) void qc.invalidateQueries({ queryKey });
  }, [qc]);

  return (
    <ShellContext.Provider
      value={{
        prefs, setPrefs, conn, shell, dispatch, shellLoaded, shellProfileId, live, sheet, say: setNotice,
        // While an update installs, its sheet stays put: other sheets would hide the progress.
        openSheet: (s) => { if (!installStore.busy() || s.kind === "update") setSheet(s); },
        closeSheet: () => setSheet(null),
        paletteOpen, setPaletteOpen, refresh, copyText, copyConnectionString, notice, dismissNotice,
      }}
    >
      {children}
    </ShellContext.Provider>
  );
}

export function useShell(): ShellContextValue {
  const v = useContext(ShellContext);
  if (!v) throw new Error("useShell outside ShellProvider");
  return v;
}
