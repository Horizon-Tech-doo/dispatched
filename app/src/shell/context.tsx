import { useQueryClient } from "@tanstack/react-query";
import { createContext, useCallback, useContext, useState, type Dispatch, type ReactNode } from "react";
import { useTheme } from "@/ui";
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
  sheet: SheetState;
  openSheet: (s: Exclude<SheetState, null>) => void;
  closeSheet: () => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  refresh: () => void;
}

const ShellContext = createContext<ShellContextValue | null>(null);

export function ShellProvider({ children }: { children: ReactNode }) {
  const [prefs, setPrefs] = usePrefs();
  useTheme(prefs.theme, prefs.density);
  const conn = useConnectionController();
  const [shell, dispatch, shellLoaded] = useProfileShell(conn.status === "connected" ? conn.profileId : null);
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
        prefs, setPrefs, conn, shell, dispatch, shellLoaded, sheet,
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
