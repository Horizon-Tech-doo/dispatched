import { X } from "lucide-react";
import { EntityGlyph, IconButton, Tabs, TabsList, TabsTrigger } from "@/ui";
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
            <div key={t.id} className="group/tab flex items-stretch border-r border-line" onAuxClick={(e) => e.button === 1 && dispatch({ type: "close", id: t.id })}>
              <TabsTrigger value={t.id} className="border-r-0 pr-1.5">
                <EntityGlyph kind={t.kind} className="text-icon" />
                <span>{name}</span>
                {sub && <span className="text-faint">{sub.topic}</span>}
              </TabsTrigger>
              <IconButton
                label={`Close ${name}`}
                onClick={() => dispatch({ type: "close", id: t.id })}
                className="my-auto mr-1 size-5 opacity-0 group-hover/tab:opacity-100 focus-visible:opacity-100 group-has-[[data-state=active]]/tab:opacity-100"
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
