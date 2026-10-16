import { useEffect, useRef, useState, type ReactNode } from "react";
import { Panel, PanelGroup, ResizeHandle, type ImperativePanelHandle } from "@/ui";
import { useShell } from "./context";
import { Hotkeys } from "./Hotkeys";
import { CommandPalette } from "./palette/CommandPalette";
import { SheetHost } from "./sheets/SheetHost";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { treeMinPercent, treeShownSize } from "./treeLayout";
import { Welcome } from "./Welcome";

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
  const treeMin = treeMinPercent(width);
  // The saved size is the person's own choice. A narrow window only raises what is shown (`treeShownSize`) and never what
  // is saved: only a layout the person causes at the handle (drag or arrow keys) is written back, not one the window's
  // size or the raised minimum causes.
  const saved = shell.layout.treeSize;
  const savedRef = useRef(saved);
  savedRef.current = saved;
  const resizing = useRef(false);
  const treePanel = useRef<ImperativePanelHandle>(null);
  // The panel keeps its size when its minimum falls again (the window widened): put it back to the saved size, once
  // the group has taken the new minimum (asked sooner, the old minimum would clamp it).
  useEffect(() => {
    const t = setTimeout(() => {
      try {
        treePanel.current?.resize(treeShownSize(savedRef.current, treeMin));
      } catch {
        // Not laid out yet (the group has no size): its default size, set from the same value, is what shows.
      }
    }, 0);
    return () => clearTimeout(t);
  }, [treeMin]);
  const onTreeLayout = (treeSize: number) => {
    if (resizing.current) dispatch({ type: "layout", patch: { treeSize } });
  };
  const drawerShown = shell.layout.drawerOpen && drawer != null;
  return (
    <div className="flex h-full flex-col bg-content text-fg">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        {ready ? (
          <PanelGroup direction="horizontal" onLayout={([treeSize]) => onTreeLayout(treeSize)}>
            <Panel ref={treePanel} id="tree" order={1} defaultSize={treeShownSize(saved, treeMin)} minSize={treeMin} maxSize={45} className="bg-side">
              {tree}
            </Panel>
            <ResizeHandle
              onDragging={(dragging) => (resizing.current = dragging)}
              onKeyDown={() => {
                resizing.current = true;
                setTimeout(() => (resizing.current = false), 100);
              }}
            />
            <Panel id="main" order={2} defaultSize={100 - treeShownSize(saved, treeMin)} minSize={40}>
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
