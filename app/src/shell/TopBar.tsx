import { RefreshCw, Search } from "lucide-react";
import { inTauri } from "@/engine";
import { Hint, IconButton, Kbd, isMac } from "@/ui";
import { ConnectionSelector } from "./ConnectionSelector";
import { useShell } from "./context";
import { paletteHint } from "./palette/commands";

export function TopBar() {
  const { conn, setPaletteOpen, refresh } = useShell();
  const macInset = isMac && inTauri();
  return (
    <header
      data-tauri-drag-region
      className="grid h-[38px] shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3 border-b border-line bg-chrome pr-2 max-[720px]:grid-cols-[minmax(0,1fr)_auto]"
      style={{ paddingLeft: macInset ? 84 : 10 }}
    >
      <div data-tauri-drag-region className="flex min-w-0 items-center gap-3 overflow-hidden">
        <ConnectionSelector />
        {conn.view && <span className="truncate text-12 text-faint max-[900px]:hidden">{conn.view.auth}</span>}
        <div data-tauri-drag-region className="h-full min-w-0 flex-1" />
      </div>
      <button
        type="button"
        onClick={() => setPaletteOpen(true)}
        className="flex h-6 w-[380px] min-w-0 max-w-[32vw] items-center gap-2 rounded-ctl max-[720px]:hidden border border-line bg-field px-2 text-12 text-faint hover:text-muted focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg"
      >
        <Search aria-hidden="true" className="size-3.5 text-icon" />
        <span className="min-w-0 flex-1 truncate text-left">{paletteHint(conn.status === "connected")}</span>
        <Kbd shortcut="mod+k" />
      </button>
      <div data-tauri-drag-region className="flex h-full items-center justify-end">
        {/* The field above gives way first; below 720 px this stands in for it (Ctrl or Cmd+K works either way). */}
        <Hint label="Go to entity or command" shortcut="mod+k">
          <IconButton label="Open the command palette" className="min-[721px]:hidden" onClick={() => setPaletteOpen(true)}>
            <Search />
          </IconButton>
        </Hint>
        <Hint label="Refresh" shortcut="f5">
          <IconButton label="Refresh" onClick={refresh} disabled={conn.status !== "connected"}>
            <RefreshCw />
          </IconButton>
        </Hint>
      </div>
    </header>
  );
}
