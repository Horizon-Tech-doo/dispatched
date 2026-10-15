import { useShell } from "./context";
import { useHotkeys, type Binding } from "./keys";

/** The window's keyboard map. Renders nothing. */
export function Hotkeys() {
  const { conn, shell, dispatch, paletteOpen, setPaletteOpen, sheet, openSheet, refresh } = useShell();
  const connected = conn.status === "connected";
  // Behind the palette or a sheet the workbench is not the target: only the keys that act on the window itself apply.
  const free = connected && !paletteOpen && sheet === null;
  const bindings: Binding[] = [
    { combo: "mod+k", run: () => setPaletteOpen(!paletteOpen), inInputs: true },
    { combo: "mod+,", run: () => openSheet({ kind: "settings" }), inInputs: true },
    { combo: "f5", run: () => free && refresh(), inInputs: true },
    { combo: "mod+f", run: () => free && document.getElementById("tree-filter")?.focus() },
    { combo: "mod+j", run: () => free && dispatch({ type: "layout", patch: { drawerOpen: !shell.layout.drawerOpen } }) },
    { combo: "mod+w", run: () => free && shell.activeTab && dispatch({ type: "close", id: shell.activeTab }) },
    ...Array.from({ length: 9 }, (_, i) => ({ combo: `mod+${i + 1}`, run: () => free && dispatch({ type: "activateIndex", index: i }) })),
  ];
  useHotkeys(bindings);
  return null;
}
