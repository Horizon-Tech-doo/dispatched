import { DropdownMenu as M } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export const menuContent =
  "z-50 min-w-[200px] overflow-hidden rounded-ctl border border-line bg-side py-1 text-12 text-fg shadow-float";
export const menuItem =
  "relative flex h-row cursor-default items-center gap-2 px-3 outline-none data-[highlighted]:bg-selection data-[disabled]:opacity-45 [&_svg]:size-3.5 [&_svg]:text-icon";

export const DropdownMenu = M.Root;
export const DropdownMenuTrigger = M.Trigger;

export function DropdownMenuContent({ className, sideOffset = 2, align = "start", ...props }: ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content sideOffset={sideOffset} align={align} className={cn(menuContent, className)} {...props} />
    </M.Portal>
  );
}
export function DropdownMenuItem({ className, ...props }: ComponentProps<typeof M.Item>) {
  return <M.Item className={cn(menuItem, className)} {...props} />;
}
export function DropdownMenuSeparator({ className, ...props }: ComponentProps<typeof M.Separator>) {
  return <M.Separator className={cn("my-1 h-px bg-line", className)} {...props} />;
}
export function DropdownMenuLabel({ className, ...props }: ComponentProps<typeof M.Label>) {
  return <M.Label className={cn("px-3 py-1 text-11 text-muted", className)} {...props} />;
}
export function MenuShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span className={cn("ml-auto pl-4 font-mono text-11 text-muted", className)} {...props} />;
}
