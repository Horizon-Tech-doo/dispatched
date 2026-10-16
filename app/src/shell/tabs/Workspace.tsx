import { useEffect, useRef } from "react";
import { EmptyState, Kbd } from "@/ui";
import { useShell } from "../context";
import { DashboardView } from "../dashboard/DashboardView";
import { isEntityTab } from "../state";
import { EntityTabs } from "./EntityTabs";
import { EntityView } from "./EntityView";

/** Where focus goes when closing a tab took the focused element with it. */
function useFocusAfterClose(profileId: string | null, tabCount: number) {
  const prev = useRef({ profileId, tabCount });
  useEffect(() => {
    const was = prev.current;
    prev.current = { profileId, tabCount };
    if (was.profileId !== profileId || tabCount >= was.tabCount) return;
    const held = document.activeElement;
    if (held && held !== document.body) return; // focus is somewhere real: leave it
    const next =
      document.querySelector<HTMLElement>('[role="tablist"][aria-label="Open entities"] [role="tab"][data-state="active"]') ??
      document.querySelector<HTMLElement>('[role="tree"][aria-label="Namespace"]');
    next?.focus();
  }, [profileId, tabCount]);
}

export function Workspace() {
  const { shell, shellProfileId } = useShell();
  useFocusAfterClose(shellProfileId, shell.tabs.length);
  const active = shell.tabs.find((t) => t.id === shell.activeTab);
  return (
    <main className="flex h-full min-h-0 flex-col">
      {shell.tabs.length > 0 && <EntityTabs />}
      <div className="min-h-0 flex-1">
        {active ? (
          isEntityTab(active) ? <EntityView key={active.id} tab={active} /> : <DashboardView key={active.id} />
        ) : (
          <EmptyState
            title="Open an entity"
            body={<>Double-click a queue, topic or subscription in the tree, press Enter on it, or press <Kbd shortcut="mod+k" /> and type its name.</>}
          />
        )}
      </div>
    </main>
  );
}
