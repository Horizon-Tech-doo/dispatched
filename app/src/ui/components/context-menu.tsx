import { ContextMenu as M } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";
import { menuContent, menuItem } from "./dropdown-menu";

export const ContextMenu = M.Root;
export const ContextMenuTrigger = M.Trigger;

export function ContextMenuContent({ className, ...props }: ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content className={cn(menuContent, className)} {...props} />
    </M.Portal>
  );
}
export function ContextMenuItem({ className, ...props }: ComponentProps<typeof M.Item>) {
  return <M.Item className={cn(menuItem, className)} {...props} />;
}
export function ContextMenuSeparator({ className, ...props }: ComponentProps<typeof M.Separator>) {
  return <M.Separator className={cn("my-1 h-px bg-line", className)} {...props} />;
}
