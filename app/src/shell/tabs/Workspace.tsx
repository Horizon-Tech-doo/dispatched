import { EmptyState, Kbd } from "@/ui";
import { useShell } from "../context";
import { EntityTabs } from "./EntityTabs";
import { EntityView } from "./EntityView";

export function Workspace() {
  const { shell } = useShell();
  const active = shell.tabs.find((t) => t.id === shell.activeTab);
  return (
    <main className="flex h-full min-h-0 flex-col">
      {shell.tabs.length > 0 && <EntityTabs />}
      <div className="min-h-0 flex-1">
        {active ? (
          <EntityView key={active.id} tab={active} />
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
