import type { ReactNode } from "react";
import { Panel, PanelGroup, ResizeHandle } from "@/ui";
import { useShell } from "./context";
import { SheetHost } from "./sheets/SheetHost";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { Welcome } from "./Welcome";

/** The window: top bar, then tree | (workspace / drawer), then status bar. */
export function Shell({ tree, workspace, drawer }: { tree?: ReactNode; workspace?: ReactNode; drawer?: ReactNode }) {
  const { conn, shell, shellLoaded, dispatch } = useShell();
  const ready = conn.status === "connected" && shellLoaded;
  const drawerShown = shell.layout.drawerOpen && drawer != null;
  return (
    <div className="flex h-full flex-col bg-content text-fg">
      <TopBar />
      <div className="flex min-h-0 flex-1">
        {ready ? (
          <PanelGroup direction="horizontal" onLayout={([treeSize]) => dispatch({ type: "layout", patch: { treeSize } })}>
            <Panel id="tree" order={1} defaultSize={shell.layout.treeSize} minSize={12} maxSize={45} className="bg-side">
              {tree}
            </Panel>
            <ResizeHandle />
            <Panel id="main" order={2} defaultSize={100 - shell.layout.treeSize} minSize={40}>
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
        ) : (
          <Welcome />
        )}
      </div>
      <StatusBar />
      <SheetHost />
    </div>
  );
}
