import { useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useEffect, useRef, useState, type Dispatch, type ReactNode } from "react";
import { commands, type ExportFormat, type FilterSpec, type PeekView } from "@/engine";
import { useTheme } from "@/ui";
import { startEventCapture } from "./drawer/activity";
import { useConnectionController, type ConnectionController } from "./connection";
import { clearViewStates } from "./messages/viewStore";
import { installStore } from "./sheets/installState";
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
  | null;

/** The outcome of the last "copy connection string": the string itself never reaches the webview, only whether it worked. */
export type CopyNotice = { entity: string; failure?: string } | null;
const COPIED_MS = 3000;

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
  /** Asks the engine to put the entity's connection string on the clipboard; the result is in `copyNotice`. */
  copyConnectionString: (entity: string) => Promise<void>;
  copyNotice: CopyNotice;
  dismissCopyNotice: () => void;
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
  const [copyNotice, setCopyNotice] = useState<CopyNotice>(null);
  const dismissCopyNotice = useCallback(() => setCopyNotice(null), []);
  useEffect(() => {
    if (!copyNotice || copyNotice.failure) return;
    const t = setTimeout(() => setCopyNotice(null), COPIED_MS);
    return () => clearTimeout(t);
  }, [copyNotice]);
  // A notice is about one connection's entity.
  useEffect(() => setCopyNotice(null), [shellProfileId]);
  const copyConnectionString = useCallback(
    async (entity: string) => {
      if (!conn.profileId) return;
      setCopyNotice(null);
      try {
        await commands.copyEntityConnectionString(conn.profileId, entity);
        setCopyNotice({ entity });
      } catch (e) {
        setCopyNotice({ entity, failure: String((e as { message?: string } | null)?.message ?? e) });
      }
    },
    [conn.profileId],
  );
  const qc = useQueryClient();
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
        prefs, setPrefs, conn, shell, dispatch, shellLoaded, shellProfileId, live, sheet,
        // While an update installs, its sheet stays put: other sheets would hide the progress.
        openSheet: (s) => { if (!installStore.busy() || s.kind === "update") setSheet(s); },
        closeSheet: () => setSheet(null),
        paletteOpen, setPaletteOpen, refresh, copyConnectionString, copyNotice, dismissCopyNotice,
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
