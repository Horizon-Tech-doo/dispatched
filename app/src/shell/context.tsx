import { useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useEffect, useState, type Dispatch, type ReactNode } from "react";
import { useTheme } from "@/ui";
import { startEventCapture } from "./drawer/activity";
import { useConnectionController, type ConnectionController } from "./connection";
import { usePrefs, useProfileShell, type Prefs, type ProfileShell, type ShellAction } from "./state";

export type SheetState =
  | { kind: "profile"; profileId: string | null; auth?: "connection-string" | "entra" }
  | { kind: "profiles" }
  | { kind: "settings" }
  | { kind: "update" }
  | null;

interface ShellContextValue {
  prefs: Prefs;
  setPrefs: (patch: Partial<Prefs>) => void;
  conn: ConnectionController;
  shell: ProfileShell;
  dispatch: Dispatch<ShellAction>;
  shellLoaded: boolean;
  /** The profile the workbench is showing; stays set while that same profile reconnects. */
  shellProfileId: string | null;
  sheet: SheetState;
  openSheet: (s: Exclude<SheetState, null>) => void;
  closeSheet: () => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  refresh: () => void;
}

const ShellContext = createContext<ShellContextValue | null>(null);

export function ShellProvider({ children }: { children: ReactNode }) {
  useEffect(() => {
    let off: (() => void) | undefined;
    let live = true;
    void startEventCapture().then((f) => (live ? (off = f) : f()));
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
  const [shell, dispatch, shellLoaded] = useProfileShell(shellProfileId);
  const [sheet, setSheet] = useState<SheetState>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const qc = useQueryClient();
  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ["entities"] });
    void qc.invalidateQueries({ queryKey: ["subscriptions"] });
  }, [qc]);

  return (
    <ShellContext.Provider
      value={{
        prefs, setPrefs, conn, shell, dispatch, shellLoaded, shellProfileId, sheet,
        openSheet: setSheet, closeSheet: () => setSheet(null),
        paletteOpen, setPaletteOpen, refresh,
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
