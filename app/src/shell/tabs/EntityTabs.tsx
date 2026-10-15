import { X } from "lucide-react";
import { IconButton, Tabs, TabsList, TabsTrigger } from "@/ui";
import { useShell } from "../context";
import { splitSubscription } from "./subtabs";

export function EntityTabs() {
  const { shell, dispatch } = useShell();
  return (
    <Tabs value={shell.activeTab ?? ""} onValueChange={(id) => dispatch({ type: "activate", id })}>
      <TabsList variant="strip" aria-label="Open entities" className="border-b border-line">
        {shell.tabs.map((t) => {
          const sub = t.kind === "subscription" ? splitSubscription(t.id) : null;
          const name = sub ? sub.name : t.id;
          return (
            // The marker and the active background belong to the whole tab, close button included.
            <div
              key={t.id}
              className="group/tab flex items-stretch border-r border-t-2 border-line border-t-transparent has-[[data-state=active]]:border-t-fg has-[[data-state=active]]:bg-content"
              onAuxClick={(e) => e.button === 1 && dispatch({ type: "close", id: t.id })}
            >
              <TabsTrigger value={t.id} className="border-r-0 border-t-0 pr-1 data-[state=active]:border-t-0 data-[state=active]:bg-transparent">
                <span>{name}</span>
                <span className="text-faint">{sub ? sub.topic : t.kind}</span>
              </TabsTrigger>
              <IconButton
                label={sub ? `Close ${name} on ${sub.topic}` : `Close ${name}`}
                onClick={() => dispatch({ type: "close", id: t.id })}
                className="my-auto mr-1.5 size-4 opacity-0 group-hover/tab:opacity-100 focus-visible:opacity-100 group-has-[[data-state=active]]/tab:opacity-100"
              >
                <X />
              </IconButton>
            </div>
          );
        })}
      </TabsList>
    </Tabs>
  );
}
