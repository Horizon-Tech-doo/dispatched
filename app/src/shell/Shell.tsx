import { useEffect, useState, type ReactNode } from "react";
import { Panel, PanelGroup, ResizeHandle } from "@/ui";
import { useShell } from "./context";
import { Hotkeys } from "./Hotkeys";
import { CommandPalette } from "./palette/CommandPalette";
import { SheetHost } from "./sheets/SheetHost";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { Welcome } from "./Welcome";

/** The tree never gets narrower than this many pixels, so a row keeps room for its counts beside a name. */
const TREE_MIN_PX = 200;

function useWindowWidth(): number {
  const [width, setWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const on = () => setWidth(window.innerWidth);
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return width;
}

/** The window: top bar, then tree | (workspace / drawer), then status bar. */
export function Shell({ tree, workspace, drawer }: { tree?: ReactNode; workspace?: ReactNode; drawer?: ReactNode }) {
  const { shell, shellLoaded, live, dispatch } = useShell();
  // Reconnecting the same profile keeps the workbench up; Welcome is for no connection or a failed one.
  const ready = live && shellLoaded;
  // The tree's share is a percentage; in a narrow window it is raised so the pane stays at least TREE_MIN_PX wide.
  const width = useWindowWidth();
  const treeMin = Math.min(45, Math.max(12, Math.ceil((TREE_MIN_PX / Math.max(width, 1)) * 100)));
  const drawerShown = shell.layout.drawerOpen && drawer != null;
  return (
    <div className="flex h-full flex-col bg-content text-fg">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        {ready ? (
          <PanelGroup direction="horizontal" onLayout={([treeSize]) => dispatch({ type: "layout", patch: { treeSize } })}>
            <Panel id="tree" order={1} defaultSize={Math.max(shell.layout.treeSize, treeMin)} minSize={treeMin} maxSize={45} className="bg-side">
              {tree}
            </Panel>
            <ResizeHandle />
            <Panel id="main" order={2} defaultSize={100 - Math.max(shell.layout.treeSize, treeMin)} minSize={40}>
              <PanelGroup direction="vertical" onLayout={(sizes) => sizes.length === 2 && dispatch({ type: "layout", patch: { drawerSize: sizes[1] } })}>
                <Panel id="workspace" order={1} defaultSize={drawerShown ? 100 - shell.layout.drawerSize : 100} minSize={30}>
                  {workspace}
                </Panel>
                {drawerShown && (
                  <>
                    <ResizeHandle />
                    <Panel id="drawer" order={2} defaultSize={shell.layout.drawerSize} minSize={8} maxSize={70} className="bg-side">
                      {drawer}
                    </Panel>
                  </>
                )}
              </PanelGroup>
            </Panel>
          </PanelGroup>
        ) : live ? (
          <div className="h-full w-full bg-content" /> // shell state for this connection is still loading
        ) : (
          <Welcome />
        )}
      </div>
      <StatusBar />
      <SheetHost />
      <Hotkeys />
      <CommandPalette />
    </div>
  );
}
