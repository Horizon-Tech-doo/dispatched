import type { ComponentProps } from "react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { cn } from "../lib/cn";

export { Panel, PanelGroup };

/** 1px line with a 5px invisible hit area; highlights on hover and drag. */
export function ResizeHandle({ className, ...props }: ComponentProps<typeof PanelResizeHandle>) {
  return (
    <PanelResizeHandle
      className={cn(
        "relative bg-line outline-none data-[panel-group-direction=horizontal]:w-px data-[panel-group-direction=vertical]:h-px",
        "after:absolute after:inset-y-0 after:-left-[2px] after:w-[5px] data-[panel-group-direction=vertical]:after:inset-x-0 data-[panel-group-direction=vertical]:after:-top-[2px] data-[panel-group-direction=vertical]:after:h-[5px] data-[panel-group-direction=vertical]:after:w-auto",
        "hover:bg-muted data-[resize-handle-state=drag]:bg-fg",
        className,
      )}
      {...props}
    />
  );
}
